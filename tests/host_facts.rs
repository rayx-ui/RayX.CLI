//! Host detection over fixture probes: one fixture per supported (and unsupported) host.

use rayx_cli::host::{
    Arch, FixtureProbe, HostFacts, MACHINE_AMD64, MACHINE_ARM64, MACHINE_UNKNOWN, Os, detect,
};

const UBUNTU_2404: &str = r#"PRETTY_NAME="Ubuntu 24.04.1 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
VERSION="24.04.1 LTS (Noble Numbat)"
VERSION_CODENAME=noble
ID=ubuntu
ID_LIKE=debian
"#;

const DEBIAN_12: &str = r#"PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
NAME="Debian GNU/Linux"
VERSION_ID="12"
VERSION_CODENAME=bookworm
ID=debian
"#;

const FEDORA_41: &str = r#"NAME="Fedora Linux"
VERSION_ID=41
ID=fedora
PRETTY_NAME="Fedora Linux 41 (Workstation Edition)"
"#;

fn ubuntu_native() -> FixtureProbe {
    FixtureProbe::new(Os::Linux, Arch::X64).with_file("/etc/os-release", UBUNTU_2404)
}

#[test]
fn windows_x64_is_native() {
    let probe = FixtureProbe::new(Os::Windows, Arch::X64)
        .with_windows_machines(MACHINE_UNKNOWN, MACHINE_AMD64);
    let facts = detect(&probe);
    assert_eq!(
        facts,
        HostFacts {
            os: Os::Windows,
            arch: Arch::X64,
            emulated: false,
            wsl: false,
            distro: None,
        }
    );
}

#[test]
fn windows_arm64_under_x64_emulation_reports_arm64() {
    // An x64 `rayx` on a Windows ARM64 machine: IsWow64Process2 reports an AMD64 process machine
    // on an ARM64 native machine.
    let probe = FixtureProbe::new(Os::Windows, Arch::X64)
        .with_windows_machines(MACHINE_AMD64, MACHINE_ARM64);
    let facts = detect(&probe);
    assert_eq!(facts.os, Os::Windows);
    assert_eq!(
        facts.arch,
        Arch::Arm64,
        "native architecture, not the process's"
    );
    assert!(facts.emulated);
}

#[test]
fn windows_arm64_native_process_is_not_emulated() {
    let probe = FixtureProbe::new(Os::Windows, Arch::Arm64)
        .with_windows_machines(MACHINE_UNKNOWN, MACHINE_ARM64);
    let facts = detect(&probe);
    assert_eq!(facts.arch, Arch::Arm64);
    assert!(!facts.emulated);
}

#[test]
fn windows_without_iswow64process2_falls_back_to_the_environment() {
    let emulated = FixtureProbe::new(Os::Windows, Arch::X64)
        .with_env("PROCESSOR_ARCHITECTURE", "AMD64")
        .with_env("PROCESSOR_ARCHITEW6432", "ARM64");
    let facts = detect(&emulated);
    assert_eq!((facts.arch, facts.emulated), (Arch::Arm64, true));

    let native =
        FixtureProbe::new(Os::Windows, Arch::X64).with_env("PROCESSOR_ARCHITECTURE", "AMD64");
    let facts = detect(&native);
    assert_eq!((facts.arch, facts.emulated), (Arch::X64, false));
}

#[test]
fn macos_arm64_under_rosetta_reports_arm64() {
    let probe = FixtureProbe::new(Os::MacOs, Arch::X64).with_macos_translated(true);
    let facts = detect(&probe);
    assert_eq!(facts.os, Os::MacOs);
    assert_eq!(
        facts.arch,
        Arch::Arm64,
        "Rosetta hides the Apple silicon CPU"
    );
    assert!(facts.emulated);
    assert!(facts.distro.is_none() && !facts.wsl);
}

#[test]
fn macos_native_arm64_and_intel_are_not_emulated() {
    let arm = detect(&FixtureProbe::new(Os::MacOs, Arch::Arm64).with_macos_translated(false));
    assert_eq!((arm.arch, arm.emulated), (Arch::Arm64, false));

    let intel = detect(&FixtureProbe::new(Os::MacOs, Arch::X64).with_macos_translated(false));
    assert_eq!((intel.arch, intel.emulated), (Arch::X64, false));
}

#[test]
fn ubuntu_2404_is_a_supported_distribution() {
    let facts = detect(&ubuntu_native());
    assert_eq!((facts.os, facts.arch), (Os::Linux, Arch::X64));
    assert!(!facts.wsl && !facts.emulated);
    let distro = facts.distro.expect("os-release is read");
    assert_eq!(distro.id, "ubuntu");
    assert_eq!(distro.version, "24.04");
    assert_eq!(distro.codename, "noble");
    assert_eq!(distro.name, "Ubuntu 24.04.1 LTS");
    assert!(distro.is_supported());
}

#[test]
fn ubuntu_under_wsl_is_detected_by_the_environment_variable() {
    let probe = ubuntu_native().with_env("WSL_DISTRO_NAME", "Ubuntu-24.04");
    let facts = detect(&probe);
    assert!(facts.wsl);
    assert_eq!(facts.distro.expect("distro").id, "ubuntu");
}

#[test]
fn wsl_is_detected_from_the_kernel_release_without_the_variable() {
    let probe = ubuntu_native().with_file(
        "/proc/sys/kernel/osrelease",
        "5.15.153.1-microsoft-standard-WSL2\n",
    );
    assert!(detect(&probe).wsl);

    let plain = ubuntu_native().with_file("/proc/sys/kernel/osrelease", "6.8.0-45-generic\n");
    assert!(!detect(&plain).wsl);
}

#[test]
fn debian_12_is_a_supported_distribution() {
    let probe = FixtureProbe::new(Os::Linux, Arch::Arm64).with_file("/etc/os-release", DEBIAN_12);
    let facts = detect(&probe);
    assert_eq!(facts.arch, Arch::Arm64);
    let distro = facts.distro.expect("distro");
    assert_eq!(
        (distro.id.as_str(), distro.version.as_str()),
        ("debian", "12")
    );
    assert!(distro.is_supported());
}

#[test]
fn fedora_is_detected_and_unsupported() {
    let probe = FixtureProbe::new(Os::Linux, Arch::X64).with_file("/etc/os-release", FEDORA_41);
    let distro = detect(&probe).distro.expect("distro");
    assert_eq!(distro.id, "fedora");
    assert_eq!(distro.version, "41");
    assert!(!distro.is_supported());
}

#[test]
fn debian_derivatives_are_supported_through_id_like() {
    let mint = "ID=linuxmint\nVERSION_ID=\"22\"\nID_LIKE=\"ubuntu debian\"\n";
    let probe = FixtureProbe::new(Os::Linux, Arch::X64).with_file("/etc/os-release", mint);
    assert!(detect(&probe).distro.expect("distro").is_supported());
}

#[test]
fn linux_without_os_release_has_no_distribution() {
    let facts = detect(&FixtureProbe::new(Os::Linux, Arch::X64));
    assert!(facts.distro.is_none());
}

#[test]
fn the_real_machine_reports_consistent_facts() {
    let facts = rayx_cli::host::facts();
    #[cfg(windows)]
    assert_eq!(facts.os, Os::Windows);
    #[cfg(target_os = "macos")]
    assert_eq!(facts.os, Os::MacOs);
    #[cfg(target_os = "linux")]
    assert_eq!(facts.os, Os::Linux);
    if facts.os != Os::Linux {
        assert!(!facts.wsl && facts.distro.is_none());
    }
}
