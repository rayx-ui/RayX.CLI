//! Finding the project `rayx` works on, from the paths the user gives and the Cargo workspace
//! around them, never from where `rayx` was built.

pub mod pins;

use std::fmt;
use std::path::{Component, Path, PathBuf};

use crate::host::{CommandSpec, RunError, Runner};

pub use pins::{Pin, PinSource, Pins};

/// A dependency a package declares, as `cargo metadata` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    /// The package name (not a rename).
    pub name: String,
    /// The version requirement, `*` when there is none.
    pub req: String,
    /// `registry+<url>` or `git+<url>[?rev=|branch=|tag=...]`; `None` for a path dependency.
    pub source: Option<String>,
    /// The absolute directory of a path dependency.
    pub path: Option<PathBuf>,
    pub optional: bool,
}

/// A Cargo workspace package.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub name: String,
    pub manifest_path: PathBuf,
    /// The package's `[package.metadata.rayx]` table.
    pub rayx_metadata: Option<serde_json::Value>,
    /// The dependencies the package declares (all kinds).
    pub dependencies: Vec<Dependency>,
}

impl Package {
    /// The directory holding the package's manifest.
    pub fn dir(&self) -> &Path {
        self.manifest_path.parent().unwrap_or(Path::new("."))
    }

    /// The first non-development dependency with this package name.
    pub fn dependency(&self, name: &str) -> Option<&Dependency> {
        self.dependencies
            .iter()
            .find(|dependency| dependency.name == name)
    }
}

/// The workspace around the selected directory.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    /// The directory the user selected: the app directory, or the current directory.
    pub app_dir: PathBuf,
    /// The workspace root directory, from `cargo locate-project --workspace`.
    pub workspace_root: PathBuf,
    /// The workspace members, from `cargo metadata`.
    pub packages: Vec<Package>,
    /// The workspace's `[workspace.metadata.rayx]` table.
    pub rayx_metadata: Option<serde_json::Value>,
    /// How the workspace was found.
    pub discovery: Discovery,
}

/// How a [`Project`] was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discovery {
    /// `cargo locate-project` and `cargo metadata`.
    Cargo,
    /// The manifests themselves, because cargo could not run (not installed yet, or the
    /// project's pinned toolchain is missing).
    Files,
}

/// Why no project was found.
#[derive(Debug)]
pub enum ProjectError {
    /// `cargo` could not be run.
    Cargo(RunError),
    /// The selected directory does not exist.
    MissingDirectory(PathBuf),
    /// The directory is not inside a Cargo workspace.
    NotAProject { dir: PathBuf, detail: String },
    /// `cargo metadata` produced output `rayx` does not understand.
    BadMetadata(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::Cargo(error) => write!(f, "{error}"),
            ProjectError::MissingDirectory(dir) => {
                write!(f, "the directory {} does not exist", dir.display())
            }
            ProjectError::NotAProject { dir, detail } => write!(
                f,
                "{} is not inside a Cargo workspace{}",
                dir.display(),
                if detail.trim().is_empty() {
                    String::new()
                } else {
                    format!(": {}", detail.trim())
                }
            ),
            ProjectError::BadMetadata(detail) => {
                write!(f, "unexpected `cargo metadata` output: {detail}")
            }
        }
    }
}

impl std::error::Error for ProjectError {}

impl Project {
    /// Finds the project for `app` (relative paths are taken from `cwd`), or for `cwd` itself.
    /// Running the same command from another directory with a relative or an absolute path
    /// selects the same app.
    pub fn discover(
        runner: &mut Runner,
        cwd: &Path,
        app: Option<&Path>,
    ) -> Result<Project, ProjectError> {
        let cwd = absolute(cwd);
        let app_dir = match app {
            Some(app) => absolute(&cwd.join(app)),
            None => cwd,
        };
        if !app_dir.is_dir() {
            return Err(ProjectError::MissingDirectory(app_dir));
        }
        match Self::discover_with_cargo(runner, &app_dir) {
            Ok(project) => Ok(project),
            // A fresh machine has no cargo yet, and a project may pin a toolchain that is not
            // installed; its files still say what to install.
            Err(error) => Self::discover_from_files(&app_dir).ok_or(error),
        }
    }

    /// `cargo locate-project` and `cargo metadata`. Rustup must not install a missing pinned
    /// toolchain on the way: a probe has no side effects.
    fn discover_with_cargo(runner: &mut Runner, app_dir: &Path) -> Result<Project, ProjectError> {
        let cargo = |args: &[&str]| {
            CommandSpec::new("cargo")
                .args(args.iter().copied())
                .env("RUSTUP_AUTO_INSTALL", "0")
                .cwd(app_dir)
        };
        let located = runner
            .query(&cargo(&[
                "locate-project",
                "--workspace",
                "--message-format",
                "plain",
            ]))
            .map_err(ProjectError::Cargo)?;
        if !located.is_success() {
            return Err(ProjectError::NotAProject {
                dir: app_dir.to_path_buf(),
                detail: located.stderr,
            });
        }
        let manifest = PathBuf::from(located.stdout.trim());
        let workspace_root = manifest
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| ProjectError::BadMetadata(format!("manifest path {manifest:?}")))?;

        let metadata = runner
            .query(&cargo(&["metadata", "--format-version", "1", "--no-deps"]))
            .map_err(ProjectError::Cargo)?;
        if !metadata.is_success() {
            return Err(ProjectError::NotAProject {
                dir: app_dir.to_path_buf(),
                detail: metadata.stderr,
            });
        }
        let json: serde_json::Value = serde_json::from_str(&metadata.stdout)
            .map_err(|error| ProjectError::BadMetadata(error.to_string()))?;
        let packages = json["packages"]
            .as_array()
            .ok_or_else(|| ProjectError::BadMetadata("no `packages` array".into()))?
            .iter()
            .filter_map(|package| {
                Some(Package {
                    name: package["name"].as_str()?.to_string(),
                    manifest_path: PathBuf::from(package["manifest_path"].as_str()?),
                    rayx_metadata: package["metadata"]
                        .get("rayx")
                        .filter(|value| !value.is_null())
                        .cloned(),
                    dependencies: package["dependencies"]
                        .as_array()
                        .map(|items| {
                            items
                                .iter()
                                .filter(|dep| dep["kind"] != "dev" && dep["kind"] != "build")
                                .filter_map(|dep| {
                                    Some(Dependency {
                                        name: dep["name"].as_str()?.to_string(),
                                        req: dep["req"].as_str().unwrap_or("*").to_string(),
                                        source: dep["source"].as_str().map(str::to_string),
                                        path: dep["path"].as_str().map(PathBuf::from),
                                        optional: dep["optional"].as_bool().unwrap_or(false),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            })
            .collect();
        let rayx_metadata = json["metadata"]
            .get("rayx")
            .filter(|value| !value.is_null())
            .cloned();

        Ok(Project {
            app_dir: app_dir.to_path_buf(),
            workspace_root,
            packages,
            rayx_metadata,
            discovery: Discovery::Cargo,
        })
    }

    /// Reads the workspace from its manifests when cargo cannot run: the nearest enclosing
    /// `[workspace]`, else the nearest package, with its members and `metadata.rayx` tables.
    fn discover_from_files(app_dir: &Path) -> Option<Project> {
        let mut nearest: Option<(PathBuf, toml::Table)> = None;
        let mut root: Option<(PathBuf, toml::Table)> = None;
        for dir in app_dir.ancestors() {
            let Some(table) = read_manifest(&dir.join("Cargo.toml")) else {
                continue;
            };
            if table.contains_key("workspace") {
                root = Some((dir.to_path_buf(), table));
                break;
            }
            nearest.get_or_insert((dir.to_path_buf(), table));
        }
        let (workspace_root, manifest) = root.or(nearest)?;

        let mut manifests = vec![(workspace_root.clone(), manifest.clone())];
        let members = manifest
            .get("workspace")
            .and_then(|workspace| workspace.get("members"))
            .and_then(toml::Value::as_array)
            .map(|members| {
                members
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for member in members {
            for dir in expand_member(&workspace_root, member) {
                if let Some(table) = read_manifest(&dir.join("Cargo.toml")) {
                    manifests.push((dir, table));
                }
            }
        }
        let packages = manifests
            .iter()
            .filter_map(|(dir, table)| {
                let package = table.get("package")?;
                Some(Package {
                    name: package.get("name")?.as_str()?.to_string(),
                    manifest_path: dir.join("Cargo.toml"),
                    rayx_metadata: package
                        .get("metadata")
                        .and_then(|metadata| metadata.get("rayx"))
                        .and_then(|rayx| serde_json::to_value(rayx).ok()),
                    dependencies: manifest_dependencies(table, dir),
                })
            })
            .collect();
        let rayx_metadata = manifest
            .get("workspace")
            .and_then(|workspace| workspace.get("metadata"))
            .and_then(|metadata| metadata.get("rayx"))
            .and_then(|rayx| serde_json::to_value(rayx).ok());

        Some(Project {
            app_dir: app_dir.to_path_buf(),
            workspace_root,
            packages,
            rayx_metadata,
            discovery: Discovery::Files,
        })
    }

    /// The workspace member with this package name.
    pub fn package(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|package| package.name == name)
    }

    /// Whether this is a gpux checkout: a workspace that contains the `gpux-testkit` package.
    pub fn is_gpux_checkout(&self) -> bool {
        self.package("gpux-testkit").is_some()
    }
}

/// The `[dependencies]` of a manifest, for when cargo cannot report them. Inherited
/// (`workspace = true`) dependencies are not resolved.
fn manifest_dependencies(table: &toml::Table, dir: &Path) -> Vec<Dependency> {
    let Some(dependencies) = table.get("dependencies").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    dependencies
        .iter()
        .map(|(key, value)| {
            let table = value.as_table();
            let name = table
                .and_then(|t| t.get("package"))
                .and_then(toml::Value::as_str)
                .unwrap_or(key)
                .to_string();
            let string = |field: &str| {
                table
                    .and_then(|t| t.get(field))
                    .and_then(toml::Value::as_str)
            };
            let path = string("path").map(|path| absolute(&dir.join(path)));
            let source = if path.is_some() {
                None
            } else if let Some(git) = string("git") {
                let query = ["rev", "branch", "tag"]
                    .iter()
                    .find_map(|kind| string(kind).map(|value| format!("?{kind}={value}")))
                    .unwrap_or_default();
                Some(format!("git+{git}{query}"))
            } else {
                Some("registry+https://github.com/rust-lang/crates.io-index".to_string())
            };
            let req = value
                .as_str()
                .or_else(|| string("version"))
                .unwrap_or("*")
                .to_string();
            Dependency {
                name,
                req,
                source,
                path,
                optional: table
                    .and_then(|t| t.get("optional"))
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(false),
            }
        })
        .collect()
}

fn read_manifest(path: &Path) -> Option<toml::Table> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}

/// The directories a `[workspace] members` entry names; a trailing `/*` lists a directory.
fn expand_member(root: &Path, member: &str) -> Vec<PathBuf> {
    match member.strip_suffix("/*") {
        Some(parent) => {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join(parent))
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|entry| entry.path())
                        .filter(|path| path.join("Cargo.toml").is_file())
                        .collect()
                })
                .unwrap_or_default();
            dirs.sort();
            dirs
        }
        None if member.contains(['*', '?', '[']) => Vec::new(),
        None => vec![root.join(member)],
    }
}

/// Makes `path` absolute and removes `.` and `..` components without touching the file system,
/// so two spellings of one directory compare equal.
fn absolute(path: &Path) -> PathBuf {
    let base = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut normalized = PathBuf::new();
    for component in base.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
