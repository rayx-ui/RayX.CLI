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

/// The first release with `setup --update` and the refresh offer after `self update`. A guest
/// older than this has no `setup --update` to run.
pub const MIN_REFRESH_RELEASE: Release = Release(Version::new(1, 1, 0));

/// A published release: `X.Y.Z`, without a prerelease or build part.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Release(Version);

/// Why a text is not a release name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseError {
    text: String,
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a release name: use the form X.Y.Z, for example 1.0.1",
            self.text
        )
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
/// `latest` and everything else are refused.
pub fn parse(text: &str) -> Result<Release, ReleaseError> {
    let error = || ReleaseError {
        text: text.to_string(),
    };
    let bare = text.strip_prefix('v').unwrap_or(text);
    let version = Version::parse(bare).map_err(|_| error())?;
    Release::from_version(&version).ok_or_else(error)
}

/// Asks whether `release` is published with a headers-only request for its installer, so nothing
/// is downloaded: success is `Exists`, an HTTP 404 (curl exit 22) is `Missing`, anything else
/// `Unknown`.
pub fn exists(runner: &mut Runner, release: &Release) -> Existence {
    let spec = CommandSpec::new("curl").args([
        "--proto",
        "=https",
        "--tlsv1.2",
        "-sfIL",
        &release.installer_url(),
    ]);
    match runner.query(&spec) {
        Ok(outcome) if outcome.is_success() => Existence::Exists,
        Ok(outcome) if outcome.code == Some(22) => Existence::Missing,
        Ok(outcome) => Existence::Unknown(match outcome.code {
            Some(code) => format!("curl exited with {code}"),
            None => "curl was ended by a signal".to_string(),
        }),
        Err(error) => Existence::Unknown(error.to_string()),
    }
}
