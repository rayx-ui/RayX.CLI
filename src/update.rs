//! `rayx self update [--check] [--release X.Y.Z]`: replaces the installed binary with the latest
//! GitHub release, or with exactly the named one.
//!
//! The release installers (cargo-dist's shell and PowerShell scripts) leave an install receipt;
//! `rayx` updates itself only when that receipt is for the running executable. The installer
//! script that runs is the release's own, which verifies the archive checksum and replaces the
//! binary, also on Windows where the running executable cannot be overwritten in place. A `rayx`
//! from `cargo install` or a development build is told how to update instead of being touched.

use std::cmp::Ordering;
use std::io::Write;
use std::path::{Path, PathBuf};

use axoupdater::{AxoUpdater, ReleaseSource, ReleaseSourceType, UpdateRequest};
use semver::Version;

use crate::release::{self, Direction, Existence, MIN_REFRESH_RELEASE, Release};

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
    /// Whether `release` is published.
    fn release_exists(&mut self, release: &Release) -> Existence;
    /// Runs the installer of exactly `release`, also an older one; the version it installed.
    fn install_release(&mut self, release: &Release) -> Result<Version, String>;
}

/// What the refresh after an update needs from the outside world.
pub trait Refresher {
    /// Whether this machine has anything to refresh: a `setup.json` or a `guests.json`.
    fn records_exist(&mut self) -> bool;
    /// Asks a yes/no question; `None` when nobody can answer (no terminal).
    fn confirm(&mut self, question: &str) -> Option<bool>;
    /// Runs `<executable> setup --update [--yes]` in this console and returns its exit code.
    fn refresh(&mut self, executable: &Path, yes: bool) -> u8;
}

/// The body of `rayx self update [--check]` without a refresh; returns the exit code.
pub fn run_with(
    check: bool,
    installed: &Version,
    backend: &mut dyn Backend,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    run_with_refresh(check, false, installed, backend, None, out, err)
}

/// The body of `rayx self update [--check] [--yes]`: after a successful install it offers to
/// refresh this machine's setup and guests with the new binary; returns the exit code.
pub fn run_with_refresh(
    check: bool,
    yes: bool,
    installed: &Version,
    backend: &mut dyn Backend,
    refresher: Option<&mut dyn Refresher>,
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
    // The path this rayx was started from: on Windows the old binary is renamed away during the
    // install, so the refresh must run the executable at the original path, which is the new one.
    let executable = backend.executable();
    match backend.install() {
        Ok(installed_now) => {
            let _ = writeln!(out, "Updated rayx {installed} -> {installed_now}.");
            match refresher {
                Some(refresher) => offer_refresh(yes, &installed_now, &executable, refresher, out),
                None => 0,
            }
        }
        Err(error) => {
            let _ = writeln!(err, "rayx: the update failed: {error}");
            1
        }
    }
}

/// The body of `rayx self update --release <X.Y.Z> [--check] [--yes]`: installs exactly that
/// release, asking first when it is older than the installed one; returns the exit code. Nothing
/// is remembered: a later plain `self update` goes to the latest release.
#[allow(clippy::too_many_arguments)]
pub fn run_release(
    requested: &Release,
    check: bool,
    yes: bool,
    installed: &Version,
    backend: &mut dyn Backend,
    mut refresher: Option<&mut dyn Refresher>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    match backend.release_exists(requested) {
        Existence::Exists => {}
        Existence::Missing => {
            let _ = writeln!(err, "rayx: release {requested} was not found");
            return 1;
        }
        Existence::Unknown(reason) => {
            let _ = writeln!(
                err,
                "rayx: cannot check whether release {requested} exists: {reason}"
            );
            return 1;
        }
    }
    let direction = requested.direction_from(installed);
    if check {
        let _ = writeln!(
            out,
            "installed {installed}, requested release {requested}: {}",
            direction.word()
        );
        return 0;
    }
    if direction == Direction::Same {
        let _ = writeln!(out, "rayx {installed} is already installed.");
        return 0;
    }
    let source = classify(&backend.executable(), backend.receipt_matches());
    if source != InstallSource::Installer {
        let _ = writeln!(err, "rayx: {}", source.instructions());
        return 1;
    }
    if direction == Direction::Downgrade && !yes {
        let question = format!("Install rayx {requested} over {installed}, an older release?");
        match refresher.as_deref_mut().and_then(|r| r.confirm(&question)) {
            Some(true) => {}
            Some(false) => {
                let _ = writeln!(err, "rayx: nothing was changed");
                return 1;
            }
            None => {
                let _ = writeln!(
                    err,
                    "rayx: installing rayx {requested} over {installed} is a downgrade and needs \
                     an answer: repeat it with `--yes` to confirm without a terminal"
                );
                return 1;
            }
        }
    }
    // The path this rayx was started from: the refresh must run the executable at the original
    // path, which is the new one once the install replaced it.
    let executable = backend.executable();
    let installed_now = match backend.install_release(requested) {
        Ok(installed_now) => installed_now,
        Err(error) => {
            let _ = writeln!(err, "rayx: the update failed: {error}");
            return 1;
        }
    };
    let _ = writeln!(out, "Updated rayx {installed} -> {installed_now}.");
    let has_refresh =
        Release::from_version(&installed_now).is_some_and(|release| release >= MIN_REFRESH_RELEASE);
    if !has_refresh {
        let _ = writeln!(
            out,
            "rayx {installed_now} predates the guest refresh: no refresh was offered"
        );
        return 0;
    }
    if direction == Direction::Downgrade {
        return 0;
    }
    match refresher {
        Some(refresher) => offer_refresh(yes, &installed_now, &executable, refresher, out),
        None => 0,
    }
}

/// Offers the refresh after an install and runs it when wanted; the refresh's exit code becomes
/// the command's.
fn offer_refresh(
    yes: bool,
    new_version: &Version,
    executable: &Path,
    refresher: &mut dyn Refresher,
    out: &mut dyn Write,
) -> u8 {
    if !refresher.records_exist() {
        return 0;
    }
    let wanted = yes
        || refresher
            .confirm(&format!(
                "Refresh this machine's setup and guests with rayx {new_version}?"
            ))
            .unwrap_or(false);
    if wanted {
        return refresher.refresh(executable, yes);
    }
    let _ = writeln!(
        out,
        "run `rayx setup --update` to refresh the setup and guests"
    );
    0
}

/// Runs `rayx self update [--release X.Y.Z]` against GitHub and returns the exit code.
pub fn run(check: bool, yes: bool, requested: Option<&str>) -> u8 {
    let Ok(installed) = Version::parse(env!("CARGO_PKG_VERSION")) else {
        eprintln!("rayx: this build has no valid version");
        return 1;
    };
    // The name is checked before the network is asked.
    let requested = match requested.map(release::parse).transpose() {
        Ok(requested) => requested,
        Err(error) => {
            eprintln!("rayx: {error}");
            return 1;
        }
    };
    let mut backend = GithubBackend::new(&installed);
    let mut refresher = SystemRefresher;
    if let Some(requested) = requested {
        return run_release(
            &requested,
            check,
            yes,
            &installed,
            &mut backend,
            Some(&mut refresher),
            &mut std::io::stdout(),
            &mut std::io::stderr(),
        );
    }
    run_with_refresh(
        check,
        yes,
        &installed,
        &mut backend,
        Some(&mut refresher),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

/// The real refresh: the records on this machine, the terminal, and the new binary.
struct SystemRefresher;

impl Refresher for SystemRefresher {
    fn records_exist(&mut self) -> bool {
        let machine = crate::setup::SystemMachine;
        let os = crate::host::facts().os;
        [
            crate::setup::records::setup_path(&machine, os),
            crate::setup::records::guests_path(&machine, os),
        ]
        .into_iter()
        .flatten()
        .any(|path| path.is_file())
    }

    fn confirm(&mut self, question: &str) -> Option<bool> {
        use std::io::IsTerminal;
        (std::io::stdin().is_terminal() && std::io::stdout().is_terminal())
            .then(|| crate::setup::ask_on_terminal(question))
    }

    fn refresh(&mut self, executable: &Path, yes: bool) -> u8 {
        let mut spec = crate::host::CommandSpec::new(executable.display().to_string())
            .args(["setup", "--update"])
            .interactive();
        if yes {
            spec = spec.arg("--yes");
        }
        match crate::host::Runner::execute().run(&spec) {
            Ok(outcome) => outcome
                .code
                .and_then(|code| u8::try_from(code).ok())
                .unwrap_or(1),
            Err(error) => {
                eprintln!("rayx: the refresh could not start: {error}");
                1
            }
        }
    }
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

    fn release_exists(&mut self, release: &Release) -> Existence {
        release::exists(&mut crate::host::Runner::execute(), release)
    }

    fn install_release(&mut self, release: &Release) -> Result<Version, String> {
        // axoupdater refuses to install an older release unless an update is forced.
        self.updater
            .configure_version_specifier(UpdateRequest::SpecificVersion(release.to_string()))
            .always_update(true);
        self.install()
    }
}
