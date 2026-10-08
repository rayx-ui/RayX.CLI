//! The Android workstation set: JDK and JAVA_HOME, Android Studio, the SDK and its packages, the
//! default AVD, emulator acceleration, cargo-ndk and the Rust targets.
//!
//! Unix output is compared with forward slashes and without shell quotes, so these tests mean the
//! same on every host that runs them.

use std::path::{Path, PathBuf};

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::{Pin, PinSource, Pins};
use rayx_cli::setup::android::{avd_name, find_ndk, find_sdk, host_abi};
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, plan, run_plan};

const UBUNTU: &str = "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";

fn env(os: Os, arch: Arch, wsl: bool, yes: bool) -> PlanEnv {
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
        yes,
    }
}

fn p(parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(PathBuf::new(), |acc, part| acc.join(part))
}

fn home(os: Os) -> PathBuf {
    match os {
        Os::Windows => p(&[r"C:\Users", "dev"]),
        Os::MacOs => p(&["/Users", "dev"]),
        Os::Linux => p(&["/home", "dev"]),
    }
}

fn sdk_dir(os: Os) -> PathBuf {
    match os {
        Os::Windows => p(&[r"C:\Users\dev\AppData\Local", "Android", "Sdk"]),
        Os::MacOs => home(os).join("Library").join("Android").join("sdk"),
        Os::Linux => home(os).join("Android").join("Sdk"),
    }
}

fn machine(os: Os) -> FakeMachine {
    FakeMachine::new()
        .with_home(home(os))
        .with_env("USER", "dev")
        .with_env("ProgramFiles", r"C:\Program Files")
        .with_env("LOCALAPPDATA", r"C:\Users\dev\AppData\Local")
        .with_env("TEMP", r"C:\Temp")
}

fn exe(os: Os, name: &str) -> String {
    if os == Os::Windows {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn bat(os: Os, name: &str) -> String {
    if os == Os::Windows {
        format!("{name}.bat")
    } else {
        name.to_string()
    }
}

/// A machine whose SDK has the command-line tools and the listed packages.
fn with_sdk(mut machine: FakeMachine, os: Os, packages: &[&str]) -> FakeMachine {
    let sdk = sdk_dir(os);
    for tool in ["sdkmanager", "avdmanager"] {
        machine = machine.with_file(
            sdk.join("cmdline-tools")
                .join("latest")
                .join("bin")
                .join(bat(os, tool)),
        );
    }
    for package in packages {
        machine = match *package {
            "platform-tools" => machine.with_file(sdk.join("platform-tools").join(exe(os, "adb"))),
            "platform" => {
                machine.with_file(sdk.join("platforms").join("android-34").join("android.jar"))
            }
            "ndk" => machine.with_dir(sdk.join("ndk").join("28.0.12674087")),
            "emulator" => machine.with_file(sdk.join("emulator").join(exe(os, "emulator"))),
            "image" => machine.with_dir(
                sdk.join("system-images")
                    .join("android-34")
                    .join("google_apis")
                    .join("x86_64"),
            ),
            other => panic!("unknown package {other}"),
        };
    }
    machine
}

fn runner(os: Os) -> Runner {
    Runner::record().with_os(os).with_root(false)
}

fn run_check(
    env: &PlanEnv,
    machine: &FakeMachine,
    runner: &mut Runner,
    path: &mut UserPath,
) -> (u8, String) {
    let plan = plan(env, &[Set::Android]).expect("plan");
    let mut out = Vec::new();
    let mut cx = Cx::new(env, runner, machine, path);
    let args = SetupArgs {
        check: true,
        android: true,
        ..SetupArgs::default()
    };
    let code = run_plan(&args, &plan, &mut cx, &mut out);
    let text = String::from_utf8(out).expect("UTF-8");
    let text = if env.host.os == Os::Windows {
        text
    } else {
        text.replace('\\', "/").replace('\'', "")
    };
    (code, text)
}

fn check(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> (u8, String) {
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    run_check(env, machine, runner, &mut path)
}

fn fwd(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// The line of the report for the step whose title contains `needle`.
fn status<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines()
        .find(|l| l.starts_with('[') && l.contains(needle))
        .unwrap_or_else(|| panic!("no step {needle:?}:\n{text}"))
}

const JDK21_RELEASE: &str = "JAVA_VERSION=\"21.0.12\"\nIMPLEMENTOR=\"Microsoft\"\n";

#[test]
fn the_jdk_is_21_per_os_and_java_home_points_at_it() {
    let (_, linux) = check(
        &env(Os::Linux, Arch::X64, false, false),
        &machine(Os::Linux),
        &mut runner(Os::Linux),
    );
    assert!(linux.contains("openjdk-21-jdk"), "{linux}");

    let (_, windows) = check(
        &env(Os::Windows, Arch::X64, false, false),
        &machine(Os::Windows),
        &mut runner(Os::Windows),
    );
    assert!(
        windows.contains("winget install --id Microsoft.OpenJDK.21 --exact"),
        "{windows}"
    );

    let (_, mac) = check(
        &env(Os::MacOs, Arch::Arm64, false, false),
        &machine(Os::MacOs).with_file("/opt/homebrew/bin/brew"),
        &mut runner(Os::MacOs),
    );
    assert!(
        mac.contains("brew install --cask microsoft-openjdk@21"),
        "{mac}"
    );
}

#[test]
fn java_home_is_set_for_the_user_once_a_jdk_21_exists() {
    let os = Os::Windows;
    let jdk = p(&[r"C:\Program Files", "Microsoft", "jdk-21.0.12.7-hotspot"]);
    let machine = machine(os).with_text(jdk.join("release"), JDK21_RELEASE);
    let env = env(os, Arch::X64, false, false);
    let (_, text) = check(&env, &machine, &mut runner(os));
    assert!(status(&text, "JDK 21").starts_with("[ok     ]"), "{text}");
    assert!(
        status(&text, "JAVA_HOME").starts_with("[missing]"),
        "{text}"
    );
    assert!(
        text.contains(&format!("set JAVA_HOME to {} for the user", jdk.display())),
        "{text}"
    );

    // Once JAVA_HOME names a JDK 21 the step is met; a JDK 17 does not count.
    let mut path = UserPath::Windows(Box::new(
        MemoryPathStore::new(None).with_user_var("JAVA_HOME", &jdk.display().to_string()),
    ));
    let (_, text) = run_check(&env, &machine, &mut runner(os), &mut path);
    assert!(
        status(&text, "JAVA_HOME").starts_with("[ok     ]"),
        "{text}"
    );

    let jdk17 = p(&[r"C:\Program Files", "Microsoft", "jdk-17"]);
    let old = machine.with_text(jdk17.join("release"), "JAVA_VERSION=\"17.0.9\"\n");
    let mut path = UserPath::Windows(Box::new(
        MemoryPathStore::new(None).with_user_var("JAVA_HOME", &jdk17.display().to_string()),
    ));
    let (_, text) = run_check(&env, &old, &mut runner(os), &mut path);
    assert!(
        status(&text, "JAVA_HOME").starts_with("[missing]"),
        "{text}"
    );
}

#[test]
fn android_studio_is_installed_on_desktops_and_skipped_under_wsl() {
    let (_, windows) = check(
        &env(Os::Windows, Arch::X64, false, false),
        &machine(Os::Windows),
        &mut runner(Os::Windows),
    );
    assert!(
        windows.contains("winget install --id Google.AndroidStudio --exact --interactive"),
        "{windows}"
    );

    let (_, mac) = check(
        &env(Os::MacOs, Arch::Arm64, false, false),
        &machine(Os::MacOs).with_file("/opt/homebrew/bin/brew"),
        &mut runner(Os::MacOs),
    );
    assert!(mac.contains("brew install --cask android-studio"), "{mac}");

    let desktop = machine(Os::Linux).with_env("XDG_CURRENT_DESKTOP", "ubuntu:GNOME");
    let (_, ubuntu) = check(
        &env(Os::Linux, Arch::X64, false, false),
        &desktop,
        &mut runner(Os::Linux),
    );
    assert!(
        ubuntu.contains("sudo snap install android-studio --classic"),
        "{ubuntu}"
    );

    let (_, headless) = check(
        &env(Os::Linux, Arch::X64, false, false),
        &machine(Os::Linux),
        &mut runner(Os::Linux),
    );
    assert!(
        status(&headless, "Android Studio").contains("no desktop session"),
        "{headless}"
    );

    let (_, wsl) = check(
        &env(Os::Linux, Arch::X64, true, false),
        &desktop,
        &mut runner(Os::Linux),
    );
    let line = status(&wsl, "Android Studio");
    assert!(line.starts_with("[ok     ]"), "{line}");
    assert!(!wsl.contains("snap install"), "{wsl}");
}

#[test]
fn the_sdk_is_found_in_xtask_order_and_missing_command_line_tools_are_downloaded() {
    let os = Os::Linux;
    let env = env(os, Arch::X64, false, false);

    // ANDROID_HOME wins over the platform default.
    let custom = p(&["/opt", "android-sdk"]);
    let machine = machine(os)
        .with_env("ANDROID_HOME", &fwd(&custom))
        .with_dir(&custom)
        .with_dir(sdk_dir(os));
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut rec = runner(os);
    let cx = Cx::new(&env, &mut rec, &machine, &mut path);
    assert_eq!(find_sdk(&cx), Some(custom));
    let default_only = machine_without_env(os).with_dir(sdk_dir(os));
    let cx = Cx::new(&env, &mut rec, &default_only, &mut path);
    assert_eq!(find_sdk(&cx), Some(sdk_dir(os)));

    // No SDK at all: the command-line tools are downloaded into the default location.
    let (code, text) = check(&env, &machine_without_env(os), &mut runner(os));
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains(
            "curl -fL --proto =https -o /tmp/rayx-commandlinetools.zip \
             https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip"
        ),
        "{text}"
    );
    assert!(
        text.contains("unzip -q -o /tmp/rayx-commandlinetools.zip -d"),
        "{text}"
    );
    assert!(text.contains("cmdline-tools/latest"), "{text}");
    assert!(
        text.contains(&format!(
            "set ANDROID_HOME to {} for the user",
            fwd(&sdk_dir(os))
        )),
        "{text}"
    );
}

fn machine_without_env(os: Os) -> FakeMachine {
    machine(os)
}

#[test]
fn sdkmanager_installs_the_pinned_packages_in_one_command_with_licenses() {
    let os = Os::Linux;
    let machine = with_sdk(machine(os), os, &[]).with_dir(sdk_dir(os));
    let env = env(os, Arch::X64, false, true);
    let (_, text) = check(&env, &machine, &mut runner(os));

    let sdk = fwd(&sdk_dir(os));
    let manager = format!("{sdk}/cmdline-tools/latest/bin/sdkmanager");
    assert!(
        text.contains(&format!(
            "{manager} --sdk_root={sdk} --install platform-tools platforms;android-34 \
             ndk;28.0.12674087 emulator system-images;android-34;google_apis;x86_64"
        )),
        "{text}"
    );
    assert!(
        !text.contains("build-tools;"),
        "build-tools are installed only when pinned:\n{text}"
    );
    // --yes answers the license prompts.
    assert!(
        text.contains(&format!(
            "{manager} --sdk_root={sdk} --licenses < (answers: y ...)"
        )),
        "{text}"
    );

    // Without --yes the prompt reaches the user.
    let env = crate::env(os, Arch::X64, false, false);
    let (_, text) = check(&env, &machine, &mut runner(os));
    assert!(
        text.lines()
            .any(|l| l.ends_with("--licenses") && !l.contains("answers")),
        "{text}"
    );
}

#[test]
fn pinned_build_tools_the_pinned_ndk_and_the_host_architecture_shape_the_package_list() {
    let os = Os::MacOs;
    let mut env = env(os, Arch::Arm64, false, false);
    env.pins.android_build_tools = Some(Pin {
        value: "36.0.0".into(),
        source: PinSource::GpuxCheckout,
    });
    let machine = with_sdk(
        machine(os).with_file("/opt/homebrew/bin/brew"),
        os,
        &["platform-tools"],
    )
    // An NDK that is not the pinned one does not stand in for it.
    .with_dir(sdk_dir(os).join("ndk").join("27.1.12297006"));
    let (_, text) = check(&env, &machine, &mut runner(os));
    assert!(text.contains("build-tools;36.0.0"), "{text}");
    assert!(
        text.contains("ndk;28.0.12674087"),
        "the pinned NDK is installed next to another one:\n{text}"
    );
    assert!(
        text.contains("system-images;android-34;google_apis;arm64-v8a"),
        "{text}"
    );
    assert!(text.contains("platforms;android-34"), "{text}");
    assert!(
        !text.contains("platform-tools "),
        "platform-tools is present:\n{text}"
    );

    // NDK_HOME style overrides are honoured too.
    let env2 = crate::env(os, Arch::X64, false, false);
    let ndk = p(&["/opt", "ndk"]);
    let with_env_ndk = with_sdk(machine_without_env(os), os, &[])
        .with_env("ANDROID_NDK_ROOT", &fwd(&ndk))
        .with_dir(&ndk)
        .with_dir(sdk_dir(os));
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut rec = runner(os);
    let cx = Cx::new(&env2, &mut rec, &with_env_ndk, &mut path);
    assert_eq!(find_ndk(&cx, Some(&sdk_dir(os))), Some(ndk));
    assert_eq!(host_abi(Arch::X64), "x86_64");
    assert_eq!(host_abi(Arch::Arm64), "arm64-v8a");
}

#[test]
fn a_default_avd_is_created_unless_it_already_exists() {
    let os = Os::Linux;
    let env = env(os, Arch::X64, false, false);
    let packages = &["platform-tools", "platform", "ndk", "emulator", "image"];
    let machine = with_sdk(machine(os), os, packages).with_dir(sdk_dir(os));
    assert_eq!(avd_name(&env), "rayx-android-34-x86_64");

    let none =
        runner(os).responder(|spec| spec.program.ends_with("emulator").then(Outcome::success));
    let (_, text) = check(&env, &machine, &mut { none });
    assert!(
        status(&text, "default AVD").starts_with("[missing]"),
        "{text}"
    );
    let sdk = fwd(&sdk_dir(os));
    assert!(
        text.contains(&format!(
            "{sdk}/cmdline-tools/latest/bin/avdmanager create avd --name rayx-android-34-x86_64 \
             --package system-images;android-34;google_apis;x86_64 < (answers: no)"
        )),
        "{text}"
    );

    // Another AVD does not stand in for the default one.
    let other = runner(os).responder(|spec| {
        spec.program
            .ends_with("emulator")
            .then(|| Outcome::success().with_stdout("Pixel_6_API_34\n"))
    });
    let (_, text) = check(&env, &machine, &mut { other });
    assert!(
        status(&text, "default AVD").starts_with("[missing]"),
        "{text}"
    );

    let default = runner(os).responder(|spec| {
        spec.program
            .ends_with("emulator")
            .then(|| Outcome::success().with_stdout("Pixel_6_API_34\nrayx-android-34-x86_64\n"))
    });
    let (_, text) = check(&env, &machine, &mut { default });
    assert!(
        status(&text, "default AVD").starts_with("[ok     ]"),
        "{text}"
    );
}

#[test]
fn acceleration_is_kvm_on_linux_the_hypervisor_platform_on_windows_and_nothing_on_macos() {
    // Linux: /dev/kvm exists but the user is not in the kvm group.
    let linux = env(Os::Linux, Arch::X64, false, false);
    let kvm = machine(Os::Linux).with_file("/dev/kvm");
    let groups = |names: &'static str| {
        runner(Os::Linux).responder(move |spec| {
            (spec.program == "id").then(|| Outcome::success().with_stdout(names))
        })
    };
    let (_, text) = check(&linux, &kvm, &mut groups("dev adm sudo\n"));
    assert!(
        status(&text, "emulator acceleration").starts_with("[missing]"),
        "{text}"
    );
    assert!(text.contains("sudo usermod -aG kvm dev"), "{text}");
    let (_, text) = check(&linux, &kvm, &mut groups("dev adm kvm\n"));
    assert!(
        status(&text, "emulator acceleration").starts_with("[ok     ]"),
        "{text}"
    );

    // Linux without /dev/kvm: the fix is a BIOS setting, so the install explains it.
    let (_, text) = check(&linux, &machine(Os::Linux), &mut runner(Os::Linux));
    assert!(text.contains("/dev/kvm does not exist"), "{text}");
    assert!(
        text.contains("cannot install: /dev/kvm does not exist"),
        "{text}"
    );

    // Windows: an elevated dism enables the Windows Hypervisor Platform; the emulator reports it.
    let windows = env(Os::Windows, Arch::X64, false, false);
    let sdk = sdk_dir(Os::Windows);
    let win_machine = with_sdk(machine(Os::Windows), Os::Windows, &["emulator"]).with_dir(&sdk);
    let (_, text) = check(&windows, &win_machine, &mut runner(Os::Windows));
    assert!(
        text.contains(
            "[admin] dism /online /Enable-Feature /FeatureName:HypervisorPlatform /All /NoRestart"
        ),
        "{text}"
    );
    let usable = runner(Os::Windows).responder(|spec| {
        spec.args
            .iter()
            .any(|a| a == "-accel-check")
            .then(|| Outcome::success().with_stdout("WHPX (10.0.26100) is installed and usable.\n"))
    });
    let (_, text) = check(&windows, &win_machine, &mut { usable });
    assert!(
        status(&text, "emulator acceleration").starts_with("[ok     ]"),
        "{text}"
    );

    // macOS needs nothing; WSL uses the Windows host's emulator.
    let (_, text) = check(
        &env(Os::MacOs, Arch::Arm64, false, false),
        &machine(Os::MacOs),
        &mut runner(Os::MacOs),
    );
    assert!(
        status(&text, "emulator acceleration").starts_with("[ok     ]"),
        "{text}"
    );
    let (_, text) = check(
        &env(Os::Linux, Arch::X64, true, false),
        &machine(Os::Linux),
        &mut runner(Os::Linux),
    );
    assert!(
        status(&text, "emulator acceleration").contains("provided by the Windows host"),
        "{text}"
    );
    assert!(
        !text.contains("system-images"),
        "WSL builds only; the emulator lives on Windows:\n{text}"
    );
}

#[test]
fn cargo_ndk_and_the_android_rust_targets_follow_the_projects_toolchain() {
    let os = Os::Linux;
    let mut env = env(os, Arch::X64, false, false);
    env.pins.rust_toolchain = Some(Pin {
        value: "1.95.0".into(),
        source: PinSource::Project,
    });
    let no_ndk = runner(os)
        .responder(|spec| (spec.program.ends_with("cargo")).then(|| Outcome::failure(101)));
    let (_, text) = check(&env, &machine(os), &mut { no_ndk });
    assert!(text.contains("cargo install cargo-ndk --locked"), "{text}");
    assert!(
        text.contains(
            "rustup target add --toolchain 1.95.0 aarch64-linux-android x86_64-linux-android"
        ),
        "{text}"
    );

    // One target present: only the other is added.
    let partial = runner(os).responder(|spec| {
        (spec.program.ends_with("rustup") && spec.args.first().is_some_and(|a| a == "target")).then(
            || Outcome::success().with_stdout("aarch64-linux-android\nx86_64-unknown-linux-gnu\n"),
        )
    });
    let machine = machine(os).with_program("rustup", "/home/dev/.cargo/bin/rustup");
    let (_, text) = check(&env, &machine, &mut { partial });
    assert!(
        text.contains("target add --toolchain 1.95.0 x86_64-linux-android"),
        "{text}"
    );
    assert!(
        !text.contains("target add --toolchain 1.95.0 aarch64"),
        "{text}"
    );
}

#[test]
fn the_batch_tools_start_by_their_full_path_even_when_it_contains_a_space() {
    let os = Os::Windows;
    let sdk = p(&[r"C:\Users\First Last\AppData\Local", "Android", "Sdk"]);
    let manager = sdk
        .join("cmdline-tools")
        .join("latest")
        .join("bin")
        .join("sdkmanager.bat");
    let machine = machine(os)
        .with_env("ANDROID_HOME", &sdk.display().to_string())
        .with_dir(&sdk)
        .with_file(&manager);
    let (_, text) = check(&env(os, Arch::X64, false, false), &machine, &mut runner(os));
    assert!(!text.contains("cmd /c"), "no cmd /c wrapper:\n{text}");
    let line = text
        .lines()
        .find(|l| l.contains("sdkmanager.bat") && l.contains("--install"))
        .unwrap_or_else(|| panic!("install line:\n{text}"));
    assert!(
        line.trim_start()
            .contains(&format!("\"{}\" \"--sdk_root=", manager.display())),
        "the program is the quoted full path: {line}"
    );
}

#[test]
fn avdmanager_is_found_with_the_same_layout_search_as_sdkmanager() {
    use rayx_cli::setup::android::{find_cmdline_tool, find_sdkmanager};

    let os = Os::Linux;
    let sdk = sdk_dir(os);
    // Only a versioned cmdline-tools directory, no `latest`.
    let machine = machine(os)
        .with_dir(&sdk)
        .with_file(
            sdk.join("cmdline-tools")
                .join("11.0")
                .join("bin")
                .join("sdkmanager"),
        )
        .with_file(
            sdk.join("cmdline-tools")
                .join("11.0")
                .join("bin")
                .join("avdmanager"),
        );
    let env = env(os, Arch::X64, false, false);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut rec = runner(os);
    let cx = Cx::new(&env, &mut rec, &machine, &mut path);
    let expected = sdk
        .join("cmdline-tools")
        .join("11.0")
        .join("bin")
        .join("avdmanager");
    assert_eq!(find_cmdline_tool(&cx, &sdk, "avdmanager"), Some(expected));
    assert!(find_sdkmanager(&cx, &sdk).is_some());
}

#[test]
fn emulator_log_lines_are_not_avds() {
    use rayx_cli::setup::android::first_avd_name;

    assert_eq!(
        first_avd_name("INFO    | Storing crashdata in: /tmp/x\n"),
        None
    );
    assert_eq!(
        first_avd_name("WARNING | something\nERROR   | other\nDEBUG | more\n\n"),
        None
    );
    assert_eq!(
        first_avd_name("INFO    | Storing crashdata\nPixel_6_API_34\n"),
        Some("Pixel_6_API_34")
    );

    let os = Os::Linux;
    let env = env(os, Arch::X64, false, false);
    let packages = &["platform-tools", "platform", "ndk", "emulator", "image"];
    let machine = with_sdk(machine(os), os, packages).with_dir(sdk_dir(os));
    let log_only = runner(os).responder(|spec| {
        spec.program.ends_with("emulator").then(|| {
            Outcome::success().with_stdout("INFO    | Storing crashdata in: /tmp/android-dev\n")
        })
    });
    let (_, text) = check(&env, &machine, &mut { log_only });
    assert!(
        status(&text, "default AVD").starts_with("[missing]"),
        "{text}"
    );

    let with_avd = runner(os).responder(|spec| {
        spec.program.ends_with("emulator").then(|| {
            Outcome::success()
                .with_stdout("INFO    | Storing crashdata in: /tmp/x\nrayx-android-34-x86_64\n")
        })
    });
    let (_, text) = check(&env, &machine, &mut { with_avd });
    assert!(
        status(&text, "default AVD").starts_with("[ok     ]"),
        "{text}"
    );
}

#[test]
fn the_system_image_and_the_avd_follow_the_platform_pin() {
    use rayx_cli::project::PinSource;

    // The gpux profile pins android-36: the image and the AVD are android-36 too.
    let os = Os::Linux;
    let mut env = env(os, Arch::X64, false, false);
    env.pins.android_platform = Pin {
        value: "android-36".into(),
        source: PinSource::GpuxCheckout,
    };
    env.pins.android_system_image = Pin {
        value: "android-36;google_apis".into(),
        source: PinSource::GpuxCheckout,
    };
    let machine = with_sdk(machine(os), os, &[]).with_dir(sdk_dir(os));
    let (_, text) = check(&env, &machine, &mut runner(os));
    assert!(text.contains("platforms;android-36"), "{text}");
    assert!(
        text.contains("system-images;android-36;google_apis;x86_64"),
        "{text}"
    );
    assert!(!text.contains("android-34"), "{text}");
    assert_eq!(avd_name(&env), "rayx-android-36-x86_64");

    // The real resolution derives the image from the platform pin and reports its source.
    let resolved = Pins::resolve(None);
    assert_eq!(
        resolved.android_system_image.value,
        "android-34;google_apis"
    );
    assert_eq!(resolved.android_system_image.source, PinSource::Default);
}

#[test]
fn a_wrong_jdk_a_wrong_java_home_and_the_installed_ndk_report_their_versions() {
    let os = Os::Windows;
    let env = env(os, Arch::X64, false, false);
    let jdk17 = p(&[r"C:\Program Files", "Microsoft", "jdk-17.0.9-hotspot"]);
    let machine = machine(os).with_text(jdk17.join("release"), "JAVA_VERSION=\"17.0.9\"\n");
    let mut path = UserPath::Windows(Box::new(
        MemoryPathStore::new(None).with_user_var("JAVA_HOME", &jdk17.display().to_string()),
    ));
    let mut rec = runner(os);
    let plan = plan(&env, &[Set::Android]).expect("plan");
    let mut cx = Cx::new(&env, &mut rec, &machine, &mut path);
    let checked = rayx_cli::setup::check(&plan, &mut cx);
    let probe = |id: &str| {
        let index = plan.steps.iter().position(|s| s.id == id).expect("step");
        checked.probed[index].clone()
    };
    let jdk = probe("jdk");
    assert!(
        !jdk.satisfied && jdk.found.as_deref() == Some("17"),
        "{jdk:?}"
    );
    let java_home = probe("java-home");
    assert!(
        !java_home.satisfied && java_home.found.as_deref() == Some("17"),
        "{java_home:?}"
    );

    let report = rayx_cli::doctor::report_for(&env, &mut runner(os), &machine, &mut path, None);
    let requirements: Vec<_> = report.sets.iter().flat_map(|(_, items)| items).collect();
    let wrong = |id: &str| {
        requirements
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.status)
            .expect("requirement")
    };
    assert_eq!(wrong("jdk"), rayx_cli::doctor::Status::WrongVersion);
    assert_eq!(wrong("java-home"), rayx_cli::doctor::Status::WrongVersion);
}
