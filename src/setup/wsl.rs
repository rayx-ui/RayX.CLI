//! `rayx setup --wsl [--clone [<dir>]]`: from Windows, make the pinned Ubuntu on WSL 2 ready, put the
//! Linux `rayx` of the same version in it, optionally clone the current checkout into it, and run
//! `rayx setup` with the same flags inside it, in this console, so `sudo` prompts reach the user.
//!
//! Nothing here reads a manifest or clones another repository: the project's dependencies are
//! resolved by Cargo inside WSL from crates.io or git.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cli::SetupArgs;
use crate::host::{CommandSpec, HostFacts, Os, Outcome, Runner};

use super::Machine;
use super::guest::{self, CloneSource, Guest, LinuxBinary};
use super::migrate::{self, Migration, WslMigration};
use super::records::{self, CloneRecord};
use super::refresh::RefreshOptions;

pub use super::guest::{
    Checkout, LINUX_BINARY, clone_script, detect_checkout, linux_dir, release_asset, setup_flags,
};

/// The distribution `--wsl` sets up: `Ubuntu-<pin>`.
pub fn distribution() -> String {
    guest::wsl_distribution(guest::pinned_ubuntu())
}

/// What `rayx setup --wsl` was asked to do.
#[derive(Clone, Debug)]
pub struct WslRequest {
    /// The set flags (`--web`, `--test`, ...) to run `rayx setup` with inside the distribution.
    pub flags: Vec<String>,
    /// `--clone`: the Linux directory, `Some("")` for the default `~/<checkout name>`.
    pub clone: Option<String>,
    /// `RAYX_WSL_BINARY`: a Linux `rayx` built on this machine, used instead of the release installer.
    pub binary_override: Option<PathBuf>,
    /// The version of this `rayx`, which the Linux binary must match.
    pub version: String,
    /// `--gpu` or `--all`: the GPU driver belongs to Windows, so Windows installs it first.
    pub windows_gpu: Option<WindowsGpu>,
}

/// The Windows-side GPU setup that runs before the distribution's own.
#[derive(Clone, Debug)]
pub struct WindowsGpu {
    /// This `rayx`, which runs `setup --gpu` on Windows.
    pub executable: PathBuf,
    /// `--yes` and `--check`, passed through.
    pub flags: Vec<String>,
}

impl WslRequest {
    /// The request of a command line; the environment names the optional binary.
    pub fn from_args(args: &SetupArgs, binary_override: Option<PathBuf>) -> Self {
        Self {
            flags: setup_flags(args),
            clone: args.clone.clone(),
            binary_override,
            version: env!("CARGO_PKG_VERSION").to_string(),
            windows_gpu: (args.gpu || args.all).then(|| WindowsGpu {
                executable: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("rayx")),
                flags: [(args.yes, "--yes"), (args.check, "--check")]
                    .into_iter()
                    .filter(|(on, _)| *on)
                    .map(|(_, flag)| flag.to_string())
                    .collect(),
            }),
        }
    }
}

/// The path of a Windows directory as WSL mounts it: `C:\Users\dev\app` is `/mnt/c/Users/dev/app`.
pub fn to_wsl_path(path: &Path) -> Result<String, String> {
    let text = path.to_string_lossy();
    let plain = text
        .strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .replace('\\', "/");
    let mut chars = plain.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
            let rest = plain[2..].trim_start_matches('/');
            let mut mounted = format!("/mnt/{}", drive.to_ascii_lowercase());
            if !rest.is_empty() {
                mounted.push('/');
                mounted.push_str(rest);
            }
            Ok(mounted)
        }
        _ => Err(format!(
            "{} is not on a Windows drive, so WSL cannot reach it under /mnt",
            path.display()
        )),
    }
}

/// What stopped `--wsl`.
#[derive(Debug, PartialEq, Eq)]
pub enum WslError {
    NotWindows,
    /// WSL 2 is missing; the message says how to get it.
    Wsl(String),
    /// The distribution has no default user yet.
    NeedsUser(String),
    Failed(String),
}

impl std::fmt::Display for WslError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WslError::NotWindows => {
                f.write_str("`rayx setup --wsl` sets up WSL from Windows; run `rayx setup` here")
            }
            WslError::Wsl(message) | WslError::NeedsUser(message) | WslError::Failed(message) => {
                f.write_str(message)
            }
        }
    }
}

/// The text of a `wsl.exe` output: it prints UTF-16, which arrives with NUL bytes between letters.
fn wsl_text(outcome: &Outcome) -> String {
    outcome.stdout.replace('\0', "")
}

fn wsl_command(args: &[&str]) -> CommandSpec {
    CommandSpec::new("wsl").args(args.iter().copied())
}

/// `RAYX_WSL_BINARY`: a Linux `rayx` built on this machine, as a Windows path.
pub fn binary_override_from_env() -> Option<PathBuf> {
    std::env::var_os("RAYX_WSL_BINARY")
        .map(PathBuf::from)
        .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
        .map(|path| PathBuf::from(path.to_string_lossy().trim_start_matches(r"\?\")))
}

/// Runs `rayx setup --wsl` against the real machine and returns the process exit code.
pub fn run(args: &SetupArgs) -> u8 {
    let host = crate::host::facts();
    let request = WslRequest::from_args(args, binary_override_from_env());
    let mut runner = if args.check {
        Runner::print()
    } else {
        Runner::execute()
    };
    let machine = super::SystemMachine;
    let mut prompter = super::lima::sizing::TerminalPrompter {
        terminal: std::io::IsTerminal::is_terminal(&std::io::stdin())
            && std::io::IsTerminal::is_terminal(&std::io::stdout()),
    };
    if let Err(error) = migrate_older_guest(
        &request,
        args.check,
        args.yes,
        &host,
        &mut runner,
        &machine,
        Some(&mut prompter),
        &mut std::io::stdout(),
    ) {
        eprintln!("rayx: {error}");
        return 1;
    }
    let existed_before = machine
        .wsl_distributions()
        .iter()
        .any(|d| d.name == distribution());
    let checkout = detect_checkout(&mut runner);
    match run_with(
        &request,
        &host,
        &mut runner,
        &machine,
        checkout.as_ref(),
        &mut std::io::stdout(),
    ) {
        Ok(()) => {
            record_outcome(
                &request,
                args.check,
                &host,
                &machine,
                &mut runner,
                checkout.as_ref(),
            );
            0
        }
        Err(error) => {
            record_failed_first_run(args.check, &host, &machine, existed_before);
            eprintln!("rayx: {error}");
            1
        }
    }
}

/// Records the distribution on this host after a successful run; `--check` records nothing.
pub fn record_outcome(
    request: &WslRequest,
    check: bool,
    host: &HostFacts,
    machine: &dyn Machine,
    runner: &mut Runner,
    checkout: Option<&Checkout>,
) {
    if check {
        return;
    }
    let guest = Guest::wsl(distribution());
    guest::record_run(
        machine,
        host.os,
        &guest,
        &request.flags,
        clone_record(request, checkout),
        None,
        runner,
    );
}

/// A first run that registered the distribution and then failed (the first user, the Linux
/// `rayx`, the clone or the inner setup) still leaves a guest `rayx` manages: it is recorded so
/// doctor, `setup --update` and `rayx wsl remove` know it. `--check` records nothing.
pub fn record_failed_first_run(
    check: bool,
    host: &HostFacts,
    machine: &dyn Machine,
    existed_before: bool,
) {
    let name = distribution();
    if check || existed_before || !machine.wsl_distributions().iter().any(|d| d.name == name) {
        return;
    }
    let update = records::GuestUpdate {
        kind: records::WSL,
        name,
        ubuntu: super::guest::pinned_ubuntu().to_string(),
        flags: Vec::new(),
        clone: None,
        vm: None,
    };
    if let Err(error) = records::record_guest(machine, host.os, &update, false) {
        eprintln!("rayx: {error}");
    }
}

/// The clone `--clone` made, as the guest record keeps it.
fn clone_record(request: &WslRequest, checkout: Option<&Checkout>) -> Option<CloneRecord> {
    let (requested, checkout) = (request.clone.as_deref()?, checkout?);
    let source = CloneSource {
        name: guest::checkout_name(&checkout.root)?,
        path: to_wsl_path(&checkout.root).ok()?,
        branch: checkout.branch.clone(),
    };
    Some(CloneRecord {
        source: checkout.root.display().to_string(),
        branch: checkout.branch.clone(),
        target: guest::clone_target(requested, &source).ok()?,
    })
}

/// A recorded distribution older than the pin is replaced side by side by `Ubuntu-<pin>` before
/// the normal `--wsl` flow runs; `Err` carries what stopped it (declined, unanswerable, failed).
#[allow(clippy::too_many_arguments)]
pub fn migrate_older_guest(
    request: &WslRequest,
    check: bool,
    yes: bool,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    mut prompter: Option<&mut dyn super::lima::sizing::Prompter>,
    out: &mut dyn Write,
) -> Result<(), String> {
    let Ok(file) = records::read_guests(machine, host.os) else {
        return Ok(());
    };
    for guest in file
        .guests
        .iter()
        .filter(|g| g.kind == records::WSL && !g.retired && migrate::newer_than_pin(&g.ubuntu))
    {
        let _ = writeln!(
            out,
            "{} runs Ubuntu {}, newer than the pinned {}: rayx leaves its release alone",
            guest.name,
            guest.ubuntu,
            super::guest::pinned_ubuntu()
        );
    }
    let Some(old) = migrate::outdated_wsl(&file) else {
        return Ok(());
    };
    // The record can be stale: the guest's own version decides. `--check` never starts a
    // distribution to ask it.
    let inspectable = !check || guest::wsl_is_running(&old.name, runner);
    let live = inspectable
        .then(|| guest::os_version(&Guest::wsl(&old.name), runner))
        .flatten();
    if live.as_deref().is_some_and(|v| !migrate::older_than_pin(v)) {
        return Ok(());
    }
    let options = RefreshOptions {
        version: request.version.clone(),
        yes,
        check,
        lima_binary: None,
        wsl_binary: request
            .binary_override
            .as_deref()
            .and_then(|path| to_wsl_path(path).ok()),
    };
    let migration = WslMigration {
        options: &options,
        host,
        machine,
    };
    let version = live.unwrap_or_else(|| old.ubuntu.clone());
    match migrate::migrate_wsl(&migration, runner, &mut prompter, out, &old, &version)? {
        Migration::Declined(why) => Err(why),
        Migration::Done { .. } | Migration::Planned => Ok(()),
    }
}

/// The body of `rayx setup --wsl` over injectable collaborators.
pub fn run_with(
    request: &WslRequest,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    checkout: Option<&Checkout>,
    out: &mut dyn Write,
) -> Result<(), WslError> {
    if host.os != Os::Windows {
        return Err(WslError::NotWindows);
    }
    require_wsl2(runner)?;
    if let Some(gpu) = &request.windows_gpu {
        let _ = writeln!(
            out,
            "==> windows: rayx setup --gpu (the driver belongs to Windows)"
        );
        runner
            .run_checked(
                &CommandSpec::new(gpu.executable.display().to_string())
                    .args(["setup", "--gpu"])
                    .args(gpu.flags.iter().cloned())
                    .interactive(),
            )
            .map_err(|error| WslError::Failed(format!("rayx setup --gpu on Windows: {error}")))?;
    }
    ensure_distribution(runner, machine, out)?;
    let guest = Guest::wsl(distribution());
    guest::ensure_linux_binary(&guest, &linux_binary(request, host)?, runner, out)
        .map_err(WslError::Failed)?;
    let directory = match &request.clone {
        Some(requested) => Some(clone_checkout(&guest, requested, checkout, runner, out)?),
        None => None,
    };
    guest::run_setup_inside(&guest, &request.flags, directory.as_deref(), runner, out)
        .map_err(WslError::Failed)
}

fn require_wsl2(runner: &mut Runner) -> Result<(), WslError> {
    let missing = || {
        WslError::Wsl(
            "WSL 2 is required: run `wsl --install` from an administrator terminal, restart, then \
             run this again"
                .to_string(),
        )
    };
    let outcome = runner
        .query(&wsl_command(&["--version"]))
        .map_err(|_| missing())?;
    // `wsl --version` exists only in the WSL 2 package; its label is localized, so look for a
    // dotted version number instead of the English words.
    if outcome.is_success() && has_version_number(&wsl_text(&outcome)) {
        Ok(())
    } else {
        Err(missing())
    }
}

/// Whether `text` holds a dotted version such as `2.6.1` (the label around it is localized).
fn has_version_number(text: &str) -> bool {
    text.split(|c: char| !c.is_ascii_digit() && c != '.')
        .any(|word| {
            let parts: Vec<&str> = word.split('.').collect();
            parts.len() >= 2 && parts.iter().all(|p| !p.is_empty())
        })
}

pub(crate) fn ensure_distribution(
    runner: &mut Runner,
    machine: &dyn Machine,
    out: &mut dyn Write,
) -> Result<(), WslError> {
    let name = distribution();
    let find = |machine: &dyn Machine| {
        machine
            .wsl_distributions()
            .into_iter()
            .find(|d| d.name == name)
    };
    let mut found = find(machine);
    if found.is_none() {
        let _ = writeln!(out, "==> wsl: installing {name}");
        runner
            .run_checked(&wsl_command(&["--install", "-d", &name]).interactive())
            .map_err(|error| WslError::Failed(format!("installing {name}: {error}")))?;
        found = find(machine);
    }
    let has_user = found
        .as_ref()
        .is_some_and(|d| d.default_uid.is_some_and(|uid| uid != 0));
    if has_user {
        return Ok(());
    }
    Err(WslError::NeedsUser(format!(
        "{name} has no user yet: open Ubuntu once from the Start menu, create the user, \
         then run `rayx setup --wsl` again"
    )))
}

/// The binary the distribution gets: the override as `/mnt/...`, or the release installer.
fn linux_binary(request: &WslRequest, host: &HostFacts) -> Result<LinuxBinary, WslError> {
    let override_path = request
        .binary_override
        .as_deref()
        .map(to_wsl_path)
        .transpose()
        .map_err(WslError::Failed)?;
    Ok(LinuxBinary {
        version: request.version.clone(),
        arch: host.arch,
        override_path,
    })
}

/// Clones (or fetches) the Windows checkout and returns the Linux directory it lives in.
fn clone_checkout(
    guest: &Guest,
    requested: &str,
    checkout: Option<&Checkout>,
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<String, WslError> {
    let checkout = checkout.ok_or_else(|| {
        WslError::Failed("--clone needs a git checkout: run it from inside one".to_string())
    })?;
    let name = guest::checkout_name(&checkout.root)
        .ok_or_else(|| WslError::Failed("the checkout has no directory name".to_string()))?;
    let source = CloneSource {
        name,
        path: to_wsl_path(&checkout.root).map_err(WslError::Failed)?,
        branch: checkout.branch.clone(),
    };
    guest::clone_checkout(guest, requested, &source, runner, out).map_err(WslError::Failed)
}
