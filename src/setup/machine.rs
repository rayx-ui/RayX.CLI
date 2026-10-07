//! What setup may observe on the machine without running a program: files, directories,
//! environment variables and programs on `PATH`. [`SystemMachine`] is the real machine and
//! [`FakeMachine`] a scripted one for tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub trait Machine {
    fn exists(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    /// The names of the entries in `dir`, sorted; empty when it cannot be read.
    fn list_dir(&self, dir: &Path) -> Vec<String>;
    fn env(&self, key: &str) -> Option<String>;
    /// The user's home directory.
    fn home(&self) -> Option<PathBuf>;
    /// The first program called `name` on `PATH`.
    fn which(&self, name: &str) -> Option<PathBuf>;
}

/// The real machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemMachine;

impl Machine for SystemMachine {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn list_dir(&self, dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn home(&self) -> Option<PathBuf> {
        let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        std::env::var_os(variable).map(PathBuf::from)
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        let extensions: Vec<String> = if cfg!(windows) {
            std::env::var("PATHEXT")
                .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
                .split(';')
                .map(str::to_ascii_lowercase)
                .collect()
        } else {
            vec![String::new()]
        };
        std::env::split_paths(&path).find_map(|dir| {
            extensions.iter().find_map(|extension| {
                let candidate = dir.join(format!("{name}{extension}"));
                candidate.is_file().then_some(candidate)
            })
        })
    }
}

/// A scripted machine: the paths, directories, variables and programs a test declares.
#[derive(Clone, Debug, Default)]
pub struct FakeMachine {
    files: BTreeSet<PathBuf>,
    dirs: BTreeMap<PathBuf, Vec<String>>,
    env: BTreeMap<String, String>,
    programs: BTreeMap<String, PathBuf>,
    home: Option<PathBuf>,
}

impl FakeMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.home = Some(home.into());
        self
    }

    pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.register_parents(&path);
        self.files.insert(path);
        self
    }

    /// Declares a directory (and, through [`FakeMachine::with_file`], what it contains).
    pub fn with_dir(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.register_parents(&path);
        self.dirs.entry(path).or_default();
        self
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_program(mut self, name: &str, path: impl Into<PathBuf>) -> Self {
        self.programs.insert(name.to_string(), path.into());
        self
    }

    fn register_parents(&mut self, path: &Path) {
        let mut child = path.to_path_buf();
        while let Some(parent) = child.parent().map(Path::to_path_buf) {
            if parent.as_os_str().is_empty() {
                break;
            }
            let name = child
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let entries = self.dirs.entry(parent.clone()).or_default();
            if !entries.contains(&name) {
                entries.push(name);
                entries.sort();
            }
            child = parent;
        }
    }
}

impl Machine for FakeMachine {
    fn exists(&self, path: &Path) -> bool {
        self.files.contains(path) || self.dirs.contains_key(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.contains_key(path)
    }

    fn list_dir(&self, dir: &Path) -> Vec<String> {
        self.dirs.get(dir).cloned().unwrap_or_default()
    }

    fn env(&self, key: &str) -> Option<String> {
        self.env.get(key).cloned()
    }

    fn home(&self) -> Option<PathBuf> {
        self.home.clone()
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }
}
