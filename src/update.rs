//! `rayx self update [--check]`: replaces the installed binary with the latest GitHub release.
//!
//! The release installers (cargo-dist's shell and PowerShell scripts) leave an install receipt;
//! `rayx` updates itself only when that receipt is for the running executable. The installer
//! script that runs is the release's own, which verifies the archive checksum and replaces the
//! binary, also on Windows where the running executable cannot be overwritten in place. A `rayx`
//! from `cargo install` or a development build is told how to update instead of being touched.

use std::cmp::Ordering;
use std::io::Write;
use std::path::{Path, PathBuf};

use axoupdater::{AxoUpdater, ReleaseSource, ReleaseSourceType};
use semver::Version;

/// The cargo-dist application name: the package the releases are built from.
pub const APP_NAME: &str = "rayx-cli";
const OWNER: &str = "rayx-ui";
const REPOSITORY: &str = "RayX.CLI";

/// The commands that install a release from scratch.
pub const INSTALL_HELP: &str = "install a release:\n  \
     macOS and Linux: curl --proto '=https' --tlsv1.2 -LsSf \
     https://github.com/rayx-ui/RayX.CLI/releases/latest/download/rayx-cli-installer.sh | sh\n  \
     Windows:         powershell -ExecutionPolicy Bypass -c \"irm \
     https://github.com/rayx-ui/RayX.CLI/releases/latest/download/rayx-cli-installer.ps1 | iex\"";

/// Where the running `rayx` came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallSource {
    /// A release installer wrote an install receipt for this executable.
    Installer,
    /// A build in a cargo `target` directory.
    DevelopmentBuild,
    /// `cargo install` (or a copy) in `~/.cargo/bin`, without a receipt.
    Cargo,
    /// Anything else without a receipt.
    Unknown,
}

/// The install source of the executable at `exe`, given whether an install receipt names it.
pub fn classify(exe: &Path, receipt_matches: bool) -> InstallSource {
    if receipt_matches {
        return InstallSource::Installer;
    }
    // Split on both separators: the path names the running executable, but also arrives from
    // receipts and tests written on another system.
    let components: Vec<String> = exe
        .to_string_lossy()
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    // A cargo `target` directory: `target/debug`, `target/release`, `target/<triple>/release`,
    // `target/dist`.
    let in_target = components
        .iter()
        .position(|c| c == "target")
        .is_some_and(|at| {
            components[at + 1..]
                .iter()
                .any(|c| matches!(c.as_str(), "debug" | "release" | "dist"))
        });
    if in_target {
        return InstallSource::DevelopmentBuild;
    }
    let cargo_bin = components
        .windows(2)
        .any(|pair| pair[0] == ".cargo" && pair[1] == "bin");
    if cargo_bin {
        InstallSource::Cargo
    } else {
        InstallSource::Unknown
    }
}

impl InstallSource {
    /// What to tell the owner of a `rayx` this command will not overwrite.
    pub fn instructions(&self) -> String {
        match self {
            InstallSource::Installer => String::new(),
            InstallSource::DevelopmentBuild => format!(
                "this rayx is a development build, so `self update` leaves it alone. Rebuild it \
                 from the checkout (`cargo build --release`), or {INSTALL_HELP}"
            ),
            InstallSource::Cargo => format!(
                "this rayx was installed with cargo, not by the release installers, so `self \
                 update` leaves it alone. Reinstall it the way you installed it \
                 (`cargo install --git https://github.com/{OWNER}/{REPOSITORY} rayx-cli --force`), \
                 or {INSTALL_HELP}"
            ),
            InstallSource::Unknown => format!(
                "this rayx was not installed by the release installers (no install receipt names \
                 it), so `self update` leaves it alone. To {INSTALL_HELP}"
            ),
        }
    }
}

/// How the installed version compares with the latest release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    UpToDate,
    UpdateAvailable,
    /// Newer than the latest release: a development version.
    Ahead,
}

pub fn compare(installed: &Version, latest: &Version) -> Standing {
    match installed.cmp(latest) {
        Ordering::Less => Standing::UpdateAvailable,
        Ordering::Equal => Standing::UpToDate,
        Ordering::Greater => Standing::Ahead,
    }
}

/// What `self update` needs from the outside world.
pub trait Backend {
    /// The version of the latest release.
    fn latest(&mut self) -> Result<Version, String>;
    /// Whether an install receipt names the running executable.
    fn receipt_matches(&mut self) -> bool;
    /// The running executable.
    fn executable(&self) -> PathBuf;
    /// Runs the release installer; the version it installed.
    fn install(&mut self) -> Result<Version, String>;
}

/// The body of `rayx self update [--check]`; returns the exit code.
pub fn run_with(
    check: bool,
    installed: &Version,
    backend: &mut dyn Backend,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let latest = match backend.latest() {
        Ok(latest) => latest,
        Err(error) => {
            let _ = writeln!(err, "rayx: cannot find the latest release: {error}");
            return 1;
        }
    };
    let standing = compare(installed, &latest);
    if check {
        let verdict = match standing {
            Standing::UpToDate => "rayx is up to date".to_string(),
            Standing::UpdateAvailable => {
                "an update is available: run `rayx self update`".to_string()
            }
            Standing::Ahead => "this rayx is newer than the latest release".to_string(),
        };
        let _ = writeln!(
            out,
            "installed {installed}, latest release {latest}: {verdict}"
        );
        return 0;
    }
    if standing != Standing::UpdateAvailable {
        let _ = writeln!(out, "rayx {installed} is the latest release ({latest}).");
        return 0;
    }
    let source = classify(&backend.executable(), backend.receipt_matches());
    if source != InstallSource::Installer {
        let _ = writeln!(err, "rayx: {}", source.instructions());
        return 1;
    }
    match backend.install() {
        Ok(installed_now) => {
            let _ = writeln!(out, "Updated rayx {installed} -> {installed_now}.");
            0
        }
        Err(error) => {
            let _ = writeln!(err, "rayx: the update failed: {error}");
            1
        }
    }
}

/// Runs `rayx self update` against GitHub and returns the exit code.
pub fn run(check: bool) -> u8 {
    let Ok(installed) = Version::parse(env!("CARGO_PKG_VERSION")) else {
        eprintln!("rayx: this build has no valid version");
        return 1;
    };
    let mut backend = GithubBackend::new(&installed);
    run_with(
        check,
        &installed,
        &mut backend,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

/// The real backend: axoupdater against the GitHub releases of this repository.
struct GithubBackend {
    updater: AxoUpdater,
    receipt: bool,
}

impl GithubBackend {
    fn new(installed: &Version) -> Self {
        let mut updater = AxoUpdater::new_for(APP_NAME);
        updater.set_release_source(ReleaseSource {
            release_type: ReleaseSourceType::GitHub,
            owner: OWNER.to_string(),
            name: REPOSITORY.to_string(),
            app_name: APP_NAME.to_string(),
        });
        let receipt = updater.load_receipt().is_ok();
        if !receipt {
            // Without a receipt there is no version to read; `--check` still compares.
            if let Ok(version) = axoupdater::Version::parse(&installed.to_string()) {
                let _ = updater.set_current_version(version);
            }
        }
        Self { updater, receipt }
    }

    fn runtime() -> Result<tokio::runtime::Runtime, String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
    }
}

impl Backend for GithubBackend {
    fn latest(&mut self) -> Result<Version, String> {
        let runtime = Self::runtime()?;
        let latest = runtime
            .block_on(self.updater.query_new_version())
            .map_err(|error| error.to_string())?
            .cloned()
            .ok_or_else(|| "no release was found".to_string())?;
        Version::parse(&latest.to_string()).map_err(|error| error.to_string())
    }

    fn receipt_matches(&mut self) -> bool {
        self.receipt
            && self
                .updater
                .check_receipt_is_for_this_executable()
                .unwrap_or(false)
    }

    fn executable(&self) -> PathBuf {
        std::env::current_exe().unwrap_or_default()
    }

    fn install(&mut self) -> Result<Version, String> {
        let result = self
            .updater
            .run_sync()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "the installer reported nothing to update".to_string())?;
        Version::parse(&result.new_version.to_string()).map_err(|error| error.to_string())
    }
}
