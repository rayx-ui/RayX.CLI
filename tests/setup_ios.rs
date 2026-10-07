//! The iOS set: Xcode through `xcodes` or `mas`, selection, license, first launch, the Simulator
//! runtime, XcodeGen and the iOS Rust targets; not applicable off macOS.

use std::path::PathBuf;

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::UserPath;
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::{Pin, PinSource, Pins};
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, plan, run_plan};

fn env(os: Os, yes: bool, xcode: Option<&str>) -> PlanEnv {
    let mut pins = Pins::resolve(None);
    pins.xcode = xcode.map(|version| Pin {
        value: version.to_string(),
        source: PinSource::Project,
    });
    PlanEnv {
        host: HostFacts {
            os,
            arch: Arch::Arm64,
            emulated: false,
            wsl: false,
            distro: (os == Os::Linux).then(|| {
                Distro::parse("ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu\"\n")
            }),
        },
        pins,
        project_root: None,
        tools_node: None,
        yes,
    }
}

fn mac(yes: bool, xcode: Option<&str>) -> PlanEnv {
    env(Os::MacOs, yes, xcode)
}

fn machine() -> FakeMachine {
    FakeMachine::new()
        .with_home("/Users/dev")
        .with_file("/opt/homebrew/bin/brew")
}

fn check(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> (u8, String) {
    let plan = plan(env, &[Set::Ios]).expect("plan");
    let mut path = UserPath::Profiles {
        files: vec![PathBuf::from("/Users/dev/.zprofile")],
        home: PathBuf::from("/Users/dev"),
    };
    let mut out = Vec::new();
    let mut cx = Cx::new(env, runner, machine, &mut path);
    let args = SetupArgs {
        check: true,
        ios: true,
        ..SetupArgs::default()
    };
    let code = run_plan(&args, &plan, &mut cx, &mut out);
    let text = String::from_utf8(out).expect("UTF-8");
    (code, text.replace('\\', "/").replace('\'', ""))
}

/// A Mac with the Command Line Tools only: nothing of the iOS set is present.
fn bare() -> Runner {
    Runner::record()
        .with_os(Os::MacOs)
        .with_root(false)
        .responder(|spec| match spec.program.as_str() {
            "xcodebuild" | "xcrun" => Some(Outcome::failure(1)),
            "xcode-select" => {
                Some(Outcome::success().with_stdout("/Library/Developer/CommandLineTools\n"))
            }
            _ => None,
        })
}

fn status<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines()
        .find(|l| l.starts_with('[') && l.contains(needle))
        .unwrap_or_else(|| panic!("no step {needle:?}:\n{text}"))
}

#[test]
fn an_unpinned_xcode_comes_from_xcodes_with_the_apple_id_prompt_in_the_console() {
    let (code, text) = check(&mac(false, None), &machine(), &mut bare());
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("[missing] ios: Xcode (newest stable)"),
        "{text}"
    );
    assert!(
        text.contains("/opt/homebrew/bin/brew install xcodesorg/made/xcodes"),
        "xcodes comes from Homebrew:\n{text}"
    );
    assert!(text.contains("xcodes install --latest"), "{text}");
    assert!(!text.contains("mas install"), "{text}");
}

#[test]
fn a_pinned_xcode_version_is_installed_and_an_installed_one_is_compared() {
    let env = mac(false, Some("26.1"));
    let machine = machine().with_program("xcodes", "/opt/homebrew/bin/xcodes");
    let (_, text) = check(&env, &machine, &mut bare());
    assert!(text.contains("[missing] ios: Xcode 26.1"), "{text}");
    assert!(text.contains("xcodes install 26.1"), "{text}");
    assert!(
        !text.contains("brew install xcodesorg"),
        "xcodes is already installed:\n{text}"
    );

    // The version comes from the app bundle, not from `xcodebuild`, which fails while the
    // Command Line Tools are selected or the license is pending (`bare()` fails it).
    let version = |installed: &'static str| {
        let machine = machine
            .clone()
            .with_text(plist_path("Xcode.app"), &plist(installed));
        check(&env, &machine, &mut bare()).1
    };
    assert!(status(&version("26.1"), "Xcode 26.1").starts_with("[ok     ]"));
    assert!(status(&version("26.1.1"), "Xcode 26.1").starts_with("[ok     ]"));
    let older = version("16.2");
    assert!(
        status(&older, "Xcode 26.1").contains("Xcode 16.2 is installed, the project pins 26.1"),
        "{older}"
    );
}

#[test]
fn the_mac_app_store_is_used_only_when_signed_in_and_no_version_is_pinned() {
    let machine = machine().with_program("mas", "/opt/homebrew/bin/mas");
    let signed_in = || {
        bare().responder(|spec| {
            (spec.program == "mas" && spec.args == ["account"])
                .then(|| Outcome::success().with_stdout("dev@example.com\n"))
        })
    };
    let (_, text) = check(&mac(false, None), &machine, &mut signed_in());
    assert!(text.contains("mas install 497799835"), "{text}");
    assert!(!text.contains("xcodes install"), "{text}");

    // A pinned version needs xcodes: the App Store only has the newest Xcode.
    let (_, text) = check(&mac(false, Some("26.1")), &machine, &mut signed_in());
    assert!(text.contains("xcodes install 26.1"), "{text}");
    assert!(!text.contains("mas install"), "{text}");

    // Signed out: xcodes again.
    let signed_out = bare().responder(|spec| (spec.program == "mas").then(|| Outcome::failure(1)));
    let (_, text) = check(&mac(false, None), &machine, &mut { signed_out });
    assert!(text.contains("xcodes install --latest"), "{text}");
}

#[test]
fn xcode_select_is_switched_to_the_newest_xcode_app() {
    let machine = machine()
        .with_text(plist_path("Xcode-16.2.app"), &plist("16.2"))
        .with_text(plist_path("Xcode.app"), &plist("26.1"));
    let (_, text) = check(&mac(false, None), &machine, &mut bare());
    assert!(
        text.contains("sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer"),
        "{text}"
    );
    assert!(
        status(&text, "xcode-select points at Xcode")
            .contains("selected: /Library/Developer/CommandLineTools"),
        "{text}"
    );

    let mut selected = bare().responder(|spec| {
        (spec.program == "xcode-select")
            .then(|| Outcome::success().with_stdout("/Applications/Xcode.app/Contents/Developer\n"))
    });
    let (_, text) = check(&mac(false, None), &machine, &mut selected);
    assert!(
        status(&text, "xcode-select points at Xcode").starts_with("[ok     ]"),
        "{text}"
    );
}

#[test]
fn the_license_is_accepted_non_interactively_only_with_yes() {
    let (_, with_yes) = check(&mac(true, None), &machine(), &mut bare());
    assert!(
        with_yes.contains("sudo xcodebuild -license accept"),
        "{with_yes}"
    );

    let (_, without) = check(&mac(false, None), &machine(), &mut bare());
    assert!(
        without.contains("sudo xcodebuild -license\n"),
        "the prompt reaches the user:\n{without}"
    );
    assert!(!without.contains("-license accept"), "{without}");

    let accepted = bare().responder(|spec| {
        (spec.program == "xcodebuild" && spec.args == ["-license", "check"]).then(Outcome::success)
    });
    let (_, text) = check(&mac(false, None), &machine(), &mut { accepted });
    assert!(
        status(&text, "Xcode license").starts_with("[ok     ]"),
        "{text}"
    );
}

#[test]
fn first_launch_runs_when_it_has_not() {
    let (_, text) = check(&mac(false, None), &machine(), &mut bare());
    assert!(text.contains("sudo xcodebuild -runFirstLaunch"), "{text}");
    let done = bare().responder(|spec| {
        (spec.program == "xcodebuild" && spec.args == ["-checkFirstLaunchStatus"])
            .then(Outcome::success)
    });
    let (_, text) = check(&mac(false, None), &machine(), &mut { done });
    assert!(
        status(&text, "first-launch").starts_with("[ok     ]"),
        "{text}"
    );
}

#[test]
fn the_simulator_runtime_is_downloaded_only_when_none_is_listed() {
    let (_, text) = check(&mac(false, None), &machine(), &mut bare());
    assert!(text.contains("xcodebuild -downloadPlatform iOS"), "{text}");

    let listed = bare().responder(|spec| {
        (spec.program == "xcrun").then(|| {
            Outcome::success().with_stdout(
                "== Runtimes ==\niOS 18.2 (18.2 - 22C150) - com.apple.CoreSimulator.SimRuntime.iOS-18-2\n",
            )
        })
    });
    let (_, text) = check(&mac(false, None), &machine(), &mut { listed });
    assert!(
        status(&text, "iOS Simulator runtime").starts_with("[ok     ]"),
        "{text}"
    );
}

#[test]
fn xcodegen_and_the_ios_rust_targets_complete_the_set() {
    let (_, text) = check(&mac(false, None), &machine(), &mut bare());
    assert!(
        text.contains("/opt/homebrew/bin/brew install xcodegen"),
        "{text}"
    );

    let mut env = mac(false, None);
    env.pins.rust_toolchain = Some(Pin {
        value: "1.95.0".into(),
        source: PinSource::Project,
    });
    let (_, text) = check(&env, &machine(), &mut bare());
    // rustup is not scripted, so every target is missing and both are added together.
    assert!(
        text.contains(
            "rustup target add --toolchain 1.95.0 aarch64-apple-ios aarch64-apple-ios-sim"
        ),
        "{text}"
    );

    let partial = bare().responder(|spec| {
        (spec.program.ends_with("rustup") && spec.args.first().is_some_and(|a| a == "target"))
            .then(|| Outcome::success().with_stdout("aarch64-apple-ios\naarch64-apple-darwin\n"))
    });
    let (_, text) = check(
        &env,
        &machine().with_program("rustup", "/Users/dev/.cargo/bin/rustup"),
        &mut { partial },
    );
    assert!(
        text.contains("target add --toolchain 1.95.0 aarch64-apple-ios-sim"),
        "{text}"
    );
}

#[test]
fn off_macos_the_ios_set_reports_that_it_does_not_apply() {
    for os in [Os::Linux, Os::Windows] {
        let machine = FakeMachine::new().with_home("/home/dev");
        let mut runner = Runner::record().with_os(os).with_root(false);
        let plan = plan(&env(os, false, None), &[Set::Ios]).expect("plan");
        let ios: Vec<_> = plan.steps.iter().filter(|s| s.set == Set::Ios).collect();
        assert_eq!(ios.len(), 1, "one informational step on {os:?}");
        let env = env(os, false, None);
        let mut path = UserPath::Windows(Box::new(rayx_cli::host::path_env::MemoryPathStore::new(
            None,
        )));
        let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
        let probed = (ios[0].probe)(&mut cx);
        assert!(
            probed.satisfied && probed.detail.contains("needs macOS"),
            "{os:?}: {probed:?}"
        );
    }
}

fn plist_path(app: &str) -> String {
    format!("/Applications/{app}/Contents/version.plist")
}

fn plist(version: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?>\n<plist version=\"1.0\"><dict>\n<key>CFBundleShortVersionString</key>\n<string>{version}</string>\n<key>CFBundleVersion</key>\n<string>23792</string>\n</dict></plist>\n"
    )
}

#[test]
fn an_installed_xcode_counts_before_it_is_selected_and_before_its_license_is_accepted() {
    // Right after `xcodes install`: Xcode.app is there, the Command Line Tools are still
    // selected and `xcodebuild` refuses to run. Only selection and the license remain.
    let machine = machine()
        .with_text(plist_path("Xcode.app"), &plist("26.1"))
        .with_dir("/Applications/Xcode.app");
    let (_, text) = check(&mac(false, Some("26.1")), &machine, &mut bare());
    assert!(
        status(&text, "Xcode 26.1").starts_with("[ok     ]"),
        "{text}"
    );
    assert!(
        status(&text, "Xcode 26.1").contains("(found 26.1)"),
        "{text}"
    );
    assert!(
        status(&text, "xcode-select points at Xcode").starts_with("[missing]"),
        "{text}"
    );
    assert!(
        status(&text, "Xcode license").starts_with("[missing]"),
        "{text}"
    );
    assert!(
        !text.contains("xcodes install"),
        "no second multi-GB download:\n{text}"
    );
}

#[test]
fn a_pinned_xcode_is_selected_even_when_a_newer_one_is_installed() {
    use rayx_cli::setup::ios::{chosen_xcode, plist_version, xcode_apps};

    let machine = machine()
        .with_text(plist_path("Xcode-16.2.app"), &plist("16.2"))
        .with_text(plist_path("Xcode-16.10.app"), &plist("16.10"))
        .with_text(plist_path("Xcode.app"), &plist("26.1"));
    assert_eq!(plist_version(&plist("16.2")).as_deref(), Some("16.2"));

    // Numeric version order: 26.1 > 16.10 > 16.2 (not the order of the file names).
    let env = mac(false, Some("16.2"));
    let mut path = UserPath::Windows(Box::new(rayx_cli::host::path_env::MemoryPathStore::new(
        None,
    )));
    let mut rec = bare();
    let cx = Cx::new(&env, &mut rec, &machine, &mut path);
    let versions: Vec<_> = xcode_apps(&cx)
        .into_iter()
        .filter_map(|a| a.version)
        .collect();
    assert_eq!(versions, ["26.1", "16.10", "16.2"]);
    assert_eq!(
        chosen_xcode(&cx).map(|a| a.path),
        Some(PathBuf::from("/Applications/Xcode-16.2.app"))
    );

    let (_, text) = check(&env, &machine, &mut bare());
    assert!(
        text.contains("sudo xcode-select --switch /Applications/Xcode-16.2.app/Contents/Developer"),
        "{text}"
    );
    assert!(
        status(&text, "Xcode 16.2").starts_with("[ok     ]"),
        "{text}"
    );

    // Unpinned: the newest by version.
    let (_, text) = check(&mac(false, None), &machine, &mut bare());
    assert!(
        text.contains("sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer"),
        "{text}"
    );

    // Selected to the wrong app: still to be switched.
    let wrong = bare().responder(|spec| {
        (spec.program == "xcode-select")
            .then(|| Outcome::success().with_stdout("/Applications/Xcode.app/Contents/Developer\n"))
    });
    let (_, text) = check(&env, &machine, &mut { wrong });
    assert!(
        status(&text, "xcode-select points at Xcode").starts_with("[missing]"),
        "{text}"
    );
}
