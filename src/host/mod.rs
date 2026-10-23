//! What this machine is: operating system, native CPU architecture, WSL and Linux distribution.
//!
//! [`detect`] is a pure function over a [`Probe`], so every host in the support matrix is tested
//! with a fixture probe. [`facts`] runs it against the real machine.

pub mod path_env;
pub mod probe;
pub mod runner;

pub use probe::{FixtureProbe, Probe, SystemProbe};
pub use runner::{CommandSpec, Elevator, Mode, Outcome, Privilege, RunError, Runner};

/// Machine types reported by `IsWow64Process2`.
pub const MACHINE_UNKNOWN: u16 = 0x0000;
/// `IMAGE_FILE_MACHINE_AMD64`.
pub const MACHINE_AMD64: u16 = 0x8664;
/// `IMAGE_FILE_MACHINE_ARM64`.
pub const MACHINE_ARM64: u16 = 0xAA64;

/// The operating system family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

impl Os {
    pub fn as_str(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::MacOs => "macos",
            Os::Linux => "linux",
        }
    }
}

/// A CPU architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X64,
    Arm64,
    /// Any other architecture; `rayx` cannot install tools for it.
    Other,
}

impl Arch {
    pub fn as_str(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::Arm64 => "arm64",
            Arch::Other => "other",
        }
    }

    /// Maps a Rust `std::env::consts::ARCH` value.
    pub fn from_rust(arch: &str) -> Self {
        match arch {
            "x86_64" => Arch::X64,
            "aarch64" => Arch::Arm64,
            _ => Arch::Other,
        }
    }

    /// Maps a Windows `PROCESSOR_ARCHITECTURE` value.
    fn from_windows_env(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "AMD64" | "X64" => Arch::X64,
            "ARM64" => Arch::Arm64,
            _ => Arch::Other,
        }
    }

    /// Maps an `IMAGE_FILE_MACHINE_*` value.
    fn from_machine(machine: u16) -> Self {
        match machine {
            MACHINE_AMD64 => Arch::X64,
            MACHINE_ARM64 => Arch::Arm64,
            _ => Arch::Other,
        }
    }
}

/// A Linux distribution, read from `/etc/os-release`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Distro {
    /// `ID`, for example `ubuntu`.
    pub id: String,
    /// `VERSION_ID`, for example `24.04`.
    pub version: String,
    /// `VERSION_CODENAME`, for example `noble`.
    pub codename: String,
    /// `PRETTY_NAME`.
    pub name: String,
    /// `ID_LIKE`, split on whitespace.
    pub id_like: Vec<String>,
}

impl Distro {
    /// Parses the contents of `/etc/os-release`.
    pub fn parse(os_release: &str) -> Self {
        let mut distro = Distro::default();
        for line in os_release.lines() {
            let line = line.trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            match key.trim() {
                "ID" => distro.id = value.to_ascii_lowercase(),
                "VERSION_ID" => distro.version = value.to_string(),
                "VERSION_CODENAME" => distro.codename = value.to_string(),
                "PRETTY_NAME" => distro.name = value.to_string(),
                "ID_LIKE" => {
                    distro.id_like = value
                        .split_whitespace()
                        .map(str::to_ascii_lowercase)
                        .collect();
                }
                _ => {}
            }
        }
        if distro.name.is_empty() {
            distro.name = format!("{} {}", distro.id, distro.version)
                .trim()
                .to_string();
        }
        distro
    }

    /// Whether `setup` supports this distribution: Ubuntu, Debian and distributions derived
    /// from them, which all install through `apt`.
    pub fn is_supported(&self) -> bool {
        let debian_like = |id: &str| id == "ubuntu" || id == "debian";
        debian_like(&self.id) || self.id_like.iter().any(|id| debian_like(id))
    }
}

/// The facts about this machine that decide what `setup` installs and `doctor` reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostFacts {
    pub os: Os,
    /// The native CPU architecture of the machine, which is not always the architecture of the
    /// running `rayx` process: see [`HostFacts::emulated`].
    pub arch: Arch,
    /// Whether this process runs under emulation (x64 `rayx` on Windows ARM64, or under Rosetta
    /// on an Apple silicon Mac). Tools are installed for [`HostFacts::arch`], not for the process.
    pub emulated: bool,
    /// Whether this is Linux running inside WSL.
    pub wsl: bool,
    /// The Linux distribution; `None` on Windows and macOS, and on Linux without `os-release`.
    pub distro: Option<Distro>,
}

/// Detects the facts of the machine this process runs on.
pub fn facts() -> HostFacts {
    detect(&SystemProbe)
}

/// Computes [`HostFacts`] from `probe`.
pub fn detect(probe: &dyn Probe) -> HostFacts {
    let os = probe.os();
    let process_arch = probe.process_arch();
    let (arch, emulated) = match os {
        Os::Windows => windows_arch(probe, process_arch),
        Os::MacOs => macos_arch(probe, process_arch),
        Os::Linux => (process_arch, false),
    };
    let wsl = os == Os::Linux && is_wsl(probe);
    let distro = if os == Os::Linux {
        probe
            .read_file("/etc/os-release")
            .map(|text| Distro::parse(&text))
    } else {
        None
    };
    HostFacts {
        os,
        arch,
        emulated,
        wsl,
        distro,
    }
}

/// `IsWow64Process2` reports the native machine and whether the process runs through WOW64 or
/// the ARM64 x64 emulator. Older Windows falls back to the `PROCESSOR_ARCHITEW6432` variable.
fn windows_arch(probe: &dyn Probe, process_arch: Arch) -> (Arch, bool) {
    if let Some((process_machine, native_machine)) = probe.windows_machines() {
        return (
            Arch::from_machine(native_machine),
            process_machine != MACHINE_UNKNOWN,
        );
    }
    match probe.env("PROCESSOR_ARCHITEW6432") {
        Some(native) if !native.is_empty() => (Arch::from_windows_env(&native), true),
        _ => match probe.env("PROCESSOR_ARCHITECTURE") {
            Some(native) if !native.is_empty() => (Arch::from_windows_env(&native), false),
            _ => (process_arch, false),
        },
    }
}

/// An x64 process on an Apple silicon Mac runs under Rosetta and sees `sysctl.proc_translated`
/// equal to 1.
fn macos_arch(probe: &dyn Probe, process_arch: Arch) -> (Arch, bool) {
    match (process_arch, probe.macos_translated()) {
        (Arch::X64, Some(true)) => (Arch::Arm64, true),
        _ => (process_arch, false),
    }
}

fn is_wsl(probe: &dyn Probe) -> bool {
    if probe
        .env("WSL_DISTRO_NAME")
        .is_some_and(|name| !name.is_empty())
    {
        return true;
    }
    probe
        .read_file("/proc/sys/kernel/osrelease")
        .is_some_and(|release| {
            let release = release.to_ascii_lowercase();
            release.contains("microsoft") || release.contains("wsl")
        })
}
