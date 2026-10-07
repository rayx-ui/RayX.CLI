//! Helpers for the unit tests of the app pipeline: a copy of the fixture workspace in a temporary
//! directory (so generated output never lands in the repository) and descriptor resolution through
//! real project discovery.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::app::{AppDescriptor, ProjectContext};

/// Resolves the app in `app_dir` (a path string, as the CLI receives it) through real discovery.
pub fn resolve_str(app_dir: &str) -> Result<AppDescriptor> {
    resolve(Path::new(app_dir))
}

/// Resolves the app in `app_dir` through real discovery.
pub fn resolve(app_dir: &Path) -> Result<AppDescriptor> {
    let context = ProjectContext::discover(app_dir, None)?;
    AppDescriptor::resolve_from(&context, app_dir, ".")
}

/// A temporary copy of `tests/fixtures/app-workspace`.
pub struct FixtureWorkspace {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
}

impl FixtureWorkspace {
    pub fn new() -> Self {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("app-workspace");
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().to_path_buf();
        copy_tree(&source, &root).expect("copy the fixture workspace");
        Self { _dir: dir, root }
    }

    /// The fixture app `apps/demo`.
    pub fn app(&self) -> AppDescriptor {
        resolve(&self.root.join("apps").join("demo")).expect("the fixture app resolves")
    }
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
