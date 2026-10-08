//! `rayx setup --wsl [--clone [<dir>]]`: from Windows, make `Ubuntu-24.04` on WSL 2 ready, put the
//! Linux `rayx` of the same version in it, optionally clone the current checkout into it, and run
//! `rayx setup` with the same flags inside it, in this console, so `sudo` prompts reach the user.
//!
//! Nothing here reads a manifest or clones another repository: the project's dependencies are
//! resolved by Cargo inside WSL from crates.io or git.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cli::SetupArgs;
use crate::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Runner};

use super::Machine;

/// The distribution `--wsl` sets up.
pub const DISTRIBUTION: &str = "Ubuntu-24.04";

/// Where the Linux `rayx` goes inside the distribution.
pub const LINUX_BINARY: &str = "$HOME/.local/bin/rayx";

/// The repository releases are published from.
pub const RELEASES: &str = "https://github.com/rayx-ui/RayX.CLI/releases/download";

/// The Windows checkout `--clone` copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkout {
    /// The checkout's top-level directory.
    pub root: PathBuf,
    /// The checked-out branch; `None` for a detached HEAD.
    pub branch: Option<String>,
}

/// What `rayx setup --wsl` was asked to do.
#[derive(Clone, Debug)]
pub struct WslRequest {
    /// The set flags (`--web`, `--test`, ...) to run `rayx setup` with inside the distribution.
    pub flags: Vec<String>,
    /// `--clone`: the Linux directory, `Some("")` for the default `~/<checkout name>`.
    pub clone: Option<String>,
    /// `RAYX_WSL_BINARY`: a Linux `rayx` built on this machine, used instead of the release asset.
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

/// The flags `rayx setup` runs with inside the distribution: the set flags, `--check` and `--yes`.
pub fn setup_flags(args: &SetupArgs) -> Vec<String> {
    [
        (args.web, "--web"),
        (args.test, "--test"),
        (args.android, "--android"),
        (args.ios, "--ios"),
        (args.gpu, "--gpu"),
        (args.all, "--all"),
        (args.check, "--check"),
        (args.yes, "--yes"),
    ]
    .into_iter()
    .filter(|(enabled, _)| *enabled)
    .map(|(_, flag)| flag.to_string())
    .collect()
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

/// A Linux directory for use inside double quotes in a script: a leading `~` becomes `$HOME`, and
/// characters that could end the quoting are refused.
pub fn linux_dir(dir: &str) -> Result<String, String> {
    let expanded = match dir {
        "~" => "$HOME".to_string(),
        _ => match dir.strip_prefix("~/") {
            Some(rest) => format!("$HOME/{rest}"),
            None => dir.to_string(),
        },
    };
    let body = expanded.strip_prefix("$HOME").unwrap_or(&expanded);
    if body
        .chars()
        .any(|c| matches!(c, '"' | '`' | '$' | '\\' | '\n' | '\''))
    {
        return Err(format!("{dir} is not a usable Linux directory name"));
    }
    // A relative path would land on the Windows directory `wsl.exe` starts in, and be recorded
    // relative to wherever a later command runs.
    if !expanded.starts_with('/') && !expanded.starts_with("$HOME") {
        return Err(format!(
            "{dir} is not an absolute Linux path: use /abs/dir or ~/dir"
        ));
    }
    Ok(expanded)
}

/// The release asset that holds the Linux `rayx` for a host: the static musl build.
pub fn release_asset(arch: Arch) -> Option<&'static str> {
    match arch {
        Arch::X64 => Some("x86_64-unknown-linux-musl"),
        Arch::Arm64 => Some("aarch64-unknown-linux-musl"),
        Arch::Other => None,
    }
}

/// The script that clones `source` (a `/mnt/...` path) into `target`, or fetches an existing clone,
/// and records `target` so `rayx wsl status` and `wsl compact --clean` find its build output.
pub fn clone_script(source: &str, branch: Option<&str>, target: &str) -> Result<String, String> {
    if source.contains('\'') || branch.is_some_and(|b| b.contains('\'')) {
        return Err("the checkout path or branch contains a quote".to_string());
    }
    let branch_flag = branch.map_or(String::new(), |b| format!(" -b '{b}'"));
    let clones = crate::wsl::CLONES_FILE;
    Ok(format!(
        "if [ -d \"{target}/.git\" ]; then git -C \"{target}\" fetch --all --prune; \
         else git -c safe.directory='*' clone{branch_flag} '{source}' \"{target}\"; fi && \
         mkdir -p \"$HOME/.config/rayx\" && {{ grep -qxF \"{target}\" \"{clones}\" 2>/dev/null || \
         echo \"{target}\" >> \"{clones}\"; }}"
    ))
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

/// A bash script run inside the distribution as the default user.
fn in_distribution(script: &str) -> CommandSpec {
    wsl_command(&["-d", DISTRIBUTION, "--exec", "bash", "-lc"]).arg(script)
}

/// Runs `rayx setup --wsl` against the real machine and returns the process exit code.
pub fn run(args: &SetupArgs) -> u8 {
    let host = crate::host::facts();
    let binary = std::env::var_os("RAYX_WSL_BINARY")
        .map(PathBuf::from)
        .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
        .map(|path| PathBuf::from(path.to_string_lossy().trim_start_matches(r"\?\")));
    let request = WslRequest::from_args(args, binary);
    let mut runner = if args.check {
        Runner::print()
    } else {
        Runner::execute()
    };
    let machine = super::SystemMachine;
    let checkout = detect_checkout(&mut runner);
    match run_with(
        &request,
        &host,
        &mut runner,
        &machine,
        checkout.as_ref(),
        &mut std::io::stdout(),
    ) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("rayx: {error}");
            1
        }
    }
}

/// The git checkout around the current directory, when there is one.
pub fn detect_checkout(runner: &mut Runner) -> Option<Checkout> {
    let top = runner
        .query(&CommandSpec::new("git").args(["rev-parse", "--show-toplevel"]))
        .ok()
        .filter(Outcome::is_success)?;
    let root = PathBuf::from(top.stdout.trim());
    let branch = runner
        .query(&CommandSpec::new("git").args(["rev-parse", "--abbrev-ref", "HEAD"]))
        .ok()
        .filter(Outcome::is_success)
        .map(|outcome| outcome.stdout.trim().to_string())
        .filter(|branch| !branch.is_empty() && branch != "HEAD");
    Some(Checkout { root, branch })
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
    ensure_linux_binary(request, host, runner, out)?;
    let directory = match &request.clone {
        Some(requested) => Some(clone_checkout(requested, checkout, runner, out)?),
        None => None,
    };
    run_setup_inside(request, directory.as_deref(), runner, out)
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

fn ensure_distribution(
    runner: &mut Runner,
    machine: &dyn Machine,
    out: &mut dyn Write,
) -> Result<(), WslError> {
    let find = |machine: &dyn Machine| {
        machine
            .wsl_distributions()
            .into_iter()
            .find(|d| d.name == DISTRIBUTION)
    };
    let mut distribution = find(machine);
    if distribution.is_none() {
        let _ = writeln!(out, "==> wsl: installing {DISTRIBUTION}");
        runner
            .run_checked(&wsl_command(&["--install", "-d", DISTRIBUTION]).interactive())
            .map_err(|error| WslError::Failed(format!("installing {DISTRIBUTION}: {error}")))?;
        distribution = find(machine);
    }
    let has_user = distribution
        .as_ref()
        .is_some_and(|d| d.default_uid.is_some_and(|uid| uid != 0));
    if has_user {
        return Ok(());
    }
    Err(WslError::NeedsUser(format!(
        "{DISTRIBUTION} has no user yet: open Ubuntu once from the Start menu, create the user, \
         then run `rayx setup --wsl` again"
    )))
}

fn ensure_linux_binary(
    request: &WslRequest,
    host: &HostFacts,
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<(), WslError> {
    if let Some(binary) = &request.binary_override {
        let source = to_wsl_path(binary).map_err(WslError::Failed)?;
        if source.contains('"') {
            return Err(WslError::Failed(format!(
                "{} is not a usable path",
                binary.display()
            )));
        }
        let _ = writeln!(
            out,
            "==> wsl: installing the Linux rayx from {}",
            binary.display()
        );
        let script = format!(
            "mkdir -p \"$HOME/.local/bin\" && cp \"{source}\" \"{LINUX_BINARY}\" && chmod 755 \"{LINUX_BINARY}\""
        );
        return run_inside(runner, &script, "copying the Linux rayx");
    }
    let version_script = format!("\"{LINUX_BINARY}\" --version");
    let installed = runner
        .query(&in_distribution(&version_script))
        .ok()
        .filter(Outcome::is_success)
        .map(|outcome| wsl_text(&outcome).trim().to_string());
    if installed.as_deref() == Some(format!("rayx {}", request.version).as_str()) {
        return Ok(());
    }
    let asset = release_asset(host.arch).ok_or_else(|| {
        WslError::Failed("this CPU architecture has no Linux rayx release".to_string())
    })?;
    let url = format!(
        "{RELEASES}/v{version}/rayx-cli-{asset}.tar.xz",
        version = request.version
    );
    let _ = writeln!(
        out,
        "==> wsl: installing the Linux rayx {}",
        request.version
    );
    let script = format!(
        "mkdir -p \"$HOME/.local/bin\" && curl --proto '=https' --tlsv1.2 -LsSf '{url}' | \
         tar -xJ --strip-components=1 -C \"$HOME/.local/bin\" 'rayx-cli-{asset}/rayx' && \
         chmod 755 \"{LINUX_BINARY}\""
    );
    run_inside(runner, &script, "downloading the Linux rayx")
}

fn run_inside(runner: &mut Runner, script: &str, what: &str) -> Result<(), WslError> {
    runner
        .run_checked(&in_distribution(script).interactive())
        .map(|_| ())
        .map_err(|error| WslError::Failed(format!("{what}: {error}")))
}

/// Clones (or fetches) the checkout and returns the Linux directory it lives in.
fn clone_checkout(
    requested: &str,
    checkout: Option<&Checkout>,
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<String, WslError> {
    let checkout = checkout.ok_or_else(|| {
        WslError::Failed("--clone needs a git checkout: run it from inside one".to_string())
    })?;
    // A Windows path, split on both separators so the name is the same on every host.
    let name = checkout
        .root
        .to_string_lossy()
        .split(['/', '\\'])
        .rfind(|part| !part.is_empty())
        .map(str::to_string)
        .ok_or_else(|| WslError::Failed("the checkout has no directory name".to_string()))?;
    let target = if requested.is_empty() {
        linux_dir(&format!("~/{name}"))
    } else {
        linux_dir(requested)
    }
    .map_err(WslError::Failed)?;
    let source = to_wsl_path(&checkout.root).map_err(WslError::Failed)?;
    let script =
        clone_script(&source, checkout.branch.as_deref(), &target).map_err(WslError::Failed)?;
    let _ = writeln!(
        out,
        "==> wsl: cloning {} into {target}",
        checkout.root.display()
    );
    run_inside(runner, &script, "cloning the checkout")?;
    Ok(target)
}

fn run_setup_inside(
    request: &WslRequest,
    directory: Option<&str>,
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<(), WslError> {
    let flags = request.flags.join(" ");
    let enter = directory.map_or(String::new(), |dir| format!("cd \"{dir}\" && "));
    let script = format!("{enter}\"{LINUX_BINARY}\" setup {flags}");
    let heading = format!("==> wsl: rayx setup {flags}");
    let _ = writeln!(out, "{}", heading.trim_end());
    runner
        .run_checked(&in_distribution(script.trim_end()).interactive())
        .map(|_| ())
        .map_err(|error| WslError::Failed(format!("rayx setup inside {DISTRIBUTION}: {error}")))
}
