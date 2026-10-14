//! The Linux guests `rayx` drives from a host: a WSL distribution on Windows, a Lima instance on
//! macOS. A guest gets the Linux `rayx` of the running version, optionally a clone of the current
//! checkout, and `rayx setup` run inside it with this console's stdio. WSL and Lima differ only in
//! how a script runs inside and in how the guest is created (`setup::wsl`, `setup::lima`).
//!
//! Nothing here reads a manifest or clones another repository: the project's dependencies are
//! resolved by Cargo inside the guest from crates.io or git.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cli::SetupArgs;
use crate::host::{Arch, CommandSpec, Outcome, Runner};
use crate::project::pins::DEFAULTS;

/// Where the Linux `rayx` goes inside a guest: the release installer's own location, so the
/// install receipt next to it lets `rayx self update` work there.
pub const LINUX_BINARY: &str = "$HOME/.rayx/bin/rayx";

/// The repository releases are published from.
pub const RELEASES: &str = "https://github.com/rayx-ui/RayX.CLI/releases/download";

/// Where a guest records the directories `--clone` created, one per line.
pub const CLONES_FILE: &str = "$HOME/.config/rayx/clones";

/// The file WSL-only versions of `rayx` wrote; still read, never written.
pub const LEGACY_CLONES_FILE: &str = "$HOME/.config/rayx/wsl-clones";

/// The kind of guest, which decides how a script runs inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestKind {
    /// A WSL distribution, driven with `wsl.exe` from Windows.
    Wsl,
    /// A Lima instance, driven with `limactl` from macOS.
    Lima,
}

impl GuestKind {
    /// The label progress lines carry (`==> wsl: ...`).
    pub fn label(self) -> &'static str {
        match self {
            GuestKind::Wsl => "wsl",
            GuestKind::Lima => "lima",
        }
    }
}

/// A guest by kind and name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guest {
    pub kind: GuestKind,
    pub name: String,
    /// A directory to put first on `PATH` for the host tool that drives the guest: Homebrew's
    /// `bin` while `limactl` is not on this process's `PATH` yet.
    pub path_dir: Option<PathBuf>,
}

impl Guest {
    pub fn wsl(name: impl Into<String>) -> Self {
        Self {
            kind: GuestKind::Wsl,
            name: name.into(),
            path_dir: None,
        }
    }

    pub fn lima(name: impl Into<String>) -> Self {
        Self {
            kind: GuestKind::Lima,
            name: name.into(),
            path_dir: None,
        }
    }

    /// Puts `dir` first on `PATH` for the commands that drive this guest.
    pub fn with_path_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.path_dir = dir;
        self
    }

    /// The command that runs `script` inside the guest with `bash -lc` as the default user. The
    /// script is one argument, so no login shell parses it.
    pub fn script(&self, script: &str) -> CommandSpec {
        let spec = match self.kind {
            GuestKind::Wsl => CommandSpec::new("wsl")
                .args(["-d", &self.name, "--exec", "bash", "-lc"])
                .arg(script),
            GuestKind::Lima => CommandSpec::new("limactl")
                .args(["shell", "--workdir", "/", &self.name, "--", "bash", "-lc"])
                .arg(script),
        };
        match &self.path_dir {
            Some(dir) => spec.env("PATH", path_with(dir)),
            None => spec,
        }
    }
}

/// This process's `PATH` with `dir` first.
pub fn path_with(dir: &Path) -> String {
    match std::env::var("PATH") {
        Ok(path) if !path.is_empty() => format!("{}:{path}", dir.display()),
        _ => dir.display().to_string(),
    }
}

/// The Ubuntu version every guest runs: the built-in pin.
pub fn pinned_ubuntu() -> &'static str {
    DEFAULTS.ubuntu
}

/// The WSL distribution of an Ubuntu version: `Ubuntu-24.04`.
pub fn wsl_distribution(ubuntu: &str) -> String {
    format!("Ubuntu-{ubuntu}")
}

/// The Lima template of an Ubuntu version: `template:ubuntu-24.04`. Never the unversioned
/// `template:ubuntu`, which follows Lima's idea of the newest release.
pub fn lima_template(ubuntu: &str) -> String {
    format!("template:ubuntu-{ubuntu}")
}

/// The release asset that holds the Linux `rayx` for a host: the static musl build. The installer
/// picks it by itself; `None` marks an architecture with no Linux release, which fails before any
/// download.
pub fn release_asset(arch: Arch) -> Option<&'static str> {
    match arch {
        Arch::X64 => Some("x86_64-unknown-linux-musl"),
        Arch::Arm64 => Some("aarch64-unknown-linux-musl"),
        Arch::Other => None,
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
        .any(|c| matches!(c, '"' | '`' | '$' | '\\' | '\'') || c.is_control())
    {
        return Err(format!("{dir} is not a usable Linux directory name"));
    }
    // A relative path would land on the host directory the tool starts in, and be recorded
    // relative to wherever a later command runs.
    if !expanded.starts_with('/') && !expanded.starts_with("$HOME") {
        return Err(format!(
            "{dir} is not an absolute Linux path: use /abs/dir or ~/dir"
        ));
    }
    Ok(expanded)
}

/// The script that clones `source` (a path the guest can read) into `target`, or fetches an
/// existing clone, and records `target` so `rayx wsl status` and `wsl compact --clean` find its
/// build output.
pub fn clone_script(source: &str, branch: Option<&str>, target: &str) -> Result<String, String> {
    if source.contains('\'') || branch.is_some_and(|b| b.contains('\'')) {
        return Err("the checkout path or branch contains a quote".to_string());
    }
    let branch_flag = branch.map_or(String::new(), |b| format!(" -b '{b}'"));
    let clones = CLONES_FILE;
    Ok(format!(
        "if [ -d \"{target}/.git\" ]; then git -C \"{target}\" fetch --all --prune; \
         else git -c safe.directory='*' clone{branch_flag} '{source}' \"{target}\"; fi && \
         mkdir -p \"$HOME/.config/rayx\" && {{ grep -qxF \"{target}\" \"{clones}\" 2>/dev/null || \
         echo \"{target}\" >> \"{clones}\"; }}"
    ))
}

/// The checkout `--clone` copies, as the guest sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloneSource {
    /// The checkout's directory name, which names the default target `~/<name>`.
    pub name: String,
    /// The checkout's path inside the guest (`/mnt/d/...` on WSL, the host path on Lima).
    pub path: String,
    /// The checked-out branch; `None` for a detached HEAD.
    pub branch: Option<String>,
}

/// The last non-empty component of a host path, split on both separators so the name is the same
/// on every host.
pub fn checkout_name(root: &Path) -> Option<String> {
    root.to_string_lossy()
        .split(['/', '\\'])
        .rfind(|part| !part.is_empty())
        .map(str::to_string)
}

/// The Linux `rayx` a guest should have and how to get it there.
#[derive(Clone, Debug)]
pub struct LinuxBinary {
    /// The version of the running `rayx`, which the Linux binary must match.
    pub version: String,
    /// The host's architecture: one without a Linux release fails before any download.
    pub arch: Arch,
    /// A Linux binary built on this machine, as a path the guest can read, used instead of the
    /// release installer (`RAYX_WSL_BINARY`, `RAYX_LIMA_BINARY`).
    pub override_path: Option<String>,
}

fn run_inside(guest: &Guest, runner: &mut Runner, script: &str, what: &str) -> Result<(), String> {
    runner
        .run_checked(&guest.script(script).interactive())
        .map(|_| ())
        .map_err(|error| format!("{what}: {error}"))
}

/// The text of a guest command's output: `wsl.exe` itself prints UTF-16, which arrives with NUL
/// bytes between letters.
fn text(outcome: &Outcome) -> String {
    outcome.stdout.replace('\0', "")
}

/// Puts the Linux `rayx` of the running version into the guest: copies the override binary, or
/// runs that release's own installer when `$HOME/.rayx/bin/rayx --version` is not `rayx <version>`.
pub fn ensure_linux_binary(
    guest: &Guest,
    binary: &LinuxBinary,
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<(), String> {
    let label = guest.kind.label();
    if let Some(source) = &binary.override_path {
        if source.contains('"') {
            return Err(format!("{source} is not a usable path"));
        }
        let _ = writeln!(out, "==> {label}: installing the Linux rayx from {source}");
        let script = format!(
            "mkdir -p \"$HOME/.rayx/bin\" && cp \"{source}\" \"{LINUX_BINARY}\" && chmod 755 \"{LINUX_BINARY}\""
        );
        return run_inside(guest, runner, &script, "copying the Linux rayx");
    }
    let version_script = format!("\"{LINUX_BINARY}\" --version");
    let installed = runner
        .query(&guest.script(&version_script))
        .ok()
        .filter(Outcome::is_success)
        .map(|outcome| text(&outcome).trim().to_string());
    if installed.as_deref() == Some(format!("rayx {}", binary.version).as_str()) {
        return Ok(());
    }
    // An architecture without a Linux release fails here, before anything is downloaded.
    release_asset(binary.arch)
        .ok_or_else(|| "this CPU architecture has no Linux rayx release".to_string())?;
    let url = format!(
        "{RELEASES}/v{version}/rayx-cli-installer.sh",
        version = binary.version
    );
    let _ = writeln!(
        out,
        "==> {label}: installing the Linux rayx {}",
        binary.version
    );
    // The release's own installer picks the musl archive, verifies it, installs into
    // `~/.rayx/bin`, adds it to PATH and writes the receipt `rayx self update` reads. `pipefail`
    // keeps a failed download from looking like an empty, successful script, and `RAYX_HOME` is
    // dropped so the binary lands where `LINUX_BINARY` expects it.
    let script = format!(
        "set -o pipefail; curl --proto '=https' --tlsv1.2 -LsSf '{url}' | env -u RAYX_HOME sh && \
         test -x \"{LINUX_BINARY}\""
    );
    run_inside(guest, runner, &script, "installing the Linux rayx")
}

/// Clones (or fetches) the checkout into the guest and returns the Linux directory it lives in.
/// `requested` is the `--clone` value: empty for the default `~/<checkout name>`.
pub fn clone_checkout(
    guest: &Guest,
    requested: &str,
    source: &CloneSource,
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<String, String> {
    let target = if requested.is_empty() {
        linux_dir(&format!("~/{}", source.name))
    } else {
        linux_dir(requested)
    }?;
    let script = clone_script(&source.path, source.branch.as_deref(), &target)?;
    let _ = writeln!(
        out,
        "==> {}: cloning {} into {target}",
        guest.kind.label(),
        source.path
    );
    run_inside(guest, runner, &script, "cloning the checkout")?;
    Ok(target)
}

/// Runs `rayx setup <flags>` inside the guest, in `directory` when a clone was made, with this
/// console's stdio so `sudo` prompts reach the user.
pub fn run_setup_inside(
    guest: &Guest,
    flags: &[String],
    directory: Option<&str>,
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<(), String> {
    let flags = flags.join(" ");
    let enter = directory.map_or(String::new(), |dir| format!("cd \"{dir}\" && "));
    let script = format!("{enter}\"{LINUX_BINARY}\" setup {flags}");
    let heading = format!("==> {}: rayx setup {flags}", guest.kind.label());
    let _ = writeln!(out, "{}", heading.trim_end());
    runner
        .run_checked(&guest.script(script.trim_end()).interactive())
        .map(|_| ())
        .map_err(|error| format!("rayx setup inside {}: {error}", guest.name))
}

/// The script that prints the guest's `VERSION_ID`.
pub const OS_VERSION_SCRIPT: &str = ". /etc/os-release; echo $VERSION_ID";

/// The Ubuntu version the guest runs (`24.04`), when it can be read.
pub fn os_version(guest: &Guest, runner: &mut Runner) -> Option<String> {
    runner
        .query(&guest.script(OS_VERSION_SCRIPT))
        .ok()
        .filter(Outcome::is_success)
        .map(|outcome| text(&outcome).trim().to_string())
        .filter(|version| !version.is_empty())
}

/// The script that prints, for every recorded clone with uncommitted or untracked changes, one
/// `DIRTY<TAB><dir>` line.
pub fn dirty_clones_script() -> String {
    format!(
        "for f in \"{CLONES_FILE}\" \"{LEGACY_CLONES_FILE}\"; do [ -f \"$f\" ] || continue; \
         while IFS= read -r d; do [ -d \"$d/.git\" ] || continue; \
         if [ -n \"$(git -C \"$d\" status --porcelain 2>/dev/null)\" ]; then printf 'DIRTY\\t%s\\n' \"$d\"; fi; \
         done < \"$f\"; done"
    )
}

/// The recorded clones in the guest that have changes `rayx` would lose by replacing or removing
/// the guest.
pub fn dirty_clones(guest: &Guest, runner: &mut Runner) -> Vec<String> {
    runner
        .query(&guest.script(&dirty_clones_script()))
        .ok()
        .filter(Outcome::is_success)
        .map(|outcome| {
            text(&outcome)
                .lines()
                .filter_map(|line| line.strip_prefix("DIRTY\t"))
                .map(|dir| dir.trim().to_string())
                .filter(|dir| dir.starts_with('/'))
                .collect()
        })
        .unwrap_or_default()
}

/// The git checkout `--clone` copies, on the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkout {
    /// The checkout's top-level directory.
    pub root: PathBuf,
    /// The checked-out branch; `None` for a detached HEAD.
    pub branch: Option<String>,
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

/// The flags `rayx setup` runs with inside a guest: the set flags, `--check` and `--yes`.
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
