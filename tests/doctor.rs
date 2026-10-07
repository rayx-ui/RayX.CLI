//! `rayx doctor` over fixture hosts: statuses and fixes, GPU adapters and `hardwareAdapter`,
//! `interactiveDesktop` per OS, WSL findings, pins with their sources and the exit code.

use std::path::PathBuf;

use assert_cmd::cargo::cargo_bin_cmd;
use rayx_cli::doctor::{Report, Status, report_for};
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::{Pin, PinSource, Pins};
use rayx_cli::setup::gpu::GpuAdapter;
use rayx_cli::setup::{FakeMachine, PlanEnv};

const UBUNTU: &str = "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";
const FEDORA: &str = "ID=fedora\nVERSION_ID=41\nPRETTY_NAME=\"Fedora Linux 41\"\n";

fn env(os: Os, arch: Arch, wsl: bool, os_release: &str) -> PlanEnv {
    PlanEnv {
        host: HostFacts {
            os,
            arch,
            emulated: false,
            wsl,
            distro: (os == Os::Linux).then(|| Distro::parse(os_release)),
        },
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
    }
}

fn report(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> Report {
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    report_for(env, runner, machine, &mut path, None)
}

fn linux_machine() -> FakeMachine {
    FakeMachine::new().with_home("/home/dev")
}

fn requirement<'a>(report: &'a Report, id: &str) -> &'a rayx_cli::doctor::Requirement {
    report
        .sets
        .iter()
        .flat_map(|(_, items)| items)
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no requirement {id}"))
}

#[test]
fn requirements_are_present_missing_or_at_the_wrong_version_with_their_fix() {
    let mut env = env(Os::Linux, Arch::X64, false, UBUNTU);
    env.pins.wasm_bindgen = Some(Pin {
        value: "0.2.129".into(),
        source: PinSource::Project,
    });
    let machine = linux_machine()
        .with_program("node", "/usr/bin/node")
        .with_program("wasm-bindgen", "/home/dev/.cargo/bin/wasm-bindgen");
    let mut runner = Runner::record()
        .with_os(Os::Linux)
        .with_root(false)
        .responder(|spec| {
            let program = spec.program.rsplit('/').next().unwrap_or("");
            match program {
                "node" => Some(Outcome::success().with_stdout("v24.1.0\n")),
                "wasm-bindgen" => Some(Outcome::success().with_stdout("wasm-bindgen 0.2.100\n")),
                _ => None,
            }
        });
    let report = report(&env, &machine, &mut runner);

    let node = requirement(&report, "node");
    assert_eq!(node.status, Status::Present);
    assert_eq!(node.found.as_deref(), Some("24"));
    assert!(node.fix.is_empty());

    let bindgen = requirement(&report, "wasm-bindgen");
    assert_eq!(bindgen.status, Status::WrongVersion);
    assert_eq!(bindgen.found.as_deref(), Some("0.2.100"));
    assert!(
        bindgen.fix.iter().any(
            |c| c.contains("cargo install wasm-bindgen-cli --version 0.2.129 --force --locked")
        ),
        "{:?}",
        bindgen.fix
    );

    let apt = requirement(&report, "apt-packages");
    assert_eq!(apt.status, Status::Missing);
    assert!(
        apt.fix
            .iter()
            .any(|c| c.starts_with("sudo apt-get install ")),
        "{:?}",
        apt.fix
    );

    let text = report.to_text();
    assert!(
        text.contains("present       Node.js 22 or newer (found 24)"),
        "{text}"
    );
    assert!(text.contains("wrong version wasm-bindgen-cli"), "{text}");
    assert!(text.contains("fix: sudo apt-get install"), "{text}");
}

#[test]
fn the_json_document_carries_the_counts_the_sets_and_the_pins() {
    let env = env(Os::Linux, Arch::X64, false, UBUNTU);
    let report = report(
        &env,
        &linux_machine(),
        &mut Runner::record().with_os(Os::Linux),
    );
    let json = report.to_json();

    let count = json["requirementCount"].as_u64().expect("requirementCount");
    assert!(count >= 15, "{count}");
    assert_eq!(count as usize, report.requirement_count());
    let by_set: usize = json["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .map(|s| s["requirements"].as_array().expect("requirements").len())
        .sum();
    assert_eq!(by_set, count as usize);
    let names: Vec<&str> = json["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .map(|s| s["name"].as_str().expect("name"))
        .collect();
    assert_eq!(
        names,
        ["base", "web", "test", "android", "gpu"],
        "no iOS set on Linux"
    );

    assert_eq!(json["pins"]["web-toolchain"]["value"], "nightly-2026-06-23");
    assert_eq!(json["pins"]["web-toolchain"]["source"], "default");
    assert_eq!(json["pins"]["jdk"]["value"], "21");
    assert!(json["hardwareAdapter"].is_boolean() && json["interactiveDesktop"].is_boolean());
    assert_eq!(json["host"]["os"], "linux");
}

#[test]
fn pins_show_where_each_came_from() {
    let mut env = env(Os::Linux, Arch::X64, false, UBUNTU);
    env.pins.rust_toolchain = Some(Pin {
        value: "1.95.0".into(),
        source: PinSource::Project,
    });
    env.pins.android_platform = Pin {
        value: "android-36".into(),
        source: PinSource::GpuxCheckout,
    };
    let report = report(
        &env,
        &linux_machine(),
        &mut Runner::record().with_os(Os::Linux),
    );
    let json = report.to_json();
    assert_eq!(json["pins"]["rust-toolchain"]["source"], "project");
    assert_eq!(json["pins"]["android-platform"]["source"], "gpux-checkout");
    let text = report.to_text();
    assert!(text.contains("rust-toolchain: 1.95.0 (project)"), "{text}");
    assert!(
        text.contains("android-platform: android-36 (gpux-checkout)"),
        "{text}"
    );
    assert!(text.contains("node: 22 (default)"), "{text}");
}

#[test]
fn gpu_adapters_and_hardware_adapter_come_from_the_hosts_own_facilities() {
    let env = env(Os::Windows, Arch::X64, false, "");
    let with_gpu = FakeMachine::new().with_gpu_adapters(vec![
        GpuAdapter::hardware(0x10DE, "NVIDIA GeForce RTX 4070", Some("32.0.15.6094")),
        GpuAdapter::software("Microsoft Basic Render Driver"),
    ]);
    let report = report(&env, &with_gpu, &mut Runner::record().with_os(Os::Windows));
    let json = report.to_json();
    assert_eq!(json["hardwareAdapter"], true);
    assert_eq!(json["gpuAdapters"][0]["vendor"], "NVIDIA");
    assert_eq!(json["gpuAdapters"][0]["driverVersion"], "32.0.15.6094");
    assert_eq!(json["gpuAdapters"][1]["software"], true);
    let text = report.to_text();
    assert!(
        text.contains("GPU: NVIDIA NVIDIA GeForce RTX 4070, driver 32.0.15.6094"),
        "{text}"
    );
    assert!(text.contains("Hardware adapter usable: yes"), "{text}");

    let basic_only = FakeMachine::new()
        .with_gpu_adapters(vec![GpuAdapter::software("Microsoft Basic Render Driver")]);
    let report = report_for_windows(&env, &basic_only);
    assert_eq!(report.to_json()["hardwareAdapter"], false);
    assert!(report.to_text().contains("Hardware adapter usable: no"));
}

fn report_for_windows(env: &PlanEnv, machine: &FakeMachine) -> Report {
    report(env, machine, &mut Runner::record().with_os(Os::Windows))
}

#[test]
fn an_interactive_desktop_is_a_console_session_on_windows_a_display_on_linux_and_aqua_on_macos() {
    let windows = env(Os::Windows, Arch::X64, false, "");
    let console = report_for_windows(&windows, &FakeMachine::new().with_console_session(true));
    assert_eq!(console.to_json()["interactiveDesktop"], true);
    let service = report_for_windows(&windows, &FakeMachine::new().with_console_session(false));
    assert_eq!(service.to_json()["interactiveDesktop"], false);

    let linux = env(Os::Linux, Arch::X64, false, UBUNTU);
    let runner = || Runner::record().with_os(Os::Linux);
    let x11 = report(
        &linux,
        &linux_machine().with_env("DISPLAY", ":0"),
        &mut runner(),
    );
    assert_eq!(x11.to_json()["interactiveDesktop"], true);
    let wayland = report(
        &linux,
        &linux_machine().with_env("WAYLAND_DISPLAY", "wayland-0"),
        &mut runner(),
    );
    assert_eq!(wayland.to_json()["interactiveDesktop"], true);
    let ssh = report(&linux, &linux_machine(), &mut runner());
    assert_eq!(ssh.to_json()["interactiveDesktop"], false);

    let mac = env(Os::MacOs, Arch::Arm64, false, "");
    let session = |name: &'static str| {
        let mut runner = Runner::record().with_os(Os::MacOs).responder(move |spec| {
            (spec.program == "launchctl")
                .then(|| Outcome::success().with_stdout(format!("{name}\n")))
        });
        report(
            &mac,
            &FakeMachine::new().with_home("/Users/dev"),
            &mut runner,
        )
        .to_json()["interactiveDesktop"]
            .clone()
    };
    assert_eq!(session("Aqua"), true);
    assert_eq!(session("Background"), false);
}

#[test]
fn wsl_findings_flag_windows_binaries_path_order_and_the_vulkan_driver_setup() {
    let env = env(Os::Linux, Arch::X64, true, UBUNTU);
    let machine = linux_machine()
        .with_program("pnpm", "/mnt/c/Users/dev/AppData/Roaming/npm/pnpm")
        .with_program("cargo", "/home/dev/.cargo/bin/cargo")
        .with_env(
            "PATH",
            "/usr/bin:/mnt/c/Windows/system32:/home/dev/.local/bin",
        );
    let report = report(&env, &machine, &mut Runner::record().with_os(Os::Linux));
    let wsl = report.to_json()["wsl"].clone();
    let findings: Vec<String> = wsl
        .as_array()
        .expect("wsl findings")
        .iter()
        .map(|f| f.as_str().expect("text").to_string())
        .collect();
    assert!(
        findings
            .iter()
            .any(|f| f.contains("`pnpm` resolves to /mnt/c/")),
        "{findings:?}"
    );
    assert!(
        !findings.iter().any(|f| f.contains("`cargo` resolves")),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.contains("/mnt/c entries before ~/.local/bin")),
        "{findings:?}"
    );
    assert!(
        findings.iter().any(|f| f == "VK_DRIVER_FILES is not set"),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.starts_with("lavapipe (software Vulkan fallback): not installed")),
        "{findings:?}"
    );

    // Correct order, lavapipe installed, driver files set: nothing to flag but the facts.
    let good = linux_machine()
        .with_env(
            "PATH",
            "/home/dev/.local/bin:/usr/bin:/mnt/c/Windows/system32",
        )
        .with_env(
            "VK_DRIVER_FILES",
            "/usr/share/vulkan/icd.d/lvp_icd.x86_64.json",
        )
        .with_file(PathBuf::from("/usr/share/vulkan/icd.d").join("lvp_icd.x86_64.json"));
    let report = report_good(&env, &good);
    let findings = report.to_json()["wsl"].to_string();
    assert!(!findings.contains("before ~/.local/bin"), "{findings}");
    assert!(
        findings.contains("VK_DRIVER_FILES = /usr/share/vulkan/icd.d/lvp_icd.x86_64.json"),
        "{findings}"
    );
    assert!(
        findings.contains("lavapipe (software Vulkan fallback): installed"),
        "{findings}"
    );
}

fn report_good(env: &PlanEnv, machine: &FakeMachine) -> Report {
    report(env, machine, &mut Runner::record().with_os(Os::Linux))
}

#[test]
fn an_unsupported_distribution_is_still_reported_and_doctor_still_succeeds() {
    let env = env(Os::Linux, Arch::X64, false, FEDORA);
    let report = report(
        &env,
        &linux_machine().with_env("DISPLAY", ":0"),
        &mut Runner::record().with_os(Os::Linux),
    );
    assert!(
        report
            .unsupported
            .as_deref()
            .is_some_and(|r| r.contains("Fedora Linux 41"))
    );
    assert_eq!(report.requirement_count(), 0);
    let text = report.to_text();
    assert!(text.contains("Requirements are not listed"), "{text}");
    assert!(
        text.contains("Interactive desktop session: yes"),
        "the environment is still reported:\n{text}"
    );
}

#[test]
fn the_binary_prints_one_json_document_for_this_machine_and_exits_zero() {
    let output = cargo_bin_cmd!("rayx")
        .args(["doctor", "--json"])
        .assert()
        .success()
        .get_output()
        .clone();
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("one JSON document");
    assert!(json["requirementCount"].as_u64().is_some_and(|n| n > 0));
    assert!(json["hardwareAdapter"].is_boolean());
    assert!(json["interactiveDesktop"].is_boolean());

    let text = cargo_bin_cmd!("rayx")
        .arg("doctor")
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(String::from_utf8_lossy(&text.stdout).starts_with("rayx doctor:"));
}

#[test]
fn on_windows_doctor_reports_each_wsl_virtual_disk_with_its_size() {
    use rayx_cli::doctor::format_bytes;
    use rayx_cli::wsl::Distribution;

    let env = env(Os::Windows, Arch::X64, false, "");
    let ubuntu = Distribution {
        id: "{abc}".into(),
        name: "Ubuntu-24.04".into(),
        base_path: PathBuf::from(r"\\?\C:\Users\dev\AppData\Local\Ubuntu"),
        version: Some(2),
        default_uid: Some(1000),
    };
    let vhdx = ubuntu.vhdx();
    assert_eq!(
        vhdx.file_name().and_then(|n| n.to_str()),
        Some("ext4.vhdx"),
        "{vhdx:?}"
    );
    assert!(!vhdx.to_string_lossy().starts_with(r"\\?\"));
    let machine = FakeMachine::new()
        .with_wsl_distributions(vec![ubuntu])
        .with_size(&vhdx, 213_581_758_464);
    let report = report_for_windows(&env, &machine);
    let json = report.to_json();
    assert_eq!(json["wslDisks"][0]["distribution"], "Ubuntu-24.04");
    assert_eq!(json["wslDisks"][0]["bytes"], 213_581_758_464u64);
    let text = report.to_text();
    assert!(text.contains("WSL disk: Ubuntu-24.04 198.9 GiB"), "{text}");

    assert_eq!(format_bytes(512), "512 B");
    assert_eq!(format_bytes(1536), "1.5 KiB");
    assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");

    // A missing disk file is reported, not hidden.
    let unreadable = FakeMachine::new().with_wsl_distributions(vec![Distribution {
        id: "{x}".into(),
        name: "Debian".into(),
        base_path: PathBuf::from(r"C:\wsl\debian"),
        version: Some(2),
        default_uid: None,
    }]);
    let text = report_for_windows(&env, &unreadable).to_text();
    assert!(text.contains("WSL disk: Debian size unknown"), "{text}");
}
