//! Helpers shared by the test groups: a temporary copy of a fixture project, so nothing a test
//! writes lands in the repository.

#![allow(dead_code)]

pub mod proof;

use rayx_cli::host::{Outcome, Runner};

/// A canonical path as a process sees its working directory: without the `\\?\` prefix Windows'
/// canonical paths carry (a UNC path keeps it).
fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

/// A recording runner that runs Cargo's read-only queries (`locate-project`, `metadata`) for
/// real, as a printing runner runs its queries, so pins are read from the fixtures; every other
/// command is recorded.
pub fn reading_cargo() -> Runner {
    Runner::record().responder(|spec| {
        let query = spec.program == "cargo"
            && spec
                .args
                .first()
                .is_some_and(|a| a == "locate-project" || a == "metadata");
        query.then(|| {
            let mut command = std::process::Command::new("cargo");
            command.args(&spec.args);
            if let Some(cwd) = &spec.cwd {
                command.current_dir(cwd);
            }
            for (key, value) in &spec.env {
                command.env(key, value);
            }
            let output = command.output().expect("cargo");
            Outcome {
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }
        })
    })
}

use std::fs;
use std::path::{Path, PathBuf};

/// A temporary copy of `tests/fixtures/<name>`, or an empty temporary directory.
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
        let workspace = Self::empty();
        copy_tree(&source, &workspace.root).expect("copy the fixture");
        workspace
    }

    pub fn empty() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = plain(fs::canonicalize(dir.path()).expect("canonical temp dir"));
        Self { _dir: dir, root }
    }

    /// `relative` (`/`-separated) inside the workspace, written with this host's separators.
    pub fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(self.root.clone(), |path, part| path.join(part))
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
