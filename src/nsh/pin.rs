//! The nsh a project pins (design §4.1, D10, D7).
//!
//! rayx reads the pin in the directory it runs nsh in, so the two never disagree about Cargo
//! config:
//!
//! 1. the NSH crates' source in the workspace's `Cargo.lock` (the workspace of the directory, else
//!    of each crate `nsh.json` lists under `"rust"`); every `nsh-*` package has one and the same
//!    source;
//! 2. without NSH crates, the `[nsh]` table of `rayx.toml` beside `nsh.json`;
//! 3. else the built-in default of `project::pins`.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::host::{CommandSpec, Mode, Runner};
use crate::project::pins::{DEFAULTS, DefaultNsh};

/// The app manifest nsh reads.
pub const NSH_JSON: &str = "nsh.json";

/// The file beside `nsh.json` that pins an app without Rust crates.
pub const RAYX_TOML: &str = "rayx.toml";

/// The crates.io index, the source of upstream's NSH releases.
pub const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// Upstream NSH, where a crates.io release's archives and tags live.
pub const UPSTREAM: &str = "https://github.com/longbridge/nsh";

/// How a git source names its commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    Tag(String),
    Branch(String),
    Rev(String),
    /// The repository's default branch.
    Default,
}

impl Reference {
    /// The tag, when it is one.
    pub fn tag(&self) -> Option<&str> {
        match self {
            Reference::Tag(tag) => Some(tag),
            _ => None,
        }
    }

    /// The ref as `rev` or `ref` of a receipt: the tag, branch or rev name; `None` for the default
    /// branch.
    pub fn name(&self) -> Option<&str> {
        match self {
            Reference::Tag(name) | Reference::Branch(name) | Reference::Rev(name) => Some(name),
            Reference::Default => None,
        }
    }
}

/// Where the pinned NSH comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A git repository at a commit: a fork tag, a branch, a rev.
    Git {
        url: String,
        reference: Reference,
        /// The full commit.
        commit: String,
    },
    /// An upstream release on crates.io.
    Release { version: String },
    /// An NSH checkout on this machine (a path dependency).
    Path { checkout: PathBuf },
}

/// Where rayx read the pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The NSH crates' source in the project's `Cargo.lock`.
    Project,
    /// The `[nsh]` table of the app's `rayx.toml`.
    RayxToml,
    /// The built-in default.
    Default,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Project => "project",
            Origin::RayxToml => "rayx.toml",
            Origin::Default => "default",
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A release whose archives rayx can download: `https://github.com/<repository>/releases/download/<tag>/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseSource {
    /// `owner/name` on GitHub.
    pub repository: String,
    pub tag: String,
}

/// The resolved pin of a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub source: Source,
    pub origin: Origin,
    /// The NSH crates' version (`0.2.0`), which the CLI's `--version` must name; `None` when the
    /// pin does not say (a git pin of `rayx.toml`).
    pub version: Option<String>,
    /// The directory the pin was read in: where nsh runs.
    pub dir: PathBuf,
    /// Whether the directory is an app (it has `nsh.json`).
    pub app: bool,
}

impl Pin {
    /// The store id: the commit's first 12 hex digits, `v<version>` for an upstream release, none
    /// for a path checkout.
    pub fn id(&self) -> Option<String> {
        match &self.source {
            Source::Git { commit, .. } => Some(commit.chars().take(12).collect()),
            Source::Release { version } => Some(format!("v{version}")),
            Source::Path { .. } => None,
        }
    }

    /// The full commit, when the pin names one.
    pub fn commit(&self) -> Option<&str> {
        match &self.source {
            Source::Git { commit, .. } => Some(commit),
            _ => None,
        }
    }

    /// The pin as people read it: `fork-0.2.0-4 (327bb9e)`, `main (5f0c2a9)`, `v0.2.0`, or the
    /// checkout's path.
    pub fn label(&self) -> String {
        match &self.source {
            Source::Git {
                reference, commit, ..
            } => {
                let short: String = commit.chars().take(7).collect();
                match reference.name() {
                    Some(name) if name.starts_with(&short) => short,
                    Some(name) => format!("{name} ({short})"),
                    None => short,
                }
            }
            Source::Release { version } => format!("v{version}"),
            Source::Path { checkout } => checkout.display().to_string(),
        }
    }

    /// The GitHub release that may hold an archive of this pin: a git tag on github.com, or the
    /// upstream release of a crates.io version. A branch, a rev and a path have none.
    pub fn release(&self) -> Option<ReleaseSource> {
        match &self.source {
            Source::Git {
                url,
                reference: Reference::Tag(tag),
                ..
            } => github_repository(url).map(|repository| ReleaseSource {
                repository,
                tag: tag.clone(),
            }),
            Source::Release { version } => Some(ReleaseSource {
                repository: github_repository(UPSTREAM).expect("upstream is on GitHub"),
                tag: format!("v{version}"),
            }),
            _ => None,
        }
    }

    /// The git URL a source install fetches: the pin's repository, or upstream for a release.
    pub fn git_url(&self) -> Option<String> {
        match &self.source {
            Source::Git { url, .. } => Some(url.clone()),
            Source::Release { .. } => Some(UPSTREAM.to_string()),
            Source::Path { .. } => None,
        }
    }

    /// What a source install checks out: the commit, or the upstream release's tag.
    pub fn git_ref(&self) -> Option<String> {
        match &self.source {
            Source::Git { commit, .. } => Some(commit.clone()),
            Source::Release { version } => Some(format!("v{version}")),
            Source::Path { .. } => None,
        }
    }

    /// The warning every command prints when an app has no pin of its own: the default applies.
    pub fn default_warning(&self) -> Option<String> {
        (self.origin == Origin::Default && self.app).then(|| {
            format!(
                "rayx: {} pins no nsh, so rayx's default {} applies; pin it in {RAYX_TOML} beside {NSH_JSON}:\n{}",
                self.dir.display(),
                self.label(),
                rayx_toml_table(&DEFAULTS.nsh)
            )
        })
    }
}

/// `owner/name` of a GitHub URL (`https://github.com/dXnSix/nsh`, with or without `.git`).
pub fn github_repository(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    (parts.next().is_none() && !owner.is_empty() && !name.is_empty())
        .then(|| format!("{owner}/{name}"))
}

/// The `[nsh]` table that pins `default` in `rayx.toml`.
pub fn rayx_toml_table(default: &DefaultNsh) -> String {
    format!(
        "[nsh]\ngit = \"{}\"\ntag = \"{}\"\ncommit = \"{}\"\n",
        default.git, default.tag, default.commit
    )
}

/// `text` with its `%XX` escapes decoded; an escape that is not two hex digits stays as written.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escape {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_string())
}

/// A pin that cannot be read in `dir`, with the reason (two sources, a broken `rayx.toml`, no
/// `Cargo.lock` yet).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreadable {
    pub dir: PathBuf,
    pub reason: String,
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

/// [`resolve`], its error kept as an [`Unreadable`] with the directory it was read in.
pub fn resolve_in(runner: &mut Runner, dir: &Path) -> Result<Pin, Unreadable> {
    resolve(runner, dir).map_err(|error| Unreadable {
        dir: dir.to_path_buf(),
        reason: format!("{error:#}"),
    })
}

/// Whether a lock package is one of NSH's crates: the defaults table's list (its `crates/nsh-*`)
/// and `nsh-platform-gpui`, from NSH's separate `crates/gpui` workspace, as nsh lists them
/// (`NSH_CRATES`). An app's own crates (`nsh-app-*`, from `nsh init`) are the app's, not NSH's.
pub fn is_nsh_crate(name: &str) -> bool {
    DEFAULTS.nsh.crates.contains(&name) || name == "nsh-platform-gpui"
}

/// Reads a `Cargo.lock` source: `git+<url>?<kind>=<name>#<commit>`, `registry+…` at `version`.
pub fn classify(source: &str, version: &str) -> Result<Source> {
    if source == CRATES_IO {
        return Ok(Source::Release {
            version: version.to_string(),
        });
    }
    let Some(git) = source.strip_prefix("git+") else {
        bail!("NSH comes from {source}, which rayx cannot install: only git and crates.io sources");
    };
    let (location, commit) = git
        .split_once('#')
        .ok_or_else(|| anyhow!("the lock's NSH source {source} names no commit"))?;
    if commit.len() < 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("the lock's NSH source {source} names no full commit");
    }
    let (url, query) = location.split_once('?').unwrap_or((location, ""));
    // Cargo's version 4 lock percent-encodes the ref (`feat/x` is `feat%2Fx`).
    let reference = match query.split_once('=') {
        Some(("tag", name)) => Reference::Tag(percent_decoded(name)),
        Some(("branch", name)) => Reference::Branch(percent_decoded(name)),
        Some(("rev", name)) => Reference::Rev(percent_decoded(name)),
        _ => Reference::Default,
    };
    Ok(Source::Git {
        url: url.to_string(),
        reference,
        commit: commit.to_ascii_lowercase(),
    })
}

/// The one source and version every NSH package ([`is_nsh_crate`]) of a `Cargo.lock` has; `None`
/// without NSH packages. A package without a `source` is a path dependency
/// (`Ok(Some((None, version)))`).
pub fn lock_source(lock: &str, path: &Path) -> Result<Option<(Option<String>, String)>> {
    let table: toml::Table = lock
        .parse()
        .with_context(|| format!("reading {}", path.display()))?;
    let mut found: Option<(Option<String>, String, String)> = None;
    for package in table
        .get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
            continue;
        };
        if !is_nsh_crate(name) {
            continue;
        }
        let source = package
            .get("source")
            .and_then(toml::Value::as_str)
            .map(str::to_string);
        let version = package
            .get("version")
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_string();
        match &found {
            None => found = Some((source, version, name.to_string())),
            Some((known, _, first)) if *known != source => bail!(
                "{} locks NSH from two sources: {first} from {} and {name} from {}; use one",
                path.display(),
                describe(known.as_deref()),
                describe(source.as_deref())
            ),
            Some(_) => {}
        }
    }
    Ok(found.map(|(source, version, _)| (source, version)))
}

fn describe(source: Option<&str>) -> &str {
    source.unwrap_or("a path")
}

/// The pin of the `[nsh]` table of `rayx.toml`, if the file has one.
pub fn rayx_toml_pin(text: &str, path: &Path) -> Result<Option<(Source, Option<String>)>> {
    let table: toml::Table = text
        .parse()
        .with_context(|| format!("reading {}", path.display()))?;
    let Some(nsh) = table.get("nsh").and_then(toml::Value::as_table) else {
        return Ok(None);
    };
    let field = |name: &str| nsh.get(name).and_then(toml::Value::as_str);
    if let Some(version) = field("version") {
        return Ok(Some((
            Source::Release {
                version: version.to_string(),
            },
            Some(version.to_string()),
        )));
    }
    let (Some(git), Some(commit)) = (field("git"), field("commit")) else {
        bail!(
            "{} has an [nsh] table without `git` and `commit` (or `version`)",
            path.display()
        );
    };
    if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!(
            "{}: `commit` must be a full commit, not {commit}",
            path.display()
        );
    }
    let reference = match (field("tag"), field("branch"), field("rev")) {
        (Some(tag), _, _) => Reference::Tag(tag.to_string()),
        (_, Some(branch), _) => Reference::Branch(branch.to_string()),
        (_, _, Some(rev)) => Reference::Rev(rev.to_string()),
        _ => Reference::Default,
    };
    Ok(Some((
        Source::Git {
            url: git.to_string(),
            reference,
            commit: commit.to_ascii_lowercase(),
        },
        None,
    )))
}

/// The pin of `dir` (made absolute), reading the project through `runner` (`cargo
/// locate-project`, and `cargo metadata` to find a path checkout). A missing `Cargo.lock` is
/// written by `cargo metadata` only when `runner` executes: a dry run (`--check`, `doctor`,
/// `rayx nsh status`) writes nothing and reports the pin as unknown until the lock exists.
pub fn resolve(runner: &mut Runner, dir: &Path) -> Result<Pin> {
    resolve_with(runner, dir, "cargo")
}

/// [`resolve`] running `cargo` as the program `cargo` names (`~/.cargo/bin/cargo` in a setup run
/// that has just installed Rust).
pub fn resolve_with(runner: &mut Runner, dir: &Path, cargo: &str) -> Result<Pin> {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let app = dir.join(NSH_JSON).is_file();
    let locked = locked_source(runner, &dir, cargo)?;
    let toml_path = dir.join(RAYX_TOML);
    let from_toml = match std::fs::read_to_string(&toml_path) {
        Ok(text) => rayx_toml_pin(&text, &toml_path)?,
        Err(_) => None,
    };
    let (source, origin, version) = match (locked, from_toml) {
        (Some((lock, _, _)), Some(_)) => bail!(
            "{} pins nsh twice: its NSH crates in {} and the [nsh] table of {}; the lock's pin is the one, so remove the [nsh] table",
            dir.display(),
            lock.display(),
            toml_path.display()
        ),
        (Some((_, source, version)), None) => (source, Origin::Project, Some(version)),
        (None, Some((source, version))) => (source, Origin::RayxToml, version),
        (None, None) => (
            default_source(&DEFAULTS.nsh),
            Origin::Default,
            Some(DEFAULTS.nsh.version.to_string()),
        ),
    };
    Ok(Pin {
        source,
        origin,
        version,
        dir,
        app,
    })
}

/// The source of the built-in default pin.
pub fn default_source(default: &DefaultNsh) -> Source {
    Source::Git {
        url: default.git.to_string(),
        reference: Reference::Tag(default.tag.to_string()),
        commit: default.commit.to_string(),
    }
}

/// The built-in default pin, read in `dir`.
pub fn default_pin(dir: &Path) -> Pin {
    Pin {
        source: default_source(&DEFAULTS.nsh),
        origin: Origin::Default,
        version: Some(DEFAULTS.nsh.version.to_string()),
        dir: dir.to_path_buf(),
        app: dir.join(NSH_JSON).is_file(),
    }
}

/// The NSH source the project locks: `(lock path, source, version)`, or `None` without NSH crates.
fn locked_source(
    runner: &mut Runner,
    dir: &Path,
    program: &str,
) -> Result<Option<(PathBuf, Source, String)>> {
    let roots = match workspace_root(runner, dir) {
        Some(root) => vec![root],
        None => manifest_roots(runner, dir)?,
    };
    let mut found: Option<(PathBuf, Source, String)> = None;
    for root in roots {
        let lock = root.join("Cargo.lock");
        if !lock.is_file() {
            if runner.mode() != Mode::Execute {
                bail!(
                    "{} has no Cargo.lock yet, so the nsh it pins is unknown: `rayx setup` writes the lock (with `cargo metadata`), as any Cargo build does",
                    root.display()
                );
            }
            // `cargo metadata` resolves and writes the lock.
            let _ = runner.query(&cargo(
                program,
                &root,
                &["metadata", "--format-version", "1"],
            ));
        }
        // A workspace whose lock cannot be had says nothing about NSH: its pin is unknown, never
        // the default.
        let text = std::fs::read_to_string(&lock).with_context(|| {
            format!(
                "{} has no Cargo.lock, and cargo could not write one (is Rust installed, with the project's toolchain?), so the nsh it pins is unknown",
                root.display()
            )
        })?;
        let Some((source, version)) = lock_source(&text, &lock)? else {
            continue;
        };
        let source = match source {
            Some(source) => classify(&source, &version)?,
            None => Source::Path {
                checkout: path_checkout(runner, &root, program)?,
            },
        };
        match &found {
            Some((first, known, _)) if *known != source => bail!(
                "{} and {} lock NSH from different sources; use one",
                first.display(),
                lock.display()
            ),
            Some(_) => {}
            None => found = Some((lock, source, version)),
        }
    }
    Ok(found)
}

fn cargo(program: &str, dir: &Path, args: &[&str]) -> CommandSpec {
    CommandSpec::new(program)
        .args(args.iter().copied())
        .env("RUSTUP_AUTO_INSTALL", "0")
        .cwd(dir)
}

/// The Cargo workspace around `dir`, if there is one: `cargo locate-project`, else the manifests
/// themselves, as project discovery reads them when cargo cannot run (a fresh machine, or the
/// project's toolchain not installed yet).
fn workspace_root(runner: &mut Runner, dir: &Path) -> Option<PathBuf> {
    crate::project::Project::discover(runner, dir, None)
        .ok()
        .map(|project| project.workspace_root)
}

/// The workspaces of the crates `nsh.json` lists under `"rust"`.
fn manifest_roots(runner: &mut Runner, dir: &Path) -> Result<Vec<PathBuf>> {
    let path = dir.join(NSH_JSON);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let manifest: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
    let mut roots: Vec<PathBuf> = Vec::new();
    for crate_dir in manifest["rust"].as_array().into_iter().flatten() {
        let Some(crate_dir) = crate_dir.as_str() else {
            continue;
        };
        if let Some(root) = workspace_root(runner, &dir.join(crate_dir))
            && !roots.contains(&root)
        {
            roots.push(root);
        }
    }
    Ok(roots)
}

/// The NSH checkout a path dependency points at: the directory above an NSH package
/// ([`is_nsh_crate`]) that holds `crates/nsh-cli/Cargo.toml`.
fn path_checkout(runner: &mut Runner, root: &Path, program: &str) -> Result<PathBuf> {
    let metadata = runner
        .query(&cargo(
            program,
            root,
            &["metadata", "--format-version", "1"],
        ))
        .ok()
        .filter(|outcome| outcome.is_success())
        .ok_or_else(|| anyhow!("`cargo metadata` failed in {}", root.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&metadata.stdout).context("reading `cargo metadata`")?;
    for package in json["packages"].as_array().into_iter().flatten() {
        let name = package["name"].as_str().unwrap_or_default();
        if !is_nsh_crate(name) || !package["source"].is_null() {
            continue;
        }
        let manifest = PathBuf::from(package["manifest_path"].as_str().unwrap_or_default());
        if let Some(checkout) = manifest.ancestors().find(|dir| {
            dir.join("crates")
                .join("nsh-cli")
                .join("Cargo.toml")
                .is_file()
        }) {
            return Ok(checkout.to_path_buf());
        }
    }
    bail!(
        "{} depends on NSH by path, but no NSH checkout (with crates/nsh-cli) holds those crates",
        root.display()
    )
}

/// Checks that the tag of a `rayx.toml` pin still names its recorded commit, with `git ls-remote`
/// (Cargo checks a lock's tags itself, so other pins ask nothing).
pub fn verify_tag(runner: &mut Runner, pin: &Pin) -> Result<()> {
    let Source::Git {
        url,
        reference: Reference::Tag(tag),
        commit,
    } = &pin.source
    else {
        return Ok(());
    };
    if pin.origin != Origin::RayxToml {
        return Ok(());
    }
    let refs = format!("refs/tags/{tag}");
    let peeled = format!("{refs}^{{}}");
    let spec = CommandSpec::new("git").args(["ls-remote", url.as_str(), &refs, &peeled]);
    let outcome = runner
        .query(&spec)
        .with_context(|| format!("asking {url} for the tag {tag}"))?;
    if !outcome.is_success() {
        bail!(
            "asking {url} for the tag {tag} failed: {}",
            outcome.stderr.trim()
        );
    }
    let mut found: Option<&str> = None;
    for line in outcome.stdout.lines() {
        let Some((sha, name)) = line.split_once('\t') else {
            continue;
        };
        if name == peeled {
            found = Some(sha);
            break;
        }
        if name == refs {
            found = Some(sha);
        }
    }
    let found = found.ok_or_else(|| anyhow!("{url} has no tag {tag}"))?;
    if !found.eq_ignore_ascii_case(commit) {
        bail!(
            "the tag {tag} of {url} is at {}, but {} records {}",
            &found[..12.min(found.len())],
            pin.dir.join(RAYX_TOML).display(),
            &commit[..12.min(commit.len())]
        );
    }
    Ok(())
}
