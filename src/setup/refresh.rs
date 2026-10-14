//! The guest refresh of `rayx setup --update`: for every current guest of this host `rayx` keeps
//! (WSL distributions on Windows, Lima VMs on macOS) it puts the Linux `rayx` of the running
//! version in the guest, upgrades the guest's packages and runs `rayx setup --update` inside.
//! This is the only place that runs `apt-get upgrade`, and only inside a guest `rayx` manages.

use std::io::Write;

use crate::host::{CommandSpec, HostFacts, Os, Outcome, Runner};

use super::Machine;
use super::guest::{self, Guest, GuestKind, LinuxBinary, pinned_ubuntu};
use super::lima::sizing::Prompter;
use super::lima::{INSTANCE, Tool, VmFlags, lima_path_dir, list_instances, list_instances_strict};
use super::migrate::{LimaMigration, LimaOld, Migration, WslMigration, migrate_lima, migrate_wsl};
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
    /// Older than the pinned Ubuntu and not replaced (declined, unanswerable or unsupported).
    Outdated(String),
    /// Older than the pinned Ubuntu and replaced side by side by the named new guest.
    Migrated(String),
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
                GuestOutcome::Refreshed | GuestOutcome::NotInspected | GuestOutcome::Migrated(_)
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
            GuestOutcome::Refreshed | GuestOutcome::NotInspected | GuestOutcome::Migrated(_) => {}
            GuestOutcome::NotUpdated(why)
            | GuestOutcome::Outdated(why)
            | GuestOutcome::Failed(why) => {
                eprintln!("rayx: {} guest {}: {why}", record.kind, record.name);
            }
        }
        report.guests.push((record.name.clone(), outcome));
    }
    // A migration that was interrupted leaves a renamed, retired VM and no current one.
    if kind == records::LIMA
        && !recorded
            .iter()
            .any(|g| g.kind == kind && g.name == INSTANCE && !g.retired)
        && let Some(old) = recorded
            .iter()
            .find(|g| g.kind == kind && g.retired && g.migrating)
    {
        let why = format!(
            "a migration was interrupted: {} is renamed and retired, and {INSTANCE} is not set up \
             yet; run `rayx setup --lima` to finish it",
            old.name
        );
        eprintln!("rayx: lima guest {INSTANCE}: {why}");
        report
            .guests
            .push((INSTANCE.to_string(), GuestOutcome::Failed(why)));
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

fn state(guest: &Guest, runner: &mut Runner, machine: &dyn Machine) -> Result<State, String> {
    Ok(match guest.kind {
        GuestKind::Lima => {
            let instances = list_instances_strict(&guest.path_dir, runner)?;
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
                return Ok(State::Missing);
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
    })
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
    mut prompter: Option<&mut dyn Prompter>,
    record: &GuestRecord,
    guest: &Guest,
    out: &mut dyn Write,
) -> GuestOutcome {
    let label = guest.kind.label();
    let current = match state(guest, runner, machine) {
        Ok(current) => current,
        Err(why) => return GuestOutcome::Failed(why),
    };
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
        && let Some(prompter) = prompter.as_mut()
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
    let live = guest::os_version(guest, runner);
    if let Some(version) = live.as_deref().filter(|v| newer_than_pin(v)) {
        let _ = writeln!(
            out,
            "{} runs Ubuntu {version}, newer than the pinned {}: rayx leaves its release alone",
            guest.name,
            pinned_ubuntu()
        );
    }
    let result = match live.filter(|v| older_than_pin(v)) {
        Some(version) => migrate_outdated(
            options,
            host,
            runner,
            machine,
            &mut prompter,
            record,
            guest,
            &version,
            out,
        ),
        None => refresh_inside(options, host, runner, guest, out),
    };
    if started {
        let _ = writeln!(
            out,
            "==> lima: stopping the VM {} again (it was stopped before the refresh)",
            guest.name
        );
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

pub use super::migrate::{newer_than_pin, older_than_pin};

/// An outdated guest is replaced side by side by a new one from the pinned Ubuntu.
#[allow(clippy::too_many_arguments)]
fn migrate_outdated(
    options: &RefreshOptions,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    prompter: &mut Option<&mut dyn Prompter>,
    record: &GuestRecord,
    guest: &Guest,
    version: &str,
    out: &mut dyn Write,
) -> GuestOutcome {
    let result = match guest.kind {
        GuestKind::Lima => {
            if guest.name != INSTANCE {
                return GuestOutcome::Outdated(format!(
                    "it runs Ubuntu {version}, older than {}; only the VM named {INSTANCE} is \
                     replaced by rayx",
                    pinned_ubuntu()
                ));
            }
            let tool = Tool {
                path_dir: guest.path_dir.clone(),
            };
            let Some(home) = machine.home() else {
                return GuestOutcome::Failed("the home directory is not known".to_string());
            };
            let instance = match list_instances(&tool, runner) {
                Ok(instances) => instances.into_iter().find(|i| i.name == INSTANCE),
                Err(error) => return GuestOutcome::Failed(error.to_string()),
            };
            let Some(instance) = instance else {
                return GuestOutcome::Failed("the VM is not listed by limactl".to_string());
            };
            let old = LimaOld {
                current_name: INSTANCE.to_string(),
                version: version.to_string(),
                entry: Some(record.clone()),
                instance,
            };
            let migration = LimaMigration {
                options,
                host,
                machine,
                home: &home,
                path_dir: guest.path_dir.clone(),
                vm_flags: VmFlags::default(),
            };
            migrate_lima(&migration, runner, prompter, out, &old).map(|(done, _)| done)
        }
        GuestKind::Wsl => {
            let migration = WslMigration {
                options,
                host,
                machine,
            };
            migrate_wsl(&migration, runner, prompter, out, record, version)
        }
    };
    match result {
        Ok(Migration::Done { new, .. }) => GuestOutcome::Migrated(new),
        Ok(Migration::Planned) => GuestOutcome::NotInspected,
        Ok(Migration::Declined(why)) => GuestOutcome::Outdated(why),
        Err(error) => GuestOutcome::Failed(error),
    }
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
