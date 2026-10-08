//! Prerequisites of app commands: which `setup` sets a target needs, what is installed on the
//! spot, and what is left to the owner.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use rayx_cli::app::prereqs::{Options, ensure, ensure_plan, required_sets, setup_command};
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Privilege, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::{
    Action, Cx, FakeMachine, Plan, PlanEnv, Probed, Set, Step, plan as setup_plan,
};

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
        project_root: Some(PathBuf::from("/work/app")),
        tools_node: None,
        yes: false,
    }
}

fn user_path() -> UserPath {
    UserPath::Windows(Box::new(MemoryPathStore::new(None)))
}

/// A step that is met once its install ran.
fn step(title: &str, privilege: Privilege, present: &Rc<Cell<bool>>) -> Step {
    let probe_state = present.clone();
    let install_state = present.clone();
    let command = title.to_string();
    Step::new(
        "fixture",
        Set::Web,
        title,
        privilege,
        move |_| {
            if probe_state.get() {
                Probed::ok()
            } else {
                Probed::missing("not installed")
            }
        },
        move |_, _| {
            let state = install_state.clone();
            Ok(vec![
                Action::Run(rayx_cli::host::CommandSpec::new("install").arg(command.clone())),
                Action::Custom {
                    description: "mark installed".to_string(),
                    run: Rc::new(move |_| {
                        state.set(true);
                        Ok(())
                    }),
                },
            ])
        },
    )
}

struct Outcome {
    result: anyhow::Result<usize>,
    output: String,
    installed: Vec<String>,
    asked: Vec<String>,
}

fn run(steps: Vec<Step>, options: Options, answer: bool) -> Outcome {
    let env = env();
    let plan = Plan { steps };
    let mut runner = Runner::record().with_os(Os::Linux).with_root(false);
    let machine = FakeMachine::new();
    let mut path = user_path();
    let mut out = Vec::new();
    let mut asked = Vec::new();
    let result = {
        let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
        ensure_plan(
            &plan,
            "rayx setup --web",
            &options,
            &mut cx,
            &mut out,
            &mut |question| {
                asked.push(question.to_string());
                answer
            },
        )
    };
    Outcome {
        result,
        output: String::from_utf8(out).expect("UTF-8"),
        installed: runner
            .specs()
            .iter()
            .map(|spec| spec.args.join(" "))
            .collect(),
        asked,
    }
}

fn terminal() -> Options {
    Options {
        no_install: false,
        interactive: true,
    }
}

fn batch() -> Options {
    Options {
        no_install: false,
        interactive: false,
    }
}

fn message(outcome: &Outcome) -> String {
    format!("{:#}", outcome.result.as_ref().expect_err("a failure"))
}

#[test]
fn the_target_decides_which_sets_a_command_needs() {
    assert_eq!(required_sets("build", "wasm"), [Set::Web]);
    assert_eq!(required_sets("run", "wasm"), [Set::Web]);
    assert_eq!(required_sets("test", "wasm"), [Set::Web, Set::Test]);
    assert_eq!(required_sets("deploy", "android"), [Set::Android]);
    assert_eq!(required_sets("build", "ios"), [Set::Ios]);
    for native in ["host", "windows", "linux", "macos", "mobile"] {
        assert!(required_sets("build", native).is_empty(), "{native}");
        assert!(required_sets("test", native).is_empty(), "{native}");
    }
    assert!(required_sets("assets", "wasm").is_empty());
}

#[test]
fn unsupported_action_target_pairs_fail_before_anything_is_offered_for_install() {
    use rayx_cli::app::check_supported;

    // `test android` would need the Android set; it says "not supported yet" instead.
    for target in ["android", "ios", "mobile"] {
        let error = check_supported("test", target).expect_err(target);
        assert!(error.to_string().contains("not supported yet"), "{error}");
    }
    let error = check_supported("deploy", "wasm").expect_err("deploy wasm");
    assert!(error.to_string().contains("only for android"), "{error}");
    assert!(check_supported("build", "nowhere").is_err());
    assert!(check_supported("explode", "host").is_err());
    for (action, target) in [
        ("build", "wasm"),
        ("run", "android"),
        ("pack", "ios"),
        ("publish", "host"),
        ("deploy", "android"),
        ("test", "wasm"),
        ("test", "host"),
    ] {
        check_supported(action, target).unwrap_or_else(|e| panic!("{action} {target}: {e}"));
    }
}

#[test]
fn the_setup_command_names_each_set_but_the_base() {
    assert_eq!(setup_command(&[]), "rayx setup");
    assert_eq!(
        setup_command(&[Set::Base, Set::Web, Set::Test]),
        "rayx setup --web --test"
    );
    assert_eq!(setup_command(&[Set::Android]), "rayx setup --android");
}

#[test]
fn a_complete_machine_installs_nothing_and_says_nothing() {
    let present = Rc::new(Cell::new(true));
    let outcome = run(
        vec![step("wasm-bindgen 0.2.129", Privilege::None, &present)],
        batch(),
        false,
    );

    assert_eq!(outcome.result.expect("ready"), 0);
    assert!(outcome.output.is_empty(), "{}", outcome.output);
    assert!(outcome.installed.is_empty());
    assert!(outcome.asked.is_empty());
}

#[test]
fn unprivileged_items_install_automatically_with_a_one_line_notice() {
    let present = Rc::new(Cell::new(false));
    let outcome = run(
        vec![step("wasm-bindgen 0.2.129", Privilege::None, &present)],
        batch(),
        false,
    );

    assert_eq!(outcome.result.expect("installed"), 1);
    assert_eq!(
        outcome.output.lines().find(|l| l.starts_with("rayx:")),
        Some("rayx: installing missing prerequisites: wasm-bindgen 0.2.129")
    );
    assert_eq!(outcome.installed, ["wasm-bindgen 0.2.129"]);
    assert!(
        outcome.asked.is_empty(),
        "nothing to ask: {:?}",
        outcome.asked
    );
}

#[test]
fn a_privileged_item_stops_a_batch_run_with_the_setup_command() {
    let present = Rc::new(Cell::new(false));
    let outcome = run(
        vec![step("clang and llvm", Privilege::Root, &present)],
        batch(),
        true,
    );

    let text = message(&outcome);
    assert!(text.contains("rayx setup --web"), "{text}");
    assert!(
        text.contains("clang and llvm needs administrator rights"),
        "{text}"
    );
    assert!(outcome.installed.is_empty(), "nothing is installed");
    assert!(outcome.asked.is_empty());
}

#[test]
fn a_privileged_item_in_a_terminal_is_installed_after_the_owner_agrees() {
    let present = Rc::new(Cell::new(false));
    let yes = run(
        vec![step("clang and llvm", Privilege::Root, &present)],
        terminal(),
        true,
    );
    assert_eq!(yes.result.expect("installed"), 1);
    assert_eq!(yes.installed, ["clang and llvm"]);
    assert_eq!(yes.asked.len(), 1);
    assert!(
        yes.asked[0].contains("rayx setup --web"),
        "{}",
        yes.asked[0]
    );

    let present = Rc::new(Cell::new(false));
    let no = run(
        vec![step("clang and llvm", Privilege::Root, &present)],
        terminal(),
        false,
    );
    assert!(message(&no).contains("rayx setup --web"));
    assert!(no.installed.is_empty(), "a refusal installs nothing");
}

#[test]
fn a_license_question_counts_as_the_owners_business_even_without_privileges() {
    let present = Rc::new(Cell::new(false));
    let licensed = step("Android SDK packages", Privilege::None, &present).with_prompt();
    let outcome = run(vec![licensed], batch(), true);

    let text = message(&outcome);
    assert!(text.contains("license answer"), "{text}");
    assert!(outcome.installed.is_empty());
}

#[test]
fn no_install_reports_what_is_missing_and_stops() {
    let present = Rc::new(Cell::new(false));
    let options = Options {
        no_install: true,
        interactive: true,
    };
    let outcome = run(
        vec![step("wasm-bindgen 0.2.129", Privilege::None, &present)],
        options,
        true,
    );

    let text = message(&outcome);
    assert!(text.contains("wasm-bindgen 0.2.129"), "{text}");
    assert!(text.contains("--no-install"), "{text}");
    assert!(outcome.installed.is_empty());
    assert!(outcome.asked.is_empty());
}

#[test]
fn a_real_plan_on_a_bare_linux_machine_names_the_sets_to_install() {
    let env = env();
    let mut runner = Runner::record().with_os(Os::Linux).with_root(false);
    let machine = FakeMachine::new().with_home("/home/dev");
    let mut path = user_path();
    let options = Options {
        no_install: true,
        interactive: false,
    };

    let error = ensure(
        &env,
        &[Set::Web, Set::Test],
        &options,
        &mut runner,
        &machine,
        &mut path,
        &mut Vec::new(),
        &mut |_| false,
    )
    .expect_err("a bare machine misses the base set");

    let text = format!("{error:#}");
    assert!(text.contains("rayx setup --web --test"), "{text}");
    // `--no-install` only looks: nothing privileged or interactive ran.
    assert!(
        runner
            .specs()
            .iter()
            .all(|spec| !spec.interactive && spec.privilege == Privilege::None),
        "{:?}",
        runner.lines()
    );
}

#[test]
fn only_the_android_license_step_asks_a_question() {
    let env = env();
    let plan = setup_plan(&env, &[Set::Android]).expect("plan");
    let asking: Vec<&str> = plan
        .steps
        .iter()
        .filter(|step| step.prompts)
        .map(|step| step.id)
        .collect();
    assert_eq!(asking, ["android-packages"]);
}

#[test]
fn a_step_builds_do_not_need_is_neither_waited_for_nor_installed() {
    let present = Rc::new(Cell::new(false));
    let ide = step("Android Studio", Privilege::None, &present).optional_for_builds();
    let outcome = run(vec![ide], batch(), false);

    assert_eq!(outcome.result.expect("nothing blocks the build"), 0);
    assert!(outcome.installed.is_empty());
    assert!(outcome.output.is_empty());
}

#[test]
fn a_winget_install_that_asks_for_the_license_belongs_to_the_owner() {
    let installed = Rc::new(Cell::new(false));
    let probe_state = installed.clone();
    let winget = |accepts: bool| {
        let probe_state = probe_state.clone();
        Step::new(
            "jdk",
            Set::Android,
            "JDK 21",
            Privilege::None,
            move |_| {
                if probe_state.get() {
                    Probed::ok()
                } else {
                    Probed::missing("no JDK")
                }
            },
            move |_, _| {
                let mut spec = rayx_cli::host::CommandSpec::new("winget").args([
                    "install",
                    "--id",
                    "Microsoft.OpenJDK.21",
                    "--exact",
                ]);
                if accepts {
                    spec = spec.arg("--accept-package-agreements");
                }
                Ok(vec![Action::Run(spec)])
            },
        )
    };

    let asks = run(vec![winget(false)], batch(), true);
    assert!(
        message(&asks).contains("license answer"),
        "{}",
        message(&asks)
    );
    assert!(asks.installed.is_empty());

    let accepted = run(vec![winget(true)], batch(), true);
    assert_eq!(
        accepted.result.expect("agreements were accepted up front"),
        1
    );
}

#[test]
fn a_host_setup_cannot_install_for_does_not_block_the_build() {
    let mut env = env();
    env.host.distro = Some(Distro::parse(
        "ID=fedora\nVERSION_ID=\"41\"\nPRETTY_NAME=\"Fedora Linux 41\"\n",
    ));
    let mut runner = Runner::record().with_os(Os::Linux).with_root(false);
    let machine = FakeMachine::new().with_home("/home/dev");
    let mut path = user_path();
    let mut out = Vec::new();

    let installed = ensure(
        &env,
        &[Set::Web],
        &Options {
            no_install: false,
            interactive: false,
        },
        &mut runner,
        &machine,
        &mut path,
        &mut out,
        &mut |_| false,
    )
    .expect("the build goes on");

    assert_eq!(installed, 0);
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(text.contains("prerequisites not checked"), "{text}");
    assert!(text.contains("Fedora"), "{text}");
    assert!(runner.specs().is_empty(), "nothing ran");
}
