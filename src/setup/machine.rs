//! What setup may observe on the machine without running a program: files, directories,
//! environment variables and programs on `PATH`. [`SystemMachine`] is the real machine and
//! [`FakeMachine`] a scripted one for tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub trait Machine {
    fn exists(&self, path: &Path) -> bool;
    /// The text of a file, or `None` when it cannot be read.
    fn read_to_string(&self, path: &Path) -> Option<String>;
    fn is_dir(&self, path: &Path) -> bool;
    /// The names of the entries in `dir`, sorted; empty when it cannot be read.
    fn list_dir(&self, dir: &Path) -> Vec<String>;
    fn env(&self, key: &str) -> Option<String>;
    /// The user's home directory.
    fn home(&self) -> Option<PathBuf>;
    /// The size of a file in bytes.
    fn file_size(&self, path: &Path) -> Option<u64> {
        let _ = path;
        None
    }
    /// The WSL distributions registered for the current user (Windows).
    fn wsl_distributions(&self) -> Vec<crate::wsl::Distribution> {
        Vec::new()
    }
    /// Whether this process runs in the console session of the machine (Windows); `None` where
    /// the notion does not apply.
    fn console_session(&self) -> Option<bool> {
        None
    }
    /// The display adapters DXGI reports (Windows); empty elsewhere, where setup asks `lspci`,
    /// `system_profiler` and `powershell.exe` instead.
    fn gpu_adapters(&self) -> Vec<super::gpu::GpuAdapter> {
        Vec::new()
    }
    /// The first program called `name` on `PATH`.
    fn which(&self, name: &str) -> Option<PathBuf>;
    /// Every program called `name` on `PATH`, in PATH order.
    fn which_all(&self, name: &str) -> Vec<PathBuf> {
        self.which(name).into_iter().collect()
    }
    /// When a file was last modified.
    fn modified(&self, path: &Path) -> Option<std::time::SystemTime> {
        let _ = path;
        None
    }
}

/// The real machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemMachine;

impl Machine for SystemMachine {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read_to_string(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
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

    fn gpu_adapters(&self) -> Vec<super::gpu::GpuAdapter> {
        super::gpu::dxgi_adapters()
    }

    fn file_size(&self, path: &Path) -> Option<u64> {
        std::fs::metadata(path).ok().map(|meta| meta.len())
    }

    fn wsl_distributions(&self) -> Vec<crate::wsl::Distribution> {
        crate::wsl::registry_distributions()
    }

    fn console_session(&self) -> Option<bool> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::RemoteDesktop::{
                ProcessIdToSessionId, WTSGetActiveConsoleSessionId,
            };
            use windows_sys::Win32::System::Threading::GetCurrentProcessId;

            let mut session = 0u32;
            // SAFETY: `session` is a live out-pointer; both calls have no other preconditions.
            let (known, console) = unsafe {
                (
                    ProcessIdToSessionId(GetCurrentProcessId(), &mut session),
                    WTSGetActiveConsoleSessionId(),
                )
            };
            Some(known != 0 && session == console)
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    fn home(&self) -> Option<PathBuf> {
        let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        std::env::var_os(variable).map(PathBuf::from)
    }

    fn modified(&self, path: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(path).ok()?.modified().ok()
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        self.which_all(name).into_iter().next()
    }

    fn which_all(&self, name: &str) -> Vec<PathBuf> {
        let Some(path) = std::env::var_os("PATH") else {
            return Vec::new();
        };
        let extensions: Vec<String> = if cfg!(windows) {
            std::env::var("PATHEXT")
                .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
                .split(';')
                .map(str::to_ascii_lowercase)
                .collect()
        } else {
            vec![String::new()]
        };
        std::env::split_paths(&path)
            .filter_map(|dir| {
                extensions.iter().find_map(|extension| {
                    let candidate = dir.join(format!("{name}{extension}"));
                    candidate.is_file().then_some(candidate)
                })
            })
            .collect()
    }
}

/// A scripted machine: the paths, directories, variables and programs a test declares.
#[derive(Clone, Debug, Default)]
pub struct FakeMachine {
    files: BTreeSet<PathBuf>,
    contents: BTreeMap<PathBuf, String>,
    dirs: BTreeMap<PathBuf, Vec<String>>,
    env: BTreeMap<String, String>,
    programs: BTreeMap<String, Vec<PathBuf>>,
    modified: BTreeMap<PathBuf, u64>,
    home: Option<PathBuf>,
    adapters: Vec<super::gpu::GpuAdapter>,
    console: Option<bool>,
    sizes: BTreeMap<PathBuf, u64>,
    wsl: Vec<crate::wsl::Distribution>,
}

impl FakeMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.home = Some(home.into());
        self
    }

    /// The adapters DXGI reports on a fake Windows machine.
    pub fn with_gpu_adapters(mut self, adapters: Vec<super::gpu::GpuAdapter>) -> Self {
        self.adapters = adapters;
        self
    }

    /// Declares when a file was last modified, in seconds after the Unix epoch.
    pub fn with_modified(mut self, path: impl Into<PathBuf>, secs: u64) -> Self {
        self.modified.insert(path.into(), secs);
        self
    }

    /// Declares a file's size in bytes.
    pub fn with_size(mut self, path: impl Into<PathBuf>, bytes: u64) -> Self {
        self.sizes.insert(path.into(), bytes);
        self
    }

    /// The WSL distributions of a fake Windows machine.
    pub fn with_wsl_distributions(mut self, distributions: Vec<crate::wsl::Distribution>) -> Self {
        self.wsl = distributions;
        self
    }

    /// Whether the fake Windows process is in the console session.
    pub fn with_console_session(mut self, console: bool) -> Self {
        self.console = Some(console);
        self
    }

    pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.register_parents(&path);
        self.files.insert(path);
        self
    }

    /// Declares a file with this text.
    pub fn with_text(mut self, path: impl Into<PathBuf>, text: &str) -> Self {
        let path = path.into();
        self.contents.insert(path.clone(), text.to_string());
        self.with_file(path)
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
        self.programs
            .entry(name.to_string())
            .or_default()
            .push(path.into());
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

    fn read_to_string(&self, path: &Path) -> Option<String> {
        self.contents.get(path).cloned()
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

    fn gpu_adapters(&self) -> Vec<super::gpu::GpuAdapter> {
        self.adapters.clone()
    }

    fn console_session(&self) -> Option<bool> {
        self.console
    }

    fn file_size(&self, path: &Path) -> Option<u64> {
        self.sizes.get(path).copied()
    }

    fn wsl_distributions(&self) -> Vec<crate::wsl::Distribution> {
        self.wsl.clone()
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        self.programs
            .get(name)
            .and_then(|paths| paths.first().cloned())
    }

    fn which_all(&self, name: &str) -> Vec<PathBuf> {
        self.programs.get(name).cloned().unwrap_or_default()
    }

    fn modified(&self, path: &Path) -> Option<std::time::SystemTime> {
        self.modified
            .get(path)
            .map(|secs| std::time::UNIX_EPOCH + std::time::Duration::from_secs(*secs))
    }
}
