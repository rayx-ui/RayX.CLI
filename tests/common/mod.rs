//! Helpers shared by the app-pipeline test groups: a temporary copy of a fixture workspace, so
//! generated output never lands in the repository.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use rayx_cli::app::{AppDescriptor, ProjectContext};

/// A temporary copy of `tests/fixtures/<name>`.
pub struct Workspace {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
}

impl Workspace {
    pub fn fixture(name: &str) -> Self {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        let dir = tempfile::tempdir().expect("temp dir");
        let root = fs::canonicalize(dir.path()).expect("canonical temp dir");
        copy_tree(&source, &root).expect("copy the fixture");
        Self { _dir: dir, root }
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    pub fn write(&self, relative: &str, text: &str) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
        fs::write(path, text).expect("write file");
    }

    pub fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path(relative)).expect("read file")
    }

    /// Appends `text` to a file of the workspace.
    pub fn append(&self, relative: &str, text: &str) {
        let mut contents = self.read(relative);
        contents.push_str(text);
        self.write(relative, &contents);
    }

    /// Moves a directory inside the workspace and fixes the workspace member list.
    pub fn rename_member(&self, from: &str, to: &str) {
        fs::rename(self.path(from), self.path(to)).expect("rename");
        let manifest = self.read("Cargo.toml").replace(from, to);
        self.write("Cargo.toml", &manifest);
    }

    pub fn context(&self, app_dir: &str) -> ProjectContext {
        ProjectContext::discover(&self.root, Some(Path::new(app_dir)))
            .expect("the fixture workspace is a project")
    }

    pub fn app(&self, app_dir: &str) -> AppDescriptor {
        let context = self.context(app_dir);
        AppDescriptor::resolve_from(&context, &self.root, app_dir).expect("the app resolves")
    }
}

pub fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
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
