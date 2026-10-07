//! What the app pipeline knows about the project it works on: the Cargo workspace around the
//! selected app, the resolved version pins and the host. Nothing here depends on where `rayx` was
//! built or on the RayX repository layout.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::host::{CommandSpec, HostFacts, Runner};
use crate::project::{Dependency, Package, Pins, Project};

/// The workspace, pins and host an app command runs against.
#[derive(Clone, Debug)]
pub struct ProjectContext {
    pub project: Project,
    pub pins: Pins,
    pub host: HostFacts,
}

impl ProjectContext {
    /// A context for an already discovered project.
    pub fn new(project: Project, host: HostFacts) -> Self {
        let pins = Pins::resolve(Some(&project));
        Self {
            project,
            pins,
            host,
        }
    }

    /// Discovers the project of `app_dir` (relative to `cwd`), or of `cwd` itself.
    pub fn discover(cwd: &Path, app_dir: Option<&Path>) -> Result<Self> {
        // Discovery only reads, so it runs for real whatever mode the app command is in.
        let project = Project::discover(&mut Runner::print(), cwd, app_dir)
            .map_err(|error| anyhow!("{error}"))?;
        Ok(Self::new(project, crate::host::facts()))
    }

    /// The Cargo workspace root: `artifacts/`, `artifacts-temp/` and `target/` live under it.
    pub fn workspace_root(&self) -> &Path {
        &self.project.workspace_root
    }

    /// The workspace member whose manifest directory is `dir`.
    pub fn package_in(&self, dir: &Path) -> Option<&Package> {
        let wanted = comparable(dir);
        self.project
            .packages
            .iter()
            .find(|package| comparable(package.dir()) == wanted)
    }

    /// The path a `[patch.<registry>] <name> = { path = ... }` entry of the workspace root
    /// manifest points at, made absolute.
    pub fn root_patch_path(&self, registry: &str, name: &str) -> Result<Option<PathBuf>> {
        let manifest_path = self.workspace_root().join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let manifest: toml::Table = toml::from_str(&text)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
        Ok(manifest
            .get("patch")
            .and_then(|patch| patch.get(registry))
            .and_then(|registry| registry.get(name))
            .and_then(|entry| entry.get("path"))
            .and_then(|path| path.as_str())
            .map(|path| self.workspace_root().join(path)))
    }

    /// The manifest directory of a package in the app's resolved dependency graph, from
    /// `cargo metadata` of the app (for example the `gpux-fonts` package).
    pub fn resolved_package_dir(&self, package: &str, app_manifest: &Path) -> Result<PathBuf> {
        let outcome = Runner::print()
            .query(
                &CommandSpec::new("cargo")
                    .args(["metadata", "--format-version", "1", "--manifest-path"])
                    .arg(app_manifest.display().to_string())
                    .env("RUSTUP_AUTO_INSTALL", "0"),
            )
            .map_err(|error| anyhow!("{error}"))?;
        if !outcome.is_success() {
            bail!(
                "cargo metadata failed for {}: {}",
                app_manifest.display(),
                outcome.stderr.trim()
            );
        }
        let json: serde_json::Value =
            serde_json::from_str(&outcome.stdout).context("unexpected `cargo metadata` output")?;
        json["packages"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|candidate| candidate["name"] == package)
            .and_then(|candidate| candidate["manifest_path"].as_str())
            .and_then(|manifest| Path::new(manifest).parent().map(Path::to_path_buf))
            .ok_or_else(|| {
                anyhow!(
                    "the dependency graph of {} has no `{package}` package",
                    app_manifest.display()
                )
            })
    }
}

/// A path in a form that two spellings of one directory share.
fn comparable(path: &Path) -> String {
    let text = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_string();
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

/// The inline `Cargo.toml` table that depends on `dependency` from the same source the app does:
/// its absolute path, its git repository and revision, or its registry version.
pub fn dependency_toml(dependency: &Dependency, optional: bool) -> Result<String> {
    let mut fields: Vec<String> = Vec::new();
    match (&dependency.path, dependency.source.as_deref()) {
        (Some(path), _) => fields.push(format!("path = \"{}\"", cargo_path(path))),
        (None, Some(source)) if source.starts_with("git+") => {
            let rest = source.trim_start_matches("git+");
            let rest = rest.split('#').next().unwrap_or(rest);
            let (url, query) = rest.split_once('?').unwrap_or((rest, ""));
            fields.push(format!("git = \"{url}\""));
            if let Some((kind, value)) = query.split_once('=')
                && matches!(kind, "rev" | "branch" | "tag")
            {
                fields.push(format!("{kind} = \"{value}\""));
            }
        }
        (None, Some(source)) if source.contains("crates.io") => {
            fields.push(format!("version = \"{}\"", dependency.req));
        }
        (None, Some(source)) => bail!(
            "the dependency `{}` comes from {source}, which generated entry crates cannot copy",
            dependency.name
        ),
        (None, None) => bail!("the dependency `{}` has no source", dependency.name),
    }
    if optional {
        fields.push("optional = true".to_string());
    }
    Ok(format!("{{ {} }}", fields.join(", ")))
}

/// A path as written in a generated `Cargo.toml`: no verbatim prefix, forward slashes.
pub fn cargo_path(path: &Path) -> String {
    crate::app::fs_util::command_path(path)
        .to_string_lossy()
        .replace('\\', "/")
}
