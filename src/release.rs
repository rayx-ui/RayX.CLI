//! Release names: the one place that knows what a `rayx` release is called, how two releases
//! compare, where a release's installer lives and whether a release is published.
//!
//! `self update --release` and the guest installs all parse, order and check releases here, so
//! none of them grows its own rules.

use std::cmp::Ordering;
use std::fmt;

use semver::Version;

use crate::host::{CommandSpec, Runner};

/// Where releases are published from.
pub const RELEASES: &str = "https://github.com/rayx-ui/RayX.CLI/releases/download";

/// The first release of this `rayx`, and the oldest one `--release` can name. `--release` names
/// this CLI's releases only; the old CLI's (1.0.0 to 1.4.1) are in `rayx-ui/RayX.CLI.Old`.
pub const OLDEST_RELEASE: Release = Release(Version::new(1, 0, 0));

/// The first release with `setup --update` and the refresh offer after `self update`: every
/// release of this `rayx`. A guest older than this has no `setup --update` to run.
pub const MIN_REFRESH_RELEASE: Release = OLDEST_RELEASE;

/// A published release: `X.Y.Z`, without a prerelease or build part.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Release(Version);

/// Why a text is not a release name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseError {
    text: String,
    /// It is a version, but older than [`OLDEST_RELEASE`].
    too_old: bool,
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.too_old {
            write!(
                f,
                "`{}` is older than {OLDEST_RELEASE}, the first release of this rayx; there is none \
                 before it (the old CLI's releases are in rayx-ui/RayX.CLI.Old)",
                self.text
            )
        } else {
            write!(
                f,
                "`{}` is not a release name: use the form X.Y.Z, for example 1.0.1",
                self.text
            )
        }
    }
}

impl std::error::Error for ReleaseError {}

/// How a requested release compares with the one in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Upgrade,
    Same,
    Downgrade,
}

impl Direction {
    /// The word `--check` prints.
    pub fn word(self) -> &'static str {
        match self {
            Direction::Upgrade => "upgrade",
            Direction::Same => "same",
            Direction::Downgrade => "downgrade",
        }
    }
}

/// Whether a release is published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Existence {
    Exists,
    Missing,
    /// The question could not be answered; the reason is for the owner.
    Unknown(String),
}

impl Release {
    /// The release a version names; `None` for a prerelease or a build with metadata.
    pub fn from_version(version: &Version) -> Option<Self> {
        (version.pre.is_empty() && version.build.is_empty()).then(|| Self(version.clone()))
    }

    pub fn version(&self) -> &Version {
        &self.0
    }

    /// What installing `self` does to a `rayx` at `installed`.
    pub fn direction_from(&self, installed: &Version) -> Direction {
        match self.0.cmp(installed) {
            Ordering::Greater => Direction::Upgrade,
            Ordering::Equal => Direction::Same,
            Ordering::Less => Direction::Downgrade,
        }
    }

    /// The installer script of this release (the shell one, which guests run).
    pub fn installer_url(&self) -> String {
        installer_url_for(&self.to_string())
    }
}

impl fmt::Display for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The installer script of the release tagged `v<version>`; also for a version that is no
/// published release name (a development build), which simply has no installer to find.
pub fn installer_url_for(version: &str) -> String {
    format!("{RELEASES}/v{version}/rayx-cli-installer.sh")
}

/// Parses `X.Y.Z`; a leading `v` is accepted and dropped. Prereleases, build metadata, ranges,
/// `latest`, releases older than [`OLDEST_RELEASE`] and everything else are refused.
pub fn parse(text: &str) -> Result<Release, ReleaseError> {
    let error = |too_old| ReleaseError {
        text: text.to_string(),
        too_old,
    };
    let bare = text.strip_prefix('v').unwrap_or(text);
    let version = Version::parse(bare).map_err(|_| error(false))?;
    let release = Release::from_version(&version).ok_or_else(|| error(false))?;
    if release < OLDEST_RELEASE {
        return Err(error(true));
    }
    Ok(release)
}

/// Asks whether `release` is published with a headers-only request for its installer, so nothing
/// is downloaded. The answer is the final HTTP status (`-w`), not curl's `-f` exit code: macOS curl
/// ends a HEAD request answered with a 404 and a body length with exit 56, not 22. A 2xx is
/// `Exists`, a 404 or 410 is `Missing`, anything else (another status, no answer) `Unknown`.
pub fn exists(runner: &mut Runner, release: &Release) -> Existence {
    let spec = CommandSpec::new("curl").args([
        "--proto",
        "=https",
        "--tlsv1.2",
        "-sIL",
        "-w",
        "\nstatus=%{http_code}\n",
        &release.installer_url(),
    ]);
    match runner.query(&spec) {
        Ok(outcome) => match (outcome.code, status_of(&outcome.stdout)) {
            (Some(0), Some(200..=299)) => Existence::Exists,
            (_, Some(404 | 410)) => Existence::Missing,
            (Some(_), Some(status)) if status >= 100 && !(200..=299).contains(&status) => {
                Existence::Unknown(format!("the server answered HTTP {status}"))
            }
            (Some(code), _) => Existence::Unknown(format!("curl exited with {code}")),
            (None, _) => Existence::Unknown("curl was ended by a signal".to_string()),
        },
        Err(error) => Existence::Unknown(error.to_string()),
    }
}

/// The status curl's `-w` appended after the headers; `None` when it printed none.
fn status_of(stdout: &str) -> Option<u16> {
    stdout
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("status="))
        .and_then(|code| code.parse().ok())
}
