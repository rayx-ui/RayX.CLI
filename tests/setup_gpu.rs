//! The GPU set over fixture adapter lists: NVIDIA, AMD and Intel on Windows, Adreno on Windows
//! ARM64, the Basic Render Driver alone, Ubuntu with NVIDIA and with Mesa, WSL and macOS.

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::gpu::{
    GpuAdapter, Vendor, has_hardware_adapter, parse_lspci, parse_system_profiler, parse_vulkaninfo,
    parse_windows_video_controllers,
};
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, plan, run_plan};

const UBUNTU: &str = "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";

fn env(os: Os, arch: Arch, wsl: bool) -> PlanEnv {
    PlanEnv {
        host: HostFacts {
            os,
            arch,
            emulated: false,
            wsl,
            distro: (os == Os::Linux).then(|| Distro::parse(UBUNTU)),
        },
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
    }
}

fn check(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> (u8, String) {
    // Only the GPU steps: the base set has its own tests.
    let mut plan = plan(env, &[Set::Gpu]).expect("plan");
    plan.steps.retain(|step| step.set == Set::Gpu);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut out = Vec::new();
    let mut cx = Cx::new(env, runner, machine, &mut path);
    let args = SetupArgs {
        check: true,
        gpu: true,
        ..SetupArgs::default()
    };
    let code = run_plan(&args, &plan, &mut cx, &mut out);
    (code, String::from_utf8(out).expect("UTF-8"))
}

fn gpu_line(text: &str) -> &str {
    text.lines()
        .find(|l| l.starts_with('[') && l.contains("gpu: GPU drivers"))
        .unwrap_or_else(|| panic!("no GPU step:\n{text}"))
}

fn windows_machine(adapters: Vec<GpuAdapter>) -> FakeMachine {
    FakeMachine::new()
        .with_home(r"C:\Users\dev")
        .with_gpu_adapters(adapters)
}

#[test]
fn nvidia_amd_and_intel_adapters_with_drivers_need_nothing_on_windows() {
    let env = env(Os::Windows, Arch::X64, false);
    for adapters in [
        vec![GpuAdapter::hardware(
            0x10DE,
            "NVIDIA GeForce RTX 4070",
            Some("32.0.15.6094"),
        )],
        vec![GpuAdapter::hardware(
            0x1002,
            "AMD Radeon RX 7800 XT",
            Some("31.0.24033.1003"),
        )],
        vec![
            GpuAdapter::hardware(0x8086, "Intel(R) Arc(TM) A770", Some("31.0.101.5333")),
            GpuAdapter::software("Microsoft Basic Render Driver"),
        ],
    ] {
        let (code, text) = check(
            &env,
            &windows_machine(adapters.clone()),
            &mut Runner::record().with_os(Os::Windows),
        );
        assert_eq!(code, 0, "{adapters:?}\n{text}");
        assert!(gpu_line(&text).starts_with("[ok     ]"), "{text}");
        assert!(gpu_line(&text).contains(&adapters[0].name), "{text}");
    }
}

#[test]
fn a_windows_adapter_without_a_driver_gets_windows_update_and_intel_gets_its_assistant() {
    let env = env(Os::Windows, Arch::X64, false);
    let nvidia = windows_machine(vec![GpuAdapter::hardware(
        0x10DE,
        "NVIDIA GeForce RTX 4070",
        None,
    )]);
    let (code, text) = check(&env, &nvidia, &mut Runner::record().with_os(Os::Windows));
    assert_eq!(code, 1, "{text}");
    assert!(
        gpu_line(&text).contains("NVIDIA GeForce RTX 4070"),
        "{text}"
    );
    assert!(
        text.contains("UsoClient.exe -ArgumentList StartScan"),
        "{text}"
    );
    assert!(
        text.contains("Start-Process ms-settings:windowsupdate-optionalupdates"),
        "{text}"
    );
    assert!(
        !text.contains("winget"),
        "NVIDIA has no winget driver package:\n{text}"
    );

    let intel = windows_machine(vec![GpuAdapter::hardware(
        0x8086,
        "Intel(R) Iris(R) Xe Graphics",
        None,
    )]);
    let (_, text) = check(&env, &intel, &mut Runner::record().with_os(Os::Windows));
    assert!(
        text.contains("winget install --id Intel.IntelDriverAndSupportAssistant --exact"),
        "{text}"
    );
    assert!(text.contains("windowsupdate-optionalupdates"), "{text}");
}

#[test]
fn adreno_on_windows_arm64_takes_the_windows_update_path() {
    let env = env(Os::Windows, Arch::Arm64, false);
    let adreno = windows_machine(vec![GpuAdapter::hardware(
        0x5143,
        "Qualcomm(R) Adreno(TM) X1-85 GPU",
        None,
    )]);
    let (code, text) = check(&env, &adreno, &mut Runner::record().with_os(Os::Windows));
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("StartScan"), "{text}");
    assert!(!text.contains("winget"), "{text}");
    assert_eq!(Vendor::from_pci(0x5143), Vendor::Qualcomm);
}

#[test]
fn only_the_basic_render_driver_counts_as_a_missing_driver() {
    let env = env(Os::Windows, Arch::X64, false);
    let basic = windows_machine(vec![GpuAdapter::software("Microsoft Basic Render Driver")]);
    let (code, text) = check(&env, &basic, &mut Runner::record().with_os(Os::Windows));
    assert_eq!(code, 1, "{text}");
    assert!(
        gpu_line(&text).contains("Microsoft Basic Render Driver"),
        "{text}"
    );
    assert!(text.contains("windowsupdate-optionalupdates"), "{text}");
    assert!(!has_hardware_adapter(&[GpuAdapter::software(
        "Microsoft Basic Render Driver"
    )]));
    assert!(has_hardware_adapter(&[
        GpuAdapter::software("Microsoft Basic Render Driver"),
        GpuAdapter::hardware(0x10DE, "NVIDIA", Some("1.0")),
    ]));
}

const LSPCI_NVIDIA: &str = "00:02.0 VGA compatible controller [0300]: Intel Corporation Alder Lake-P GT2 [Iris Xe Graphics] [8086:46a6] (rev 0c)\n01:00.0 3D controller [0302]: NVIDIA Corporation AD107M [GeForce RTX 4060 Max-Q / Mobile] [10de:28a0] (rev a1)\n00:1f.3 Audio device [0403]: Intel Corporation Alder Lake PCH-P High Definition Audio [8086:51c8]\n";

fn vulkan(devices: &[(&str, &str, &str, &str)]) -> String {
    let mut text = String::from("Devices:\n========\n");
    for (index, (vendor, name, kind, driver)) in devices.iter().enumerate() {
        text.push_str(&format!(
            "GPU{index}:\n\tapiVersion         = 1.3.274\n\tdriverVersion      = 1.0.0\n\tvendorID           = {vendor}\n\tdeviceID           = 0x0001\n\tdeviceType         = {kind}\n\tdeviceName         = {name}\n\tdriverID           = DRIVER_ID_X\n\tdriverName         = x\n\tdriverInfo         = {driver}\n"
        ));
    }
    text
}

fn ubuntu_runner(lspci: &'static str, vulkaninfo: String) -> Runner {
    Runner::record()
        .with_os(Os::Linux)
        .with_root(false)
        .responder(move |spec| match spec.program.as_str() {
            "lspci" => Some(Outcome::success().with_stdout(lspci)),
            "vulkaninfo" => Some(Outcome::success().with_stdout(vulkan_text(&vulkaninfo))),
            _ => None,
        })
}

fn vulkan_text(text: &str) -> String {
    text.to_string()
}

#[test]
fn ubuntu_with_nvidia_installs_the_driver_through_ubuntu_drivers_unless_it_is_active() {
    let env = env(Os::Linux, Arch::X64, false);
    let machine = FakeMachine::new().with_home("/home/dev");

    // Mesa's lavapipe (a CPU device) is all Vulkan sees: the NVIDIA driver is not active.
    let inactive = vulkan(&[(
        "0x10005",
        "llvmpipe (LLVM 17)",
        "PHYSICAL_DEVICE_TYPE_CPU",
        "Mesa 24",
    )]);
    let (code, text) = check(&env, &machine, &mut ubuntu_runner(LSPCI_NVIDIA, inactive));
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("sudo ubuntu-drivers install"), "{text}");
    assert!(
        gpu_line(&text).contains("NVIDIA Corporation AD107M"),
        "{text}"
    );

    // With the NVIDIA device active, and Intel on Mesa, nothing is missing.
    let active = vulkan(&[
        (
            "0x10de",
            "NVIDIA GeForce RTX 4060",
            "PHYSICAL_DEVICE_TYPE_DISCRETE_GPU",
            "560.35.03",
        ),
        (
            "0x8086",
            "Intel(R) Graphics (ADL GT2)",
            "PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU",
            "Mesa 24.0.9",
        ),
    ]);
    let (_, text) = check(&env, &machine, &mut ubuntu_runner(LSPCI_NVIDIA, active));
    assert!(gpu_line(&text).starts_with("[ok     ]"), "{text}");
    assert!(!text.contains("ubuntu-drivers"), "{text}");
}

#[test]
fn ubuntu_with_amd_or_intel_relies_on_mesa_and_vulkan_tools_for_the_probe() {
    let env = env(Os::Linux, Arch::X64, false);
    let machine = FakeMachine::new().with_home("/home/dev");
    let lspci = "06:00.0 VGA compatible controller [0300]: Advanced Micro Devices, Inc. [AMD/ATI] Navi 32 [Radeon RX 7700 XT / 7800 XT] [1002:747e] (rev c8)\n";
    let radv = vulkan(&[(
        "0x1002",
        "AMD Radeon RX 7800 XT (RADV NAVI32)",
        "PHYSICAL_DEVICE_TYPE_DISCRETE_GPU",
        "Mesa 24.0.9",
    )]);
    let (code, text) = check(&env, &machine, &mut ubuntu_runner(lspci, radv));
    assert_eq!(
        code, 1,
        "vulkan-tools is not installed in this fixture:\n{text}"
    );
    assert!(
        text.contains("[missing] gpu: vulkaninfo (vulkan-tools)"),
        "{text}"
    );
    assert!(gpu_line(&text).starts_with("[ok     ]"), "{text}");
    assert!(
        !text.contains("ubuntu-drivers"),
        "AMD needs no vendor driver:\n{text}"
    );
}

#[test]
fn under_wsl_the_windows_hosts_adapters_are_reported_and_fixed_on_windows() {
    let env = env(Os::Linux, Arch::X64, true);
    let machine = FakeMachine::new().with_home("/home/dev");
    let json = r#"[{"Name":"NVIDIA GeForce RTX 4070","PNPDeviceID":"PCI\\VEN_10DE&DEV_2786&SUBSYS_1","DriverVersion":null,"DriverDate":null},{"Name":"Microsoft Basic Render Driver","PNPDeviceID":"ROOT\\BASIC","DriverVersion":"10.0","DriverDate":null}]"#;
    let mut runner = Runner::record()
        .with_os(Os::Linux)
        .with_root(false)
        .responder(move |spec| {
            (spec.program == "powershell.exe"
                && spec
                    .args
                    .iter()
                    .any(|a| a.contains("Win32_VideoController")))
            .then(|| Outcome::success().with_stdout(json))
        });
    let (code, text) = check(&env, &machine, &mut runner);
    assert_eq!(code, 1, "{text}");
    assert!(
        gpu_line(&text).contains("NVIDIA GeForce RTX 4070"),
        "{text}"
    );
    assert!(
        text.contains("powershell.exe"),
        "the fix runs on the Windows host:\n{text}"
    );
    assert!(
        !text.contains("lspci") && !text.contains("ubuntu-drivers"),
        "{text}"
    );
    assert!(
        !text.contains("vulkan-tools"),
        "the distribution has no hardware driver to probe:\n{text}"
    );
}

#[test]
fn macos_reports_the_metal_device_and_installs_nothing() {
    let env = env(Os::MacOs, Arch::Arm64, false);
    let json = r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M2 Pro","spdisplays_vendor":"sppci_vendor_Apple","spdisplays_mtlgpufamilysupport":"spdisplays_metal3"}]}"#;
    let mut runner = Runner::record().with_os(Os::MacOs).responder(move |spec| {
        (spec.program == "system_profiler").then(|| Outcome::success().with_stdout(json))
    });
    let (code, text) = check(
        &env,
        &FakeMachine::new().with_home("/Users/dev"),
        &mut runner,
    );
    assert_eq!(code, 0, "{text}");
    assert!(gpu_line(&text).contains("Apple M2 Pro"), "{text}");
    assert!(
        runner
            .specs()
            .iter()
            .all(|s| s.program == "system_profiler"),
        "only a query ran"
    );
}

#[test]
fn the_detection_parsers_read_real_tool_output() {
    let pci = parse_lspci(LSPCI_NVIDIA);
    assert_eq!(pci.len(), 2, "the audio device is not an adapter");
    assert_eq!(pci[0].vendor, Vendor::Intel);
    assert_eq!(
        pci[0].name,
        "Intel Corporation Alder Lake-P GT2 [Iris Xe Graphics]"
    );
    assert_eq!(pci[1].vendor, Vendor::Nvidia);
    assert_eq!(pci[1].vendor_id, 0x10DE);

    let devices = parse_vulkaninfo(&vulkan(&[
        (
            "0x10de",
            "NVIDIA GeForce RTX 4060",
            "PHYSICAL_DEVICE_TYPE_DISCRETE_GPU",
            "560.35.03",
        ),
        ("0x10005", "llvmpipe", "PHYSICAL_DEVICE_TYPE_CPU", "Mesa 24"),
    ]));
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].vendor_id, 0x10DE);
    assert_eq!(devices[0].driver_info.as_deref(), Some("560.35.03"));
    assert!(!devices[0].cpu && devices[1].cpu);

    let single = parse_windows_video_controllers(
        r#"{"Name":"Intel(R) UHD","PNPDeviceID":"PCI\\VEN_8086&DEV_9A49","DriverVersion":"31.0.101.4502","DriverDate":"2023-05-01"}"#,
    );
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].vendor, Vendor::Intel);
    assert_eq!(single[0].driver_version.as_deref(), Some("31.0.101.4502"));

    let mac = parse_system_profiler(
        r#"{"SPDisplaysDataType":[{"sppci_model":"AMD Radeon Pro 5500M","spdisplays_vendor":"sppci_vendor_amd"}]}"#,
    );
    assert_eq!(mac[0].vendor, Vendor::Amd);
}

#[test]
fn the_real_windows_host_enumerates_its_adapters_through_dxgi() {
    #[cfg(windows)]
    {
        let adapters = rayx_cli::setup::gpu::dxgi_adapters();
        // Every Windows machine has at least the Basic Render Driver (WARP).
        assert!(!adapters.is_empty(), "DXGI reported no adapters");
        assert!(adapters.iter().all(|a| !a.name.is_empty()));
    }
    #[cfg(not(windows))]
    assert!(rayx_cli::setup::gpu::dxgi_adapters().is_empty());
}

#[test]
fn a_linux_adapter_is_usable_only_when_vulkan_shows_a_hardware_device_of_its_vendor() {
    use rayx_cli::doctor::report_for;

    let env = env(Os::Linux, Arch::X64, false);
    let machine = FakeMachine::new().with_home("/home/dev");
    let report = |lspci: &'static str, vulkaninfo: String| {
        let mut runner = ubuntu_runner(lspci, vulkaninfo);
        let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
        report_for(&env, &mut runner, &machine, &mut path, None).to_json()["hardwareAdapter"]
            .clone()
    };
    // NVIDIA card without an active driver: only lavapipe renders.
    let lavapipe = vulkan(&[("0x10005", "llvmpipe", "PHYSICAL_DEVICE_TYPE_CPU", "Mesa 24")]);
    assert_eq!(report(LSPCI_NVIDIA, lavapipe.clone()), false);
    // A virtual display adapter (virtio) has no hardware driver either.
    let virtio = "00:02.0 VGA compatible controller [0300]: Red Hat, Inc. Virtio 1.0 GPU [1af4:1050] (rev 01)\n";
    assert_eq!(report(virtio, lavapipe), false);
    // With the NVIDIA device active it is usable.
    let active = vulkan(&[(
        "0x10de",
        "NVIDIA GeForce RTX 4060",
        "PHYSICAL_DEVICE_TYPE_DISCRETE_GPU",
        "560",
    )]);
    assert_eq!(report(LSPCI_NVIDIA, active), true);
}

#[test]
fn virtual_adapters_are_informational_and_pciutils_comes_with_the_probe() {
    let env = env(Os::Linux, Arch::X64, false);
    let machine = FakeMachine::new().with_home("/home/dev");
    let virtio = "00:02.0 VGA compatible controller [0300]: Red Hat, Inc. Virtio 1.0 GPU [1af4:1050] (rev 01)\n";
    let lavapipe = vulkan(&[("0x10005", "llvmpipe", "PHYSICAL_DEVICE_TYPE_CPU", "Mesa 24")]);
    let (_, text) = check(&env, &machine, &mut ubuntu_runner(virtio, lavapipe));
    assert!(
        gpu_line(&text).starts_with("[ok     ]"),
        "a VM display is not a missing driver:\n{text}"
    );
    assert!(
        text.contains("vulkaninfo (vulkan-tools) and lspci (pciutils)"),
        "{text}"
    );
    assert!(text.contains("vulkan-tools pciutils"), "{text}");
}

#[test]
fn under_wsl_the_adapter_needs_dev_dxg_to_count_as_usable() {
    use rayx_cli::doctor::report_for;

    let env = env(Os::Linux, Arch::X64, true);
    let json = r#"[{"Name":"NVIDIA GeForce RTX 4070","PNPDeviceID":"PCI\\VEN_10DE&DEV_2786","DriverVersion":"32.0.15.6094","DriverDate":null}]"#;
    let usable = |dxg: bool| {
        let mut machine = FakeMachine::new().with_home("/home/dev");
        if dxg {
            machine = machine.with_file("/dev/dxg");
        }
        let mut runner = Runner::record().with_os(Os::Linux).responder(move |spec| {
            (spec.program == "powershell.exe").then(|| Outcome::success().with_stdout(json))
        });
        let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
        report_for(&env, &mut runner, &machine, &mut path, None).to_json()["hardwareAdapter"]
            .clone()
    };
    assert_eq!(usable(true), true);
    assert_eq!(usable(false), false);
}
