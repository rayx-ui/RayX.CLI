//! The Android set enforces what the project pins: the NDK version and the default AVD. Another
//! NDK or AVD on the machine does not satisfy either; an NDK the user names through
//! `ANDROID_NDK_HOME`, the one variable nsh reads, does. Through `rayx app` the pinned NDK is
//! what nsh builds with unless `ANDROID_NDK_HOME` says otherwise.

use std::path::PathBuf;

use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::android::{avd_names, find_ndk};
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Probed, Set, check, plan};

const SDK: &str = "/home/dev/Android/Sdk";
const PINNED: &str = "28.0.12674087";
const UBUNTU: &str = "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";

fn env() -> PlanEnv {
    PlanEnv {
        host: HostFacts {
            os: Os::Linux,
            arch: Arch::X64,
            emulated: false,
            wsl: false,
            distro: Some(Distro::parse(UBUNTU)),
        },
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
    }
}

fn sdk_with_ndks(versions: &[&str]) -> FakeMachine {
    let sdk = PathBuf::from(SDK);
    versions.iter().fold(
        FakeMachine::new()
            .with_home("/home/dev")
            .with_dir(&sdk)
            .with_dir(sdk.join("ndk")),
        |machine, version| machine.with_dir(sdk.join("ndk").join(version)),
    )
}

fn found_ndk(machine: &FakeMachine) -> Option<PathBuf> {
    let env = env();
    let mut runner = Runner::record().with_os(Os::Linux).with_root(false);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let cx = Cx::new(&env, &mut runner, machine, &mut path);
    find_ndk(&cx, Some(&PathBuf::from(SDK)))
}

/// The probe result of the step `id`.
fn probed(machine: &FakeMachine, id: &str) -> Probed {
    let env = env();
    let plan = plan(&env, &[Set::Android]).expect("plan");
    let mut runner = Runner::record().with_os(Os::Linux).with_root(false);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = Cx::new(&env, &mut runner, machine, &mut path);
    let checked = check(&plan, &mut cx);
    let index = plan
        .steps
        .iter()
        .position(|step| step.id == id)
        .unwrap_or_else(|| panic!("step {id}"));
    checked.probed[index].clone()
}

/// Whether the NDK package is among the packages the Android set would install.
fn ndk_is_missing(machine: &FakeMachine) -> bool {
    let env = env();
    let plan = plan(&env, &[Set::Android]).expect("plan");
    let mut runner = Runner::record().with_os(Os::Linux).with_root(false);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = Cx::new(&env, &mut runner, machine, &mut path);
    let checked = check(&plan, &mut cx);
    let index = plan
        .steps
        .iter()
        .position(|step| step.id == "android-packages")
        .expect("packages step");
    checked.probed[index]
        .missing
        .iter()
        .any(|package| package == &format!("ndk;{PINNED}"))
}

#[test]
fn the_pinned_ndk_is_installed_beside_an_ndk_of_another_version() {
    let other = sdk_with_ndks(&["27.2.12479018"]);

    assert!(ndk_is_missing(&other));
    assert!(!ndk_is_missing(&sdk_with_ndks(&["27.2.12479018", PINNED])));
}

#[test]
fn a_build_uses_the_pinned_ndk_when_it_is_installed() {
    let machine = sdk_with_ndks(&["27.2.12479018", PINNED, "29.0.1"]);

    assert_eq!(
        found_ndk(&machine),
        Some(PathBuf::from(SDK).join("ndk").join(PINNED))
    );
}

#[test]
fn without_the_pinned_ndk_the_newest_installed_one_is_reported() {
    let machine = sdk_with_ndks(&["26.1.10909125", "27.2.12479018"]);

    assert_eq!(
        found_ndk(&machine),
        Some(PathBuf::from(SDK).join("ndk").join("27.2.12479018"))
    );
}

#[test]
fn an_ndk_named_in_the_environment_wins_and_satisfies_the_pin() {
    let machine = sdk_with_ndks(&["27.2.12479018"])
        .with_env("ANDROID_NDK_HOME", "/opt/ndk")
        .with_dir("/opt/ndk");

    assert_eq!(found_ndk(&machine), Some(PathBuf::from("/opt/ndk")));
    assert!(!ndk_is_missing(&machine));
}

#[test]
fn only_android_ndk_home_names_an_ndk_as_it_does_for_nsh() {
    // nsh reads `ANDROID_NDK_HOME` only: the other spellings choose nothing for it.
    for name in ["ANDROID_NDK_ROOT", "NDK_HOME"] {
        let machine = sdk_with_ndks(&["27.2.12479018"])
            .with_env(name, "/opt/ndk")
            .with_dir("/opt/ndk");
        assert!(ndk_is_missing(&machine), "{name} does not satisfy the pin");
        assert_eq!(
            found_ndk(&machine),
            Some(PathBuf::from(SDK).join("ndk").join("27.2.12479018")),
            "{name}: nsh would build with the newest NDK of the SDK"
        );
    }
}

#[test]
fn an_android_ndk_home_that_names_no_directory_is_what_nsh_uses_and_is_reported() {
    let machine = sdk_with_ndks(&[PINNED]).with_env("ANDROID_NDK_HOME", "/gone/ndk");

    assert_eq!(found_ndk(&machine), Some(PathBuf::from("/gone/ndk")));
    let step = probed(&machine, "android-ndk-home");
    assert!(!step.satisfied);
    assert_eq!(
        step.detail,
        "ANDROID_NDK_HOME is /gone/ndk, which is no directory"
    );

    // Unset, or naming an NDK, it is fine.
    assert!(probed(&sdk_with_ndks(&[PINNED]), "android-ndk-home").satisfied);
    let named = sdk_with_ndks(&[PINNED])
        .with_env("ANDROID_NDK_HOME", "/opt/ndk")
        .with_dir("/opt/ndk");
    let step = probed(&named, "android-ndk-home");
    assert!(step.satisfied);
    assert_eq!(step.found.as_deref(), Some("/opt/ndk"));
}

#[test]
fn a_broken_android_ndk_home_stops_setup_with_the_fix() {
    use rayx_cli::cli::SetupArgs;
    use rayx_cli::setup::run_plan;

    let env = env();
    let plan = plan(&env, &[Set::Android]).expect("plan");
    let machine = sdk_with_ndks(&[PINNED]).with_env("ANDROID_NDK_HOME", "/gone/ndk");
    let mut runner = Runner::record().with_os(Os::Linux).with_root(false);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    let mut out = Vec::new();
    let checked = run_plan(
        &SetupArgs {
            check: true,
            ..SetupArgs::default()
        },
        &plan,
        &mut cx,
        &mut out,
    );
    let text = String::from_utf8(out).expect("UTF-8");
    assert_eq!(checked, 1, "{text}");
    assert!(
        text.contains("[missing] android: ANDROID_NDK_HOME, when set, names an NDK"),
        "{text}"
    );
    assert!(
        text.contains("unset it, so `rayx app` uses the pinned NDK 28.0.12674087"),
        "{text}"
    );
}

#[test]
fn avd_names_skip_the_emulators_log_lines() {
    let output = "INFO    | Storing crashdata\nPixel_9\nWARNING | x\nrayx-android-35-x86_64\n";

    assert_eq!(
        avd_names(output).collect::<Vec<_>>(),
        ["Pixel_9", "rayx-android-35-x86_64"]
    );
}

#[test]
fn a_probe_that_finds_only_other_avds_names_them() {
    let env = env();
    let machine = sdk_with_ndks(&[PINNED])
        .with_file(format!("{SDK}/emulator/emulator"))
        .with_file(format!("{SDK}/platform-tools/adb"));
    let plan = plan(&env, &[Set::Android]).expect("plan");
    let mut runner = Runner::record()
        .with_os(Os::Linux)
        .with_root(false)
        .responder(|spec| {
            spec.program
                .ends_with("emulator")
                .then(|| Outcome::success().with_stdout("Pixel_9_Pro_XL\n"))
        });
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    let checked = check(&plan, &mut cx);
    let index = plan
        .steps
        .iter()
        .position(|step| step.id == "android-avd")
        .expect("avd step");
    let probed = &checked.probed[index];

    assert!(!probed.satisfied);
    assert!(probed.detail.contains("Pixel_9_Pro_XL"), "{probed:?}");
}
