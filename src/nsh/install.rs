//! Installing a pin into the store (design §4.3, D11): the release's distribution for this host
//! when there is one, else a checkout at the commit with the CLI built inside it.
//!
//! Downloads run `curl` and the source build `git`, `rustup` and `cargo`, all through
//! `host::Runner`; the digest is checked and the archive unpacked in Rust. Both paths end with
//! the CLI's `--version`, which must name the lock's NSH version and the pin's commit. Nothing is
//! written outside the store and the system temporary directory.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use super::pin::{Pin, ReleaseSource, Source};
use super::store::{Kind, Receipt, Store, rfc3339};
use crate::host::{Arch, CommandSpec, HostFacts, Mode, Os, Runner};
use crate::setup::{Machine, rust};

/// The release platform of a host, as the fork's and upstream's archives name it; `None` where
/// no archive is built (macOS x64, other architectures), which installs from source.
pub fn platform(host: &HostFacts) -> Option<&'static str> {
    match (host.os, host.arch) {
        (Os::MacOs, Arch::Arm64) => Some("macos-arm64"),
        (Os::Linux, Arch::X64) => Some("linux-x86_64"),
        (Os::Linux, Arch::Arm64) => Some("linux-arm64"),
        (Os::Windows, Arch::X64) => Some("windows-x86_64"),
        (Os::Windows, Arch::Arm64) => Some("windows-arm64"),
        _ => None,
    }
}

/// `nsh-<platform>.tar.gz`, `.zip` on Windows.
pub fn archive_name(platform: &str) -> String {
    if platform.starts_with("windows-") {
        format!("nsh-{platform}.zip")
    } else {
        format!("nsh-{platform}.tar.gz")
    }
}

/// `https://github.com/<repository>/releases/download/<tag>/<archive>`.
pub fn archive_url(release: &ReleaseSource, archive: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/{}/{archive}",
        release.repository, release.tag
    )
}

/// The CLI's file name on the host.
fn exe(os: Os) -> &'static str {
    if os == Os::Windows { "nsh.exe" } else { "nsh" }
}

/// The CLI of a distribution entry, relative to the entry.
pub fn distribution_cli(os: Os) -> String {
    format!("nsh/bin/{}", exe(os))
}

/// The CLI of a source entry, relative to the entry.
pub fn source_cli(os: Os) -> String {
    format!("source/target/release/{}", exe(os))
}

/// Removes `CARGO` from a command rayx runs for nsh: a `CARGO` set by `cargo run` makes nsh's
/// build bypass rustup's proxy (design §3).
pub fn without_cargo(spec: CommandSpec) -> CommandSpec {
    spec.unset("CARGO")
}

/// A command run in an NSH checkout (its toolchain install and its CLI build): without `CARGO`,
/// and without `RUSTUP_TOOLCHAIN`, which rustup's proxies set for whatever ran rayx (`cargo run`,
/// `cargo xtask`) and which would override NSH's own `rust-toolchain.toml`.
pub fn in_nsh_checkout(spec: CommandSpec) -> CommandSpec {
    without_cargo(spec).unset("RUSTUP_TOOLCHAIN")
}

/// The Rust programs an install runs, found as setup's Rust steps find them: on `PATH`, else in
/// `~/.cargo/bin`, which a `rayx setup` run that has just installed rustup does not have on its
/// own `PATH` yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustTools {
    pub rustup: String,
    pub cargo: String,
}

impl RustTools {
    /// The bare names, for whatever `PATH` offers.
    pub fn on_path() -> Self {
        Self {
            rustup: "rustup".to_string(),
            cargo: "cargo".to_string(),
        }
    }

    /// The programs `machine` has.
    pub fn find(machine: &dyn Machine, os: Os) -> Self {
        Self {
            rustup: rust::rustup_program_on(machine, os),
            cargo: rust::tool_program_on(machine, os, "cargo"),
        }
    }
}

/// What `nsh --version` printed: `nsh 0.2.0 (327bb9e)` or `nsh 0.2.0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliVersion {
    pub version: String,
    /// The short commit, when the build knew it.
    pub commit: Option<String>,
}

/// Reads the first line of `nsh --version`.
pub fn parse_version(text: &str) -> Option<CliVersion> {
    let line = text.lines().next()?.trim();
    let rest = line.strip_prefix("nsh ")?;
    let (version, commit) = match rest.split_once(' ') {
        Some((version, commit)) => (
            version,
            Some(
                commit
                    .trim()
                    .trim_start_matches('(')
                    .trim_end_matches(')')
                    .to_string(),
            ),
        ),
        None => (rest, None),
    };
    Some(CliVersion {
        version: version.to_string(),
        commit: commit.filter(|c| !c.is_empty()),
    })
}

/// Checks that `cli --version` names the pin's NSH version and commit; returns what it printed.
pub fn check_cli(runner: &mut Runner, cli: &Path, pin: &Pin) -> Result<CliVersion> {
    let spec = without_cargo(CommandSpec::new(cli.display().to_string()).arg("--version"));
    let outcome = runner
        .query(&spec)
        .with_context(|| format!("running {} --version", cli.display()))?;
    if !outcome.is_success() {
        bail!(
            "{} --version failed: {}",
            cli.display(),
            outcome.stderr.trim()
        );
    }
    let found = parse_version(&outcome.stdout).ok_or_else(|| {
        anyhow!(
            "{} --version printed {:?}, not `nsh <version> (<commit>)`",
            cli.display(),
            outcome.stdout.trim()
        )
    })?;
    if let Some(want) = &pin.version
        && &found.version != want
    {
        bail!(
            "the nsh at {} is version {}, but the project's NSH crates are {want}",
            cli.display(),
            found.version
        );
    }
    if let Some(commit) = pin.commit() {
        match &found.commit {
            Some(short) if commit.starts_with(short.as_str()) && short.len() >= 7 => {}
            Some(short) => bail!(
                "the nsh at {} was built from {short}, but the pin is {}",
                cli.display(),
                &commit[..12.min(commit.len())]
            ),
            None => bail!(
                "the nsh at {} names no commit, so it cannot be checked against the pin {}",
                cli.display(),
                &commit[..12.min(commit.len())]
            ),
        }
    }
    Ok(found)
}

/// The result of an install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub receipt: Receipt,
    pub cli: PathBuf,
    /// Whether the entry was complete before (nothing was done).
    pub already: bool,
}

/// What an install works with.
pub struct Install<'a> {
    pub runner: &'a mut Runner,
    pub store: &'a Store,
    pub host: &'a HostFacts,
    /// The `rustup` and `cargo` a source build runs.
    pub tools: &'a RustTools,
    /// Progress lines.
    pub out: &'a mut dyn Write,
}

impl Install<'_> {
    /// Whether this is `--check`: commands are printed and nothing is written, not even the lock
    /// or an entry's directories. A recording runner (tests) writes into its temporary store like
    /// a real install.
    fn dry_run(&self) -> bool {
        self.runner.mode() == Mode::Print
    }

    /// Installs `pin` (a git or release pin; a path pin has no store entry), or finds it installed.
    pub fn run(&mut self, pin: &Pin) -> Result<Installed> {
        let id = pin
            .id()
            .ok_or_else(|| anyhow!("a path checkout of NSH is built in place, not installed"))?;
        let _lock = if self.dry_run() {
            None
        } else {
            Some(self.store.lock(&id, self.out)?)
        };
        if let Some(installed) = self.complete(pin, &id) {
            return Ok(installed);
        }
        if !self.dry_run() && self.store.receipt(&id).is_some() {
            // A receipt whose CLI no longer answers as the pin wants: the entry is unfinished
            // again until the new install completes it, so an interrupted repair is never
            // reported as installed.
            let _ = writeln!(
                self.out,
                "rayx: nsh {id} does not answer as nsh {} should; installing it again",
                pin.label()
            );
            self.store.unfinish(&id)?;
        }
        // A `rayx.toml` tag must still name its recorded commit.
        super::pin::verify_tag(self.runner, pin)?;
        if let (Some(release), Some(platform)) = (pin.release(), platform(self.host)) {
            if let Some(installed) = self.install_release(pin, &id, &release, platform)? {
                return Ok(installed);
            }
        } else if pin.release().is_some() {
            let _ = writeln!(
                self.out,
                "rayx: no NSH archive is built for this host ({} {}); building nsh {id} from source",
                self.host.os.as_str(),
                self.host.arch.as_str()
            );
        }
        self.build_source(pin, &id)
    }

    /// The entry when it is complete and its CLI still answers as the pin wants.
    fn complete(&mut self, pin: &Pin, id: &str) -> Option<Installed> {
        let receipt = self.store.receipt(id)?;
        let cli = self.store.cli(&receipt);
        check_cli(self.runner, &cli, pin).ok()?;
        Some(Installed {
            receipt,
            cli,
            already: true,
        })
    }

    /// Downloads, checks and unpacks the release archive; `Ok(None)` when the release has no
    /// archive for this host, which falls back to source.
    fn install_release(
        &mut self,
        pin: &Pin,
        id: &str,
        release: &ReleaseSource,
        platform: &str,
    ) -> Result<Option<Installed>> {
        let archive = archive_name(platform);
        let url = archive_url(release, &archive);
        let dir = self.store.dir(id);
        let downloads = dir.join("download");
        if !self.dry_run() {
            std::fs::create_dir_all(&downloads)
                .with_context(|| format!("creating {}", downloads.display()))?;
        }
        let digest_file = downloads.join(format!("{archive}.sha256"));
        let archive_file = downloads.join(&archive);
        let _ = writeln!(self.out, "==> nsh: downloading {url}");
        if !self.download(&format!("{url}.sha256"), &digest_file)? {
            let _ = writeln!(
                self.out,
                "rayx: the release {} of {} has no {archive}; building nsh {id} from source",
                release.tag, release.repository
            );
            let _ = std::fs::remove_dir_all(&downloads);
            return Ok(None);
        }
        if !self.download(&url, &archive_file)? {
            bail!("{url} has a digest but no archive");
        }
        if self.dry_run() {
            return Ok(Some(self.dry_run_result(pin, id, Kind::Distribution, url)));
        }
        let want = read_digest(&digest_file)?;
        let found = sha256_of(&archive_file)?;
        if want != found {
            let _ = std::fs::remove_dir_all(&downloads);
            bail!(
                "{archive} does not match its digest: the release says {want}, the download is {found}; nothing was installed"
            );
        }
        let target = dir.join("nsh");
        if target.exists() {
            std::fs::remove_dir_all(&target)
                .with_context(|| format!("removing the unfinished {}", target.display()))?;
        }
        unpack(&archive_file, &dir).with_context(|| format!("unpacking {archive}"))?;
        if !target.join("bin").is_dir() {
            bail!("{archive} holds no nsh/bin directory");
        }
        let _ = std::fs::remove_dir_all(&downloads);
        let cli_rel = distribution_cli(self.host.os);
        self.finish(pin, id, Kind::Distribution, url, cli_rel)
            .map(Some)
    }

    /// `curl` into `file`; `Ok(false)` when the server says the file does not exist.
    fn download(&mut self, url: &str, file: &Path) -> Result<bool> {
        let spec = CommandSpec::new("curl").args([
            "--proto",
            "=https",
            "--tlsv1.2",
            "-sSL",
            "-w",
            "%{http_code}",
            "-o",
            &file.display().to_string(),
            url,
        ]);
        let outcome = self
            .runner
            .run(&spec)
            .with_context(|| format!("downloading {url}"))?;
        let status: Option<u16> = outcome.stdout.trim().parse().ok();
        match status {
            _ if self.dry_run() => Ok(true),
            Some(200..=299) if outcome.is_success() => Ok(true),
            Some(404 | 410) => {
                let _ = std::fs::remove_file(file);
                Ok(false)
            }
            other => bail!(
                "downloading {url} failed ({}): {}",
                other.map_or("no answer".to_string(), |code| format!("HTTP {code}")),
                outcome.stderr.trim()
            ),
        }
    }

    /// A checkout at the commit in `<id>/source`, with the CLI built inside it.
    fn build_source(&mut self, pin: &Pin, id: &str) -> Result<Installed> {
        let url = pin
            .git_url()
            .ok_or_else(|| anyhow!("the pin {} has no git source", pin.label()))?;
        let reference = pin.git_ref().expect("a git source has a ref");
        let source = self.store.dir(id).join("source");
        if !self.dry_run() {
            std::fs::create_dir_all(&source)
                .with_context(|| format!("creating {}", source.display()))?;
        }
        let _ = writeln!(
            self.out,
            "==> nsh: building nsh {id} from {url} at {reference}"
        );
        let git = |args: &[&str]| {
            CommandSpec::new("git")
                .args(args.iter().copied())
                .cwd(&source)
        };
        if !source.join(".git").exists() {
            self.runner.run_checked(&git(&["init", "--quiet"]))?;
        }
        if self.host.os == Os::Windows {
            self.runner
                .run_checked(&git(&["config", "core.longPaths", "true"]))?;
        }
        let at_commit = pin.commit().is_some_and(|commit| {
            self.runner
                .query(&git(&["rev-parse", "HEAD"]))
                .ok()
                .filter(|o| o.is_success())
                .is_some_and(|o| o.stdout.trim() == commit)
        });
        if !at_commit {
            // GitHub serves a commit by its id; another host may only serve the ref.
            let by_commit = self.runner.run(&git(&[
                "fetch",
                "--depth",
                "1",
                url.as_str(),
                reference.as_str(),
            ]))?;
            if !by_commit.is_success() {
                let named = match &pin.source {
                    Source::Git { reference, .. } => reference.name().map(str::to_string),
                    _ => None,
                }
                .ok_or_else(|| {
                    anyhow!(
                        "{url} refused the commit {reference}: {}",
                        by_commit.stderr.trim()
                    )
                })?;
                self.runner.run_checked(&git(&[
                    "fetch",
                    "--depth",
                    "1",
                    url.as_str(),
                    named.as_str(),
                ]))?;
            }
            self.runner
                .run_checked(&git(&["checkout", "--quiet", "--detach", "FETCH_HEAD"]))?;
            if let Some(commit) = pin.commit()
                && !self.dry_run()
            {
                let head = self
                    .runner
                    .query(&git(&["rev-parse", "HEAD"]))
                    .map(|o| o.stdout.trim().to_string())
                    .unwrap_or_default();
                if head != commit {
                    bail!("{url} gave {head} for the pin, not its commit {commit}");
                }
            }
        }
        // NSH's own rust-toolchain.toml.
        self.runner.run_checked(
            &in_nsh_checkout(
                CommandSpec::new(self.tools.rustup.as_str()).args(["toolchain", "install"]),
            )
            .cwd(&source)
            .interactive(),
        )?;
        self.runner.run_checked(
            // `--target-dir target`, relative to the checkout, outranks a user's
            // `CARGO_TARGET_DIR` and `build.target-dir`: the CLI must land in the entry, where nsh
            // also builds its hosts (NSH's own xtask passes it for the same reason).
            &in_nsh_checkout(CommandSpec::new(self.tools.cargo.as_str()).args([
                "build",
                "--release",
                "--locked",
                "-p",
                "nsh-cli",
                "--target-dir",
                "target",
            ]))
            .cwd(&source)
            .interactive(),
        )?;
        if self.dry_run() {
            return Ok(self.dry_run_result(pin, id, Kind::Source, url));
        }
        let cli_rel = source_cli(self.host.os);
        self.finish(pin, id, Kind::Source, url, cli_rel)
    }

    /// Checks the CLI and writes the receipt.
    fn finish(
        &mut self,
        pin: &Pin,
        id: &str,
        kind: Kind,
        url: String,
        cli_rel: String,
    ) -> Result<Installed> {
        let cli = cli_rel
            .split('/')
            .fold(self.store.dir(id), |path, part| path.join(part));
        let found = check_cli(self.runner, &cli, pin)?;
        let receipt = Receipt {
            version: super::store::RECEIPT_VERSION,
            id: id.to_string(),
            kind,
            url,
            reference: match &pin.source {
                Source::Git { reference, .. } => reference.name().map(str::to_string),
                Source::Release { version } => Some(format!("v{version}")),
                Source::Path { .. } => None,
            },
            commit: pin.commit().map(str::to_string),
            nsh: found.version,
            cli: cli_rel,
            installed: rfc3339(std::time::SystemTime::now()),
        };
        self.store.complete(&receipt)?;
        let _ = writeln!(
            self.out,
            "Installed nsh {} ({}) in {}.",
            pin.label(),
            kind.as_str(),
            self.store.dir(id).display()
        );
        Ok(Installed {
            receipt,
            cli,
            already: false,
        })
    }

    /// What a dry run (`--check`) reports: the receipt an install would write.
    fn dry_run_result(&self, pin: &Pin, id: &str, kind: Kind, url: String) -> Installed {
        let cli_rel = match kind {
            Kind::Distribution => distribution_cli(self.host.os),
            Kind::Source => source_cli(self.host.os),
        };
        let receipt = Receipt {
            version: super::store::RECEIPT_VERSION,
            id: id.to_string(),
            kind,
            url,
            reference: None,
            commit: pin.commit().map(str::to_string),
            nsh: pin.version.clone().unwrap_or_default(),
            cli: cli_rel,
            installed: String::new(),
        };
        Installed {
            cli: self.store.cli(&receipt),
            receipt,
            already: false,
        }
    }
}

/// The digest of a `.sha256` file: its first word (`<hex>  <file>`), in lowercase.
pub fn read_digest(file: &Path) -> Result<String> {
    let text =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let digest = text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("{} holds no SHA-256 digest", file.display());
    }
    Ok(digest)
}

/// The SHA-256 of a file, in lowercase hex.
pub fn sha256_of(file: &Path) -> Result<String> {
    let mut reader =
        std::fs::File::open(file).with_context(|| format!("reading {}", file.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Unpacks the `nsh/` of an archive (`.tar.gz` or `.zip`) into `dir`, so it lands as
/// `dir/nsh/…`. Entries outside `nsh/`, absolute paths and `..` are refused.
pub fn unpack(archive: &Path, dir: &Path) -> Result<()> {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    std::fs::create_dir_all(dir)?;
    if name.ends_with(".zip") {
        let file = std::fs::File::open(archive)?;
        let mut zip = zip::ZipArchive::new(file)?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let Some(relative) = entry.enclosed_name() else {
                bail!("{name} holds an unsafe path {:?}", entry.name());
            };
            let target = inside_nsh(&relative, dir, name)?;
            if entry.is_dir() {
                std::fs::create_dir_all(&target)?;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut out)?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode))?;
            }
        }
        return Ok(());
    }
    let file = std::fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    tar.set_preserve_permissions(true);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let relative = entry.path()?.into_owned();
        inside_nsh(&relative, dir, name)?;
        if !entry.unpack_in(dir)? {
            bail!("{name} holds an unsafe path {}", relative.display());
        }
    }
    Ok(())
}

/// `dir/<relative>` when `relative` lies under `nsh/` and has no `..`.
fn inside_nsh(relative: &Path, dir: &Path, archive: &str) -> Result<PathBuf> {
    use std::path::Component;
    let mut components = relative.components();
    let first = components.next();
    let normal = relative
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !normal || first.is_none_or(|c| c.as_os_str() != "nsh") {
        bail!("{archive} holds {} outside nsh/", relative.display());
    }
    Ok(dir.join(relative))
}
