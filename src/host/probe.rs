//! The questions host detection asks of the machine, with a real answer ([`SystemProbe`]) and a
//! scripted one ([`FixtureProbe`]).

use std::collections::BTreeMap;

use super::{Arch, Os};

/// Everything [`super::detect`] needs to know about the machine.
pub trait Probe {
    /// The operating system family this binary runs on.
    fn os(&self) -> Os;
    /// The architecture this process was compiled for.
    fn process_arch(&self) -> Arch;
    /// An environment variable.
    fn env(&self, key: &str) -> Option<String>;
    /// The text of a file, or `None` when it cannot be read.
    fn read_file(&self, path: &str) -> Option<String>;
    /// `(process machine, native machine)` from `IsWow64Process2`, on Windows only. The process
    /// machine is [`super::MACHINE_UNKNOWN`] when the process is not emulated.
    fn windows_machines(&self) -> Option<(u16, u16)>;
    /// `sysctl.proc_translated` on macOS: `Some(true)` under Rosetta.
    fn macos_translated(&self) -> Option<bool>;
}

/// The real machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemProbe;

impl Probe for SystemProbe {
    fn os(&self) -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }

    fn process_arch(&self) -> Arch {
        Arch::from_rust(std::env::consts::ARCH)
    }

    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn windows_machines(&self) -> Option<(u16, u16)> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};

            let mut process_machine = 0;
            let mut native_machine = 0;
            // SAFETY: both out-pointers refer to live locals, and the pseudo-handle returned by
            // `GetCurrentProcess` is always valid.
            let ok = unsafe {
                IsWow64Process2(
                    GetCurrentProcess(),
                    &mut process_machine,
                    &mut native_machine,
                )
            };
            (ok != 0).then_some((process_machine, native_machine))
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    fn macos_translated(&self) -> Option<bool> {
        #[cfg(target_os = "macos")]
        {
            let mut translated: libc::c_int = 0;
            let mut size = std::mem::size_of::<libc::c_int>();
            // SAFETY: the name is a NUL-terminated string, and `translated` and `size` are live
            // locals describing a buffer of exactly `size` bytes.
            let status = unsafe {
                libc::sysctlbyname(
                    c"sysctl.proc_translated".as_ptr(),
                    (&mut translated as *mut libc::c_int).cast(),
                    &mut size,
                    std::ptr::null_mut(),
                    0,
                )
            };
            // The key does not exist on Intel Macs, which are never translated.
            Some(status == 0 && translated == 1)
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }
}

/// A scripted machine for tests and dry runs.
#[derive(Clone, Debug)]
pub struct FixtureProbe {
    os: Os,
    process_arch: Arch,
    env: BTreeMap<String, String>,
    files: BTreeMap<String, String>,
    windows_machines: Option<(u16, u16)>,
    macos_translated: Option<bool>,
}

impl FixtureProbe {
    /// A machine running `os` with a process compiled for `process_arch` and nothing else set.
    pub fn new(os: Os, process_arch: Arch) -> Self {
        Self {
            os,
            process_arch,
            env: BTreeMap::new(),
            files: BTreeMap::new(),
            windows_machines: None,
            macos_translated: None,
        }
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_file(mut self, path: &str, contents: &str) -> Self {
        self.files.insert(path.to_string(), contents.to_string());
        self
    }

    /// The `IsWow64Process2` answer: `(process machine, native machine)`.
    pub fn with_windows_machines(mut self, process_machine: u16, native_machine: u16) -> Self {
        self.windows_machines = Some((process_machine, native_machine));
        self
    }

    pub fn with_macos_translated(mut self, translated: bool) -> Self {
        self.macos_translated = Some(translated);
        self
    }
}

impl Probe for FixtureProbe {
    fn os(&self) -> Os {
        self.os
    }

    fn process_arch(&self) -> Arch {
        self.process_arch
    }

    fn env(&self, key: &str) -> Option<String> {
        self.env.get(key).cloned()
    }

    fn read_file(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }

    fn windows_machines(&self) -> Option<(u16, u16)> {
        self.windows_machines
    }

    fn macos_translated(&self) -> Option<bool> {
        self.macos_translated
    }
}
