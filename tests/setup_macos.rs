//! The base set on macOS: Command Line Tools through `softwareupdate`, the `xcode-select`
//! fallback, Homebrew, rustup and the toolchain.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::UserPath;
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Privilege, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::macos::{CLT_MARKER, parse_clt_labels};
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, execute, plan, run_plan};

fn env(arch: Arch, yes: bool) -> PlanEnv {
    PlanEnv {
        host: HostFacts {
            os: Os::MacOs,
            arch,
            emulated: false,
            wsl: false,
            distro: None,
        },
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes,
    }
}

const LISTING_NEW: &str = "Software Update Tool\n\nFinding available software\nSoftware Update found the following new or updated software:\n* Label: Command Line Tools for Xcode-15.9\n\tTitle: Command Line Tools for Xcode, Version: 15.9, Size: 700000KiB, Recommended: YES, \n* Label: Command Line Tools for Xcode-15.10\n\tTitle: Command Line Tools for Xcode, Version: 15.10, Size: 700000KiB, Recommended: YES, \n* Label: macOS Sequoia 15.1-24B83\n\tTitle: macOS Sequoia 15.1, Version: 15.1, Size: 3000000KiB, Recommended: YES, Action: restart, \n";
const LISTING_OLD: &str = "Software Update Tool\n\nSoftware Update found the following new or updated software:\n   * Command Line Tools for Xcode-14.2\n\tCommand Line Tools for Xcode (14.2), 400000K [recommended]\n";

fn machine() -> FakeMachine {
    FakeMachine::new().with_home("/Users/dev")
}

fn user_path() -> UserPath {
    UserPath::Profiles {
        files: vec![PathBuf::from("/Users/dev/.zprofile")],
        home: PathBuf::from("/Users/dev"),
    }
}

fn bare_runner() -> Runner {
    Runner::record().with_os(Os::MacOs).with_root(false)
}

fn check(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> (u8, String) {
    let plan = plan(env, &[Set::Base]).expect("plan");
    let mut path = user_path();
    let mut out = Vec::new();
    let mut cx = Cx::new(env, runner, machine, &mut path);
    let args = SetupArgs {
        check: true,
        ..SetupArgs::default()
    };
    let code = run_plan(&args, &plan, &mut cx, &mut out);
    (code, String::from_utf8(out).expect("UTF-8"))
}

#[test]
fn the_newest_command_line_tools_label_is_chosen_in_either_output_style() {
    assert_eq!(
        parse_clt_labels(LISTING_NEW),
        [
            "Command Line Tools for Xcode-15.9",
            "Command Line Tools for Xcode-15.10"
        ],
        "numeric order: 15.10 is newer than 15.9, and macOS updates are not offered"
    );
    assert_eq!(
        parse_clt_labels(LISTING_OLD),
        ["Command Line Tools for Xcode-14.2"]
    );
    assert!(parse_clt_labels("No new software available.\n").is_empty());
    assert_eq!(
        CLT_MARKER,
        "/tmp/.com.apple.dt.CommandLineTools.installondemand.in-progress"
    );
}

#[test]
fn command_line_tools_install_through_softwareupdate_with_sudo() {
    let env = env(Arch::Arm64, true);
    let plan = plan(&env, &[Set::Base]).expect("plan");
    let machine = machine();
    let mut path = user_path();
    let mut runner = bare_runner().responder(|spec| match spec.program.as_str() {
        "xcode-select" => Some(Outcome::failure(2)),
        "softwareupdate" if spec.args == ["--list"] => {
            Some(Outcome::success().with_stdout(LISTING_NEW))
        }
        _ => None,
    });
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    execute(&plan, &mut cx, &mut Vec::new());

    let first: Vec<_> = runner
        .specs()
        .iter()
        .filter(|s| s.program == "softwareupdate")
        .collect();
    assert_eq!(first.len(), 2, "list, then install");
    assert_eq!(first[0].args, ["--list"]);
    assert_eq!(
        first[1].args,
        ["--install", "Command Line Tools for Xcode-15.10"]
    );
    assert_eq!(first[1].privilege, Privilege::Root);
    assert!(
        runner
            .lines()
            .iter()
            .any(|l| l == "sudo softwareupdate --install 'Command Line Tools for Xcode-15.10'"),
        "{:?}",
        runner.lines()
    );
    assert!(
        !runner
            .specs()
            .iter()
            .any(|s| s.program == "xcode-select" && s.args == ["--install"]),
        "no fallback when a label is offered"
    );
}

#[test]
fn without_an_offered_label_xcode_select_installs_and_setup_waits_for_it() {
    let env = env(Arch::X64, false);
    let plan = plan(&env, &[Set::Base]).expect("plan");
    let machine = machine();
    let mut path = user_path();
    let polls = Rc::new(Cell::new(0));
    let seen = polls.clone();
    let mut runner = bare_runner().responder(move |spec| match spec.program.as_str() {
        "xcode-select" if spec.args == ["-p"] => {
            // Missing at the probe and for the first two polls, then installed.
            seen.set(seen.get() + 1);
            Some(if seen.get() <= 3 {
                Outcome::failure(2)
            } else {
                Outcome::success()
            })
        }
        "softwareupdate" => Some(Outcome::success().with_stdout("No new software available.\n")),
        _ => None,
    });
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path).with_poll_interval(Duration::ZERO);
    execute(&plan, &mut cx, &mut Vec::new());

    let programs: Vec<String> = runner
        .specs()
        .iter()
        .filter(|s| s.program == "xcode-select" || s.program == "softwareupdate")
        .map(|s| format!("{} {}", s.program, s.args.join(" ")))
        .collect();
    assert_eq!(
        programs,
        [
            "xcode-select -p",
            "softwareupdate --list",
            "xcode-select --install",
            "xcode-select -p",
            "xcode-select -p",
            "xcode-select -p",
        ],
        "probe, list, fallback install, then polls until it finishes"
    );
    assert!(
        !runner.specs().iter().any(|s| s
            .args
            .first()
            .is_some_and(|a| a == "--install" && s.program == "softwareupdate")),
        "nothing is installed through softwareupdate"
    );
}

#[test]
fn check_shows_the_marker_and_the_fallback_for_the_command_line_tools() {
    let env = env(Arch::Arm64, false);
    let mut runner = bare_runner()
        .responder(|spec| (spec.program == "xcode-select").then(|| Outcome::failure(2)));
    let (code, text) = check(&env, &machine(), &mut runner);
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("[missing] base: Xcode Command Line Tools"),
        "{text}"
    );
    assert!(text.contains(CLT_MARKER), "{text}");
    assert!(text.contains("sudo softwareupdate --install"), "{text}");
    assert!(text.contains("xcode-select --install"), "{text}");
}

#[test]
fn homebrew_installs_with_the_official_script_and_apple_silicon_gets_its_prefix_on_path() {
    let machine = machine();
    let arm = env(Arch::Arm64, true);
    let (_, text) = check(&arm, &machine, &mut bare_runner());
    let line = text
        .lines()
        .find(|l| l.contains("Homebrew/install/HEAD/install.sh"))
        .unwrap_or_else(|| panic!("installer line:\n{text}"));
    assert!(
        line.contains("NONINTERACTIVE=1"),
        "--yes makes it non-interactive: {line}"
    );
    assert!(
        text.contains("add /opt/homebrew/bin to the user PATH"),
        "{text}"
    );

    let intel = env(Arch::X64, false);
    let (_, text) = check(&intel, &machine, &mut bare_runner());
    let line = text
        .lines()
        .find(|l| l.contains("Homebrew/install/HEAD/install.sh"))
        .unwrap_or_else(|| panic!("installer line:\n{text}"));
    assert!(
        !line.contains("NONINTERACTIVE"),
        "the prompt reaches the user: {line}"
    );
    assert!(!text.contains("/opt/homebrew"), "{text}");
}

#[test]
fn rustup_and_the_toolchain_follow_with_the_cargo_bin_directory_on_path() {
    let env = env(Arch::Arm64, false);
    let (_, text) = check(&env, &machine(), &mut bare_runner());
    assert!(text.contains("sh.rustup.rs"), "{text}");
    assert!(text.contains("rustup toolchain install stable"), "{text}");
    let cargo_bin = PathBuf::from("/Users/dev").join(".cargo").join("bin");
    assert!(
        text.contains(&format!("add {} to the user PATH", cargo_bin.display())),
        "{text}"
    );
    let order: Vec<usize> = [
        "Command Line Tools",
        "Homebrew",
        "rustup (",
        "Rust stable",
        "~/.cargo/bin",
    ]
    .iter()
    .map(|needle| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle}:\n{text}"))
    })
    .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "ordered:\n{text}");
}

#[test]
fn a_provisioned_mac_has_nothing_to_do() {
    let env = env(Arch::Arm64, false);
    let machine = machine()
        .with_file("/opt/homebrew/bin/brew")
        .with_program("rustup", "/Users/dev/.cargo/bin/rustup")
        .with_program("cargo", "/Users/dev/.cargo/bin/cargo");
    let mut runner = bare_runner().responder(|spec| match spec.program.as_str() {
        "xcode-select" => {
            Some(Outcome::success().with_stdout("/Library/Developer/CommandLineTools\n"))
        }
        p if p.ends_with("rustup") => {
            Some(Outcome::success().with_stdout("stable-aarch64-apple-darwin (default)\n"))
        }
        _ => None,
    });
    let (code, text) = check(&env, &machine, &mut runner);
    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("[missing]"), "{text}");
}

mod marker {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use rayx_cli::host::{Outcome, Runner};
    use rayx_cli::setup::{Cx, FakeMachine, Set, execute, plan};

    use super::{LISTING_NEW, bare_runner, env, machine, user_path};
    use rayx_cli::host::Arch;

    /// Runs the Command Line Tools install against a temporary marker file and returns what the
    /// marker looked like at each `softwareupdate` call, the final state and the report.
    fn run(install: Outcome) -> (Vec<(String, bool)>, bool, Option<String>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let marker: PathBuf = dir.path().join("installondemand.in-progress");
        let seen: Rc<RefCell<Vec<(String, bool)>>> = Rc::default();
        let observed = seen.clone();
        let watched = marker.clone();
        let mut runner: Runner = bare_runner().responder(move |spec| match spec.program.as_str() {
            "xcode-select" => Some(Outcome::failure(2)),
            "softwareupdate" => {
                observed
                    .borrow_mut()
                    .push((spec.args.join(" "), watched.exists()));
                Some(if spec.args == ["--list"] {
                    Outcome::success().with_stdout(LISTING_NEW)
                } else {
                    install.clone()
                })
            }
            _ => None,
        });
        let env = env(Arch::Arm64, false);
        let plan = plan(&env, &[Set::Base]).expect("plan");
        let machine: FakeMachine = machine();
        let mut path = user_path();
        let mut cx = Cx::new(&env, &mut runner, &machine, &mut path).with_clt_marker(&marker);
        let report = execute(&plan, &mut cx, &mut Vec::new());
        let calls = seen.borrow().clone();
        (calls, marker.exists(), report.failure().map(str::to_string))
    }

    #[test]
    fn the_marker_exists_while_softwareupdate_runs_and_is_removed_after_success() {
        let (calls, still_there, failure) = run(Outcome::success());
        assert_eq!(failure, None);
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!(
            calls[0],
            ("--list".to_string(), true),
            "marked before listing"
        );
        assert!(calls[1].0.starts_with("--install "), "{calls:?}");
        assert!(calls[1].1, "still marked while installing");
        assert!(!still_there, "the marker is removed afterwards");
    }

    #[test]
    fn the_marker_is_removed_even_when_the_install_fails() {
        let (calls, still_there, failure) = run(Outcome::failure(1));
        assert!(failure.is_some_and(|m| m.contains("Command Line Tools")));
        assert!(calls[1].1);
        assert!(
            !still_there,
            "a failed install must not leave the marker behind"
        );
    }
}

#[test]
fn homebrew_under_yes_asks_for_sudo_credentials_before_the_noninteractive_installer() {
    let env = env(Arch::Arm64, true);
    let plan = plan(&env, &[Set::Base]).expect("plan");
    let machine = machine();
    let mut path = user_path();
    let mut runner = bare_runner().responder(|spec| match spec.program.as_str() {
        "xcode-select" => Some(Outcome::success()),
        _ => None,
    });
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    execute(&plan, &mut cx, &mut Vec::new());
    let programs: Vec<&str> = runner.specs().iter().map(|s| s.program.as_str()).collect();
    let sudo = programs
        .iter()
        .position(|p| *p == "sudo")
        .expect("sudo -v runs");
    let installer = programs
        .iter()
        .position(|p| *p == "/bin/bash")
        .expect("installer runs");
    assert_eq!(runner.specs()[sudo].args, ["-v"]);
    assert!(sudo < installer, "credentials first: {programs:?}");

    // Without --yes the installer prompts for itself.
    let env = crate::env(Arch::Arm64, false);
    let plan = rayx_cli::setup::plan(&env, &[Set::Base]).expect("plan");
    let mut path = user_path();
    let mut runner =
        bare_runner().responder(|spec| (spec.program == "xcode-select").then(Outcome::success));
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    execute(&plan, &mut cx, &mut Vec::new());
    assert!(!runner.specs().iter().any(|s| s.program == "sudo"));
}
