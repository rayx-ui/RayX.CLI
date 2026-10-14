//! The guest refresh of `rayx setup --update`: for every current guest of this host `rayx` keeps
//! (WSL distributions on Windows, Lima VMs on macOS) it puts the Linux `rayx` of the running
//! version in the guest, upgrades the guest's packages and runs `rayx setup --update` inside.
//! This is the only place that runs `apt-get upgrade`, and only inside a guest `rayx` manages.

use std::io::Write;

use crate::host::{CommandSpec, HostFacts, Os, Outcome, Runner};

use super::Machine;
use super::guest::{self, Guest, GuestKind, LinuxBinary, pinned_ubuntu};
use super::lima::sizing::Prompter;
use super::lima::{lima_path_dir, parse_instances};
use super::records::{self, GuestRecord};

/// What a refresh needs besides the host and the guests.
#[derive(Clone, Debug)]
pub struct RefreshOptions {
    /// The version of the running `rayx`, which every guest must have.
    pub version: String,
    /// `--yes`: no questions, `-y` for apt.
    pub yes: bool,
    /// `--check`: report, change nothing, start nothing.
    pub check: bool,
    /// `RAYX_LIMA_BINARY`: a Linux `rayx` readable inside a Lima VM.
    pub lima_binary: Option<String>,
    /// `RAYX_WSL_BINARY` as a `/mnt/...` path.
    pub wsl_binary: Option<String>,
}

/// What happened to one guest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuestOutcome {
    Refreshed,
    /// Declined, or nobody could be asked.
    NotUpdated(String),
    /// Older than the pinned Ubuntu: migration replaces it, a refresh does not touch it.
    Outdated(String),
    Failed(String),
    /// `--check` of a guest that is not running.
    NotInspected,
}

/// The guests a refresh looked at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub guests: Vec<(String, GuestOutcome)>,
    /// `guests.json` exists but could not be used, so no guest could be looked at.
    pub unreadable: bool,
}

impl RefreshReport {
    /// 1 when any guest failed, was declined or is outdated, or the record could not be read.
    pub fn code(&self) -> u8 {
        let unfinished = self.guests.iter().any(|(_, outcome)| {
            !matches!(
                outcome,
                GuestOutcome::Refreshed | GuestOutcome::NotInspected
            )
        });
        u8::from(self.unreadable || unfinished)
    }
}

/// The kind of guest this host keeps.
fn kind_for(os: Os) -> Option<&'static str> {
    match os {
        Os::MacOs => Some(records::LIMA),
        Os::Windows => Some(records::WSL),
        Os::Linux => None,
    }
}

/// Refreshes every current guest of this host. A failing guest is reported and the next one still
/// runs.
pub fn refresh_guests(
    options: &RefreshOptions,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    mut prompter: Option<&mut dyn Prompter>,
    out: &mut dyn Write,
) -> RefreshReport {
    let mut report = RefreshReport::default();
    let Some(kind) = kind_for(host.os).filter(|_| !host.wsl) else {
        return report;
    };
    let recorded = match records::read_guests(machine, host.os) {
        Ok(file) => file.guests,
        Err(error) => {
            eprintln!("rayx: {error}");
            report.unreadable = true;
            return report;
        }
    };
    let path_dir = lima_path_dir(machine);
    for record in recorded.iter().filter(|g| g.kind == kind && !g.retired) {
        let guest = match record.kind.as_str() {
            records::LIMA => Guest::lima(&record.name).with_path_dir(path_dir.clone()),
            _ => Guest::wsl(&record.name),
        };
        let outcome = refresh_one(
            options,
            host,
            runner,
            machine,
            prompter.as_deref_mut().map(|p| p as &mut dyn Prompter),
            record,
            &guest,
            out,
        );
        match &outcome {
            GuestOutcome::Refreshed | GuestOutcome::NotInspected => {}
            GuestOutcome::NotUpdated(why)
            | GuestOutcome::Outdated(why)
            | GuestOutcome::Failed(why) => {
                eprintln!("rayx: {} guest {}: {why}", record.kind, record.name);
            }
        }
        report.guests.push((record.name.clone(), outcome));
    }
    report
}

/// Whether the guest is up: a running Lima VM, or a WSL distribution that is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Running,
    Stopped,
    Missing,
}

fn state(guest: &Guest, runner: &mut Runner, machine: &dyn Machine) -> State {
    match guest.kind {
        GuestKind::Lima => {
            let spec = CommandSpec::new("limactl").args(["list", "--json"]);
            let spec = match &guest.path_dir {
                Some(dir) => spec.env("PATH", guest::path_with(dir)),
                None => spec,
            };
            let instances = runner
                .query(&spec)
                .ok()
                .filter(Outcome::is_success)
                .map(|outcome| parse_instances(&outcome.stdout))
                .unwrap_or_default();
            match instances.iter().find(|i| i.name == guest.name) {
                Some(i) if i.is_running() => State::Running,
                Some(_) => State::Stopped,
                None => State::Missing,
            }
        }
        GuestKind::Wsl => {
            if !machine
                .wsl_distributions()
                .iter()
                .any(|d| d.name == guest.name)
            {
                return State::Missing;
            }
            let running = runner
                .query(&CommandSpec::new("wsl").args(["--list", "--running", "--quiet"]))
                .ok()
                .filter(Outcome::is_success)
                .is_some_and(|outcome| {
                    outcome
                        .stdout
                        .replace('\0', "")
                        .lines()
                        .any(|line| line.trim() == guest.name)
                });
            if running {
                State::Running
            } else {
                State::Stopped
            }
        }
    }
}

fn limactl(guest: &Guest, args: &[&str]) -> CommandSpec {
    let spec = CommandSpec::new("limactl")
        .args(args.iter().copied())
        .interactive();
    match &guest.path_dir {
        Some(dir) => spec.env("PATH", guest::path_with(dir)),
        None => spec,
    }
}

#[allow(clippy::too_many_arguments)]
fn refresh_one(
    options: &RefreshOptions,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    prompter: Option<&mut dyn Prompter>,
    record: &GuestRecord,
    guest: &Guest,
    out: &mut dyn Write,
) -> GuestOutcome {
    let label = guest.kind.label();
    let current = state(guest, runner, machine);
    if current == State::Missing {
        return GuestOutcome::Failed(format!(
            "it is recorded but {label} does not have it: `rayx {label} remove {}` drops the \
             record",
            record.name
        ));
    }
    if options.check {
        return check_one(options, host, runner, record, guest, current, out);
    }
    if !options.yes
        && let Some(prompter) = prompter
        && (!prompter.interactive()
            || !prompter.confirm(&format!("Update {label} guest {}?", record.name)))
    {
        return GuestOutcome::NotUpdated(
            "not updated: answer yes, or run `rayx setup --update --yes`".to_string(),
        );
    }
    let _ = writeln!(out, "==> {label}: refreshing {}", record.name);
    // A VM that was stopped is stopped again afterwards, whatever happens in between.
    let started = guest.kind == GuestKind::Lima && current == State::Stopped;
    if started {
        let _ = writeln!(out, "==> lima: starting the VM {}", guest.name);
        if let Err(error) =
            runner.run_checked(&limactl(guest, &["start", "--tty=false", &guest.name]))
        {
            return GuestOutcome::Failed(format!("starting the VM: {error}"));
        }
    }
    let result = refresh_inside(options, host, runner, guest, out);
    if started {
        let _ = writeln!(out, "==> lima: stopping the VM {}", guest.name);
        let _ = runner.run_checked(&limactl(guest, &["stop", &guest.name]));
    }
    result
}

/// The refresh of a guest that is up.
fn refresh_inside(
    options: &RefreshOptions,
    host: &HostFacts,
    runner: &mut Runner,
    guest: &Guest,
    out: &mut dyn Write,
) -> GuestOutcome {
    if let Some(version) = guest::os_version(guest, runner)
        && older_than_pin(&version)
    {
        return GuestOutcome::Outdated(format!(
            "it runs Ubuntu {version}, older than {}: set it up again to replace it",
            pinned_ubuntu()
        ));
    }
    if let Err(error) =
        guest::ensure_linux_binary(guest, &binary(options, host, guest), runner, out)
    {
        return GuestOutcome::Failed(error);
    }
    let yes = if options.yes { " -y" } else { "" };
    let upgrade = format!("sudo apt-get update && sudo apt-get upgrade{yes}");
    let _ = writeln!(out, "==> {}: {upgrade}", guest.kind.label());
    if let Err(error) = runner.run_checked(&guest.script(&upgrade).interactive()) {
        return GuestOutcome::Failed(format!("upgrading the packages: {error}"));
    }
    match guest::run_setup_inside(guest, &inner_flags(options), None, runner, out) {
        Ok(()) => GuestOutcome::Refreshed,
        Err(error) => GuestOutcome::Failed(error),
    }
}

fn inner_flags(options: &RefreshOptions) -> Vec<String> {
    let mut flags = vec!["--update".to_string()];
    if options.yes {
        flags.push("--yes".to_string());
    }
    flags
}

fn binary(options: &RefreshOptions, host: &HostFacts, guest: &Guest) -> LinuxBinary {
    LinuxBinary {
        version: options.version.clone(),
        arch: host.arch,
        override_path: match guest.kind {
            GuestKind::Lima => options.lima_binary.clone(),
            GuestKind::Wsl => options.wsl_binary.clone(),
        },
    }
}

/// Whether an Ubuntu `VERSION_ID` is older than the pin (numeric on `major.minor`).
pub fn older_than_pin(version: &str) -> bool {
    fn key(version: &str) -> Vec<u32> {
        version
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    }
    key(version) < key(pinned_ubuntu())
}

/// `--check`: the guest's `rayx` and Ubuntu against this host's, the apt commands, and what
/// `rayx setup --update --check` says inside it. A guest that is not running is not inspected.
fn check_one(
    options: &RefreshOptions,
    host: &HostFacts,
    runner: &mut Runner,
    record: &GuestRecord,
    guest: &Guest,
    current: State,
    out: &mut dyn Write,
) -> GuestOutcome {
    let label = guest.kind.label();
    let _ = writeln!(out, "==> {label}: checking {}", record.name);
    if current != State::Running {
        let _ = writeln!(
            out,
            "{} is not running, so it is not inspected (--check starts nothing)",
            record.name
        );
        return GuestOutcome::NotInspected;
    }
    let inside = guest::linux_binary_version(guest, runner);
    let _ = writeln!(
        out,
        "rayx in the guest: {} (this rayx is {})",
        inside.as_deref().unwrap_or("missing"),
        options.version
    );
    let ubuntu = guest::os_version(guest, runner);
    let _ = writeln!(
        out,
        "Ubuntu: {} (pinned {})",
        ubuntu.as_deref().unwrap_or("unknown"),
        pinned_ubuntu()
    );
    if let Some(version) = &ubuntu
        && older_than_pin(version)
    {
        return GuestOutcome::Outdated(format!(
            "it runs Ubuntu {version}, older than {}: set it up again to replace it",
            pinned_ubuntu()
        ));
    }
    let _ = guest::ensure_linux_binary(guest, &binary(options, host, guest), runner, out);
    let yes = if options.yes { " -y" } else { "" };
    let upgrade = format!("sudo apt-get update && sudo apt-get upgrade{yes}");
    let _ = runner.run_checked(&guest.script(&upgrade).interactive());
    let mut flags = inner_flags(options);
    flags.push("--check".to_string());
    let script = format!("\"{}\" setup {}", guest::LINUX_BINARY, flags.join(" "));
    let _ = writeln!(out, "==> {label}: rayx setup {}", flags.join(" "));
    match runner.query(&guest.script(&script)) {
        Ok(outcome) => {
            let _ = write!(out, "{}", outcome.stdout.replace('\0', ""));
            GuestOutcome::Refreshed
        }
        Err(error) => GuestOutcome::Failed(format!("rayx setup --update --check inside: {error}")),
    }
}
