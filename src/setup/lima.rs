//! `rayx setup --lima [--clone [<dir>]]`: on macOS, make the Lima VM `rayx` (vz, the pinned
//! Ubuntu) ready, put the Linux `rayx` of the same version in it, optionally clone the current
//! checkout into it, and run `rayx setup` with the same flags inside it, in this console.
//!
//! Only the instance named `rayx` is ever created or started here; instances the owner made
//! themselves are never touched. Nothing here needs `sudo` on the host.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::cli::SetupArgs;
use crate::host::{CommandSpec, HostFacts, Os, Outcome, Runner};

use super::guest::{
    self, Checkout, CloneSource, Guest, LinuxBinary, lima_template, pinned_ubuntu, setup_flags,
};
use super::macos::brew_in;
use super::{Machine, SystemMachine, ask_on_terminal};

/// The VM `rayx` manages.
pub const INSTANCE: &str = "rayx";

/// The oldest macOS Lima's `vz` virtualization type runs on.
pub const MIN_MACOS: u32 = 13;

/// The oldest Lima `rayx` drives (the `create`/`start` split and `--mount-only` need Lima 2).
pub const MIN_LIMA_MAJOR: u32 = 2;

/// The default VM size, in CPUs and GiB.
pub const DEFAULT_CPUS: u32 = 6;
pub const DEFAULT_MEMORY_GIB: u32 = 12;
pub const DEFAULT_DISK_GIB: u32 = 100;

const GIB: u64 = 1024 * 1024 * 1024;

/// What a VM is created with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmSettings {
    pub cpus: u32,
    pub memory_gib: u32,
    pub disk_gib: u32,
    /// Whether the macOS home is mounted writable in the VM.
    pub home_writable: bool,
}

impl Default for VmSettings {
    fn default() -> Self {
        Self {
            cpus: DEFAULT_CPUS,
            memory_gib: DEFAULT_MEMORY_GIB,
            disk_gib: DEFAULT_DISK_GIB,
            home_writable: false,
        }
    }
}

/// The `--vm-*` flags: a value the owner gave, or none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VmFlags {
    pub cpus: Option<u32>,
    pub memory_gib: Option<u32>,
    pub disk_gib: Option<u32>,
    pub home_writable: bool,
}

impl VmFlags {
    pub fn from_args(args: &SetupArgs) -> Self {
        Self {
            cpus: args.vm_cpus,
            memory_gib: args.vm_memory,
            disk_gib: args.vm_disk,
            home_writable: args.vm_home_writable,
        }
    }
}

/// What `rayx setup --lima` was asked to do.
#[derive(Clone, Debug)]
pub struct LimaRequest {
    /// The set flags (`--web`, ...), `--check` and `--yes`, for `rayx setup` inside the VM.
    pub flags: Vec<String>,
    /// `--clone`: the Linux directory, `Some("")` for the default `~/<checkout name>`.
    pub clone: Option<String>,
    /// `RAYX_LIMA_BINARY`: a Linux `rayx` readable inside the VM, used instead of the installer.
    pub binary_override: Option<String>,
    /// The version of this `rayx`, which the Linux binary must match.
    pub version: String,
    /// The `--vm-*` flags.
    pub vm: VmFlags,
    pub check: bool,
    pub yes: bool,
}

impl LimaRequest {
    /// The request of a command line; the environment names the optional binary.
    pub fn from_args(args: &SetupArgs, binary_override: Option<String>) -> Self {
        Self {
            flags: setup_flags(args),
            clone: args.clone.clone(),
            binary_override,
            version: env!("CARGO_PKG_VERSION").to_string(),
            vm: VmFlags::from_args(args),
            check: args.check,
            yes: args.yes,
        }
    }
}

/// What stopped `--lima`.
#[derive(Debug, PartialEq, Eq)]
pub enum LimaError {
    NotMacOs,
    /// The Mac, Homebrew or Lima is not usable; the message says what to do.
    Prerequisite(String),
    /// An update was not made: declined, or nobody could be asked.
    Declined(String),
    Failed(String),
}

impl std::fmt::Display for LimaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimaError::NotMacOs => f.write_str("`rayx setup --lima` creates a Lima VM on macOS"),
            LimaError::Prerequisite(message)
            | LimaError::Declined(message)
            | LimaError::Failed(message) => f.write_str(message),
        }
    }
}

/// One Lima instance, from `limactl list --json`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Instance {
    pub name: String,
    /// `Running`, `Stopped`, `Broken`, ...
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub cpus: u32,
    /// Bytes.
    #[serde(default)]
    pub memory: u64,
    /// Bytes.
    #[serde(default)]
    pub disk: u64,
    /// The instance directory (holds `lima.yaml`).
    #[serde(default)]
    pub dir: String,
}

impl Instance {
    pub fn is_running(&self) -> bool {
        self.status.eq_ignore_ascii_case("running")
    }

    pub fn memory_gib(&self) -> u64 {
        self.memory / GIB
    }

    pub fn disk_gib(&self) -> u64 {
        self.disk / GIB
    }
}

/// The instances in `limactl list --json` output: one JSON object per line (a JSON array is
/// understood too). Lines that are not instances are ignored.
pub fn parse_instances(text: &str) -> Vec<Instance> {
    if let Ok(list) = serde_json::from_str::<Vec<Instance>>(text.trim()) {
        return list;
    }
    text.lines()
        .filter_map(|line| serde_json::from_str::<Instance>(line.trim()).ok())
        .collect()
}

/// The version in `limactl --version` (`limactl version 2.2.1`): the first dotted number.
pub fn parse_lima_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|word| word.trim_start_matches('v'))
        .find(|word| {
            word.split('.').count() >= 2
                && word
                    .split('.')
                    .all(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(|word| {
            word.split(['-', '+'])
                .next()
                .unwrap_or(word)
                .trim_end_matches('.')
                .to_string()
        })
}

/// The major number of a dotted version.
fn major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

/// The `limactl create` command of a new VM.
pub fn create_command(settings: &VmSettings, home: &Path) -> CommandSpec {
    let mount = format!(
        "--mount-only={}{}",
        home.display(),
        if settings.home_writable { ":w" } else { "" }
    );
    CommandSpec::new("limactl")
        .args(["create", "--tty=false"])
        .arg(format!("--name={INSTANCE}"))
        .arg("--vm-type=vz")
        .arg(format!("--cpus={}", settings.cpus))
        .arg(format!("--memory={}", settings.memory_gib))
        .arg(format!("--disk={}", settings.disk_gib))
        .arg(mount)
        .arg("--containerd=none")
        .arg(lima_template(pinned_ubuntu()))
        .interactive()
}

/// The `limactl start` command; it returns once the VM is ready.
pub fn start_command() -> CommandSpec {
    CommandSpec::new("limactl")
        .args(["start", "--tty=false", INSTANCE])
        .interactive()
}

/// The `limactl` commands of this module run with this directory first on `PATH` (Homebrew's
/// `bin`, while `limactl` is not on this process's `PATH` yet).
struct Tool {
    path_dir: Option<PathBuf>,
}

impl Tool {
    fn spec(&self, spec: CommandSpec) -> CommandSpec {
        match &self.path_dir {
            Some(dir) => spec.env("PATH", guest::path_with(dir)),
            None => spec,
        }
    }
}

/// Runs `rayx setup --lima` against the real machine and returns the process exit code.
pub fn run(args: &SetupArgs) -> u8 {
    let host = crate::host::facts();
    let binary = std::env::var("RAYX_LIMA_BINARY")
        .ok()
        .filter(|path| !path.trim().is_empty());
    let request = LimaRequest::from_args(args, binary);
    let mut runner = if args.check {
        Runner::print()
    } else {
        Runner::execute()
    };
    let machine = SystemMachine;
    let checkout = match detect_checkout(&request, &host, &mut runner) {
        Ok(checkout) => checkout,
        Err(error) => {
            eprintln!("rayx: {error}");
            return 1;
        }
    };
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mut confirm = move |question: &str| interactive && ask_on_terminal(question);
    match run_with(
        &request,
        &host,
        &mut runner,
        &machine,
        checkout.as_ref(),
        Some(&mut confirm),
        &mut std::io::stdout(),
    ) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("rayx: {error}");
            1
        }
    }
}

/// The checkout `--clone` copies. Off macOS this fails before anything runs, and without `--clone`
/// no command runs at all.
pub fn detect_checkout(
    request: &LimaRequest,
    host: &HostFacts,
    runner: &mut Runner,
) -> Result<Option<Checkout>, LimaError> {
    if host.os != Os::MacOs {
        return Err(LimaError::NotMacOs);
    }
    Ok(request
        .clone
        .as_ref()
        .and_then(|_| guest::detect_checkout(runner)))
}

/// The body of `rayx setup --lima` over injectable collaborators. `confirm` answers the question
/// before an outdated Lima is upgraded; `None` upgrades without asking, as `--yes` does.
pub fn run_with(
    request: &LimaRequest,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    checkout: Option<&Checkout>,
    confirm: Option<&mut dyn FnMut(&str) -> bool>,
    out: &mut dyn Write,
) -> Result<(), LimaError> {
    if host.os != Os::MacOs {
        return Err(LimaError::NotMacOs);
    }
    require_macos(runner)?;
    let brew = brew_in(machine).ok_or_else(|| {
        LimaError::Prerequisite(
            "Homebrew is not installed: run `rayx setup` first to install Homebrew".to_string(),
        )
    })?;
    let home = machine.home().ok_or_else(|| {
        LimaError::Failed("the home directory of this user is not known".to_string())
    })?;
    // A clone that cannot succeed is refused before anything is installed or created.
    let clone = request
        .clone
        .as_deref()
        .map(|requested| plan_clone(requested, checkout, &home))
        .transpose()?;
    // Right after `brew install lima`, Homebrew's `bin` may not be on this process's PATH.
    let tool = Tool {
        path_dir: machine
            .which("limactl")
            .is_none()
            .then(|| brew.parent().map(Path::to_path_buf))
            .flatten(),
    };
    ensure_lima(request, &brew, &tool, runner, confirm, out)?;

    let instances = list_instances(&tool, runner)?;
    let existing = instances.iter().find(|i| i.name == INSTANCE);
    match existing {
        None => {
            let settings = settings_for_new_vm(&request.vm);
            let _ = writeln!(
                out,
                "==> lima: creating the VM {INSTANCE} (Ubuntu {}, {} CPUs, {} GiB memory, {} GiB disk, home {})",
                pinned_ubuntu(),
                settings.cpus,
                settings.memory_gib,
                settings.disk_gib,
                if settings.home_writable {
                    "writable"
                } else {
                    "read-only"
                },
            );
            run_limactl(
                runner,
                &tool.spec(create_command(&settings, &home)),
                "creating the VM",
            )?;
            start_vm(runner, &tool, out)?;
        }
        Some(instance) if !instance.is_running() => start_vm(runner, &tool, out)?,
        Some(_) => {}
    }

    let guest = Guest::lima(INSTANCE).with_path_dir(tool.path_dir.clone());
    let binary = LinuxBinary {
        version: request.version.clone(),
        arch: host.arch,
        override_path: request.binary_override.clone(),
    };
    guest::ensure_linux_binary(&guest, &binary, runner, out).map_err(LimaError::Failed)?;
    let directory = match &clone {
        Some((requested, source)) => Some(
            guest::clone_checkout(&guest, requested, source, runner, out)
                .map_err(LimaError::Failed)?,
        ),
        None => None,
    };
    guest::run_setup_inside(&guest, &request.flags, directory.as_deref(), runner, out)
        .map_err(LimaError::Failed)
}

/// What a new VM gets: the defaults, then the `--vm-*` flags.
fn settings_for_new_vm(flags: &VmFlags) -> VmSettings {
    let defaults = VmSettings::default();
    VmSettings {
        cpus: flags.cpus.unwrap_or(defaults.cpus),
        memory_gib: flags.memory_gib.unwrap_or(defaults.memory_gib),
        disk_gib: flags.disk_gib.unwrap_or(defaults.disk_gib),
        home_writable: flags.home_writable,
    }
}

fn require_macos(runner: &mut Runner) -> Result<(), LimaError> {
    let too_old = || {
        LimaError::Prerequisite(format!(
            "`rayx setup --lima` needs macOS {MIN_MACOS} or later: Lima's vz virtualization does \
             not run on older releases"
        ))
    };
    let outcome = runner
        .query(&CommandSpec::new("sw_vers").arg("-productVersion"))
        .ok()
        .filter(Outcome::is_success)
        .ok_or_else(too_old)?;
    match major(outcome.stdout.trim()) {
        Some(version) if version >= MIN_MACOS => Ok(()),
        _ => Err(too_old()),
    }
}

/// Installs Lima with Homebrew, or upgrades one older than 2.0 after asking.
fn ensure_lima(
    request: &LimaRequest,
    brew: &Path,
    tool: &Tool,
    runner: &mut Runner,
    confirm: Option<&mut dyn FnMut(&str) -> bool>,
    out: &mut dyn Write,
) -> Result<(), LimaError> {
    let found = runner
        .query(&tool.spec(CommandSpec::new("limactl").arg("--version")))
        .ok()
        .filter(Outcome::is_success)
        .and_then(|outcome| parse_lima_version(&outcome.stdout));
    let brew = brew.display().to_string();
    match found {
        Some(version) if major(&version).is_some_and(|m| m >= MIN_LIMA_MAJOR) => Ok(()),
        None => {
            let _ = writeln!(out, "==> lima: installing Lima with Homebrew");
            run_limactl(
                runner,
                &CommandSpec::new(brew)
                    .args(["install", "lima"])
                    .interactive(),
                "brew install lima",
            )
        }
        Some(version) => {
            let command = format!("{brew} upgrade lima");
            if !request.yes
                && let Some(confirm) = confirm
                && !confirm(&format!(
                    "Lima {version} is older than {MIN_LIMA_MAJOR}.0. Upgrade it now?"
                ))
            {
                return Err(LimaError::Declined(format!(
                    "Lima {version} is older than {MIN_LIMA_MAJOR}.0 and was not upgraded: upgrade \
                     it with `rayx setup --lima --yes` or by hand: {command}"
                )));
            }
            let _ = writeln!(out, "==> lima: upgrading Lima {version} with Homebrew");
            run_limactl(
                runner,
                &CommandSpec::new(brew)
                    .args(["upgrade", "lima"])
                    .interactive(),
                "brew upgrade lima",
            )
        }
    }
}

fn run_limactl(runner: &mut Runner, spec: &CommandSpec, what: &str) -> Result<(), LimaError> {
    runner
        .run_checked(spec)
        .map(|_| ())
        .map_err(|error| LimaError::Failed(format!("{what}: {error}")))
}

/// Every Lima instance. A machine where `limactl` cannot run yet (`--check` before Lima is
/// installed) has none.
fn list_instances(tool: &Tool, runner: &mut Runner) -> Result<Vec<Instance>, LimaError> {
    let listed = runner
        .query(&tool.spec(CommandSpec::new("limactl").args(["list", "--json"])))
        .ok()
        .filter(Outcome::is_success);
    match listed {
        Some(outcome) => Ok(parse_instances(&outcome.stdout)),
        None if runner.is_dry_run() => Ok(Vec::new()),
        None => Err(LimaError::Failed(
            "`limactl list --json` failed: Lima is not usable".to_string(),
        )),
    }
}

fn start_vm(runner: &mut Runner, tool: &Tool, out: &mut dyn Write) -> Result<(), LimaError> {
    let _ = writeln!(out, "==> lima: starting the VM {INSTANCE}");
    run_limactl(runner, &tool.spec(start_command()), "starting the VM")
}

/// The clone `--clone` asks for: the requested target and the checkout as the VM sees it. The VM
/// mounts the macOS home at the same path, so the clone source is the checkout's own path.
fn plan_clone(
    requested: &str,
    checkout: Option<&Checkout>,
    home: &Path,
) -> Result<(String, CloneSource), LimaError> {
    let checkout = checkout.ok_or_else(|| {
        LimaError::Failed("--clone needs a git checkout: run it from inside one".to_string())
    })?;
    if !checkout.root.starts_with(home) {
        return Err(LimaError::Failed(format!(
            "--clone needs a checkout under {}, which the VM mounts",
            home.display()
        )));
    }
    let name = guest::checkout_name(&checkout.root)
        .ok_or_else(|| LimaError::Failed("the checkout has no directory name".to_string()))?;
    if !requested.is_empty() {
        guest::linux_dir(requested).map_err(LimaError::Failed)?;
    }
    Ok((
        requested.to_string(),
        CloneSource {
            name,
            path: checkout.root.display().to_string(),
            branch: checkout.branch.clone(),
        },
    ))
}
