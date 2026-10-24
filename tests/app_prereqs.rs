//! Prerequisites of nsh commands (RUN-2, design §6.1): which `setup` sets a command and its
//! `--target` need, what is installed on the spot (the pinned nsh among it), what is left to the
//! owner, and the `--browser` rayx adds for `test --target web`.

use std::cell::Cell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rayx_cli::app::prereqs::{
    Options, browser_args, ensure, ensure_plan, has_browser, required_sets, setup_command,
    system_browser, target,
};
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
        nsh: None,
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

fn words(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn the_command_and_its_target_decide_the_sets_beyond_base() {
    assert_eq!(required_sets("run", Some("web")), [Set::Web]);
    assert_eq!(required_sets("dev", Some("web")), [Set::Web]);
    assert_eq!(required_sets("package", Some("web")), [Set::Web]);
    assert_eq!(required_sets("test", Some("web")), [Set::Web, Set::Test]);
    for apple in ["ios", "ios-simulator", "catalyst"] {
        assert_eq!(required_sets("run", Some(apple)), [Set::Ios], "{apple}");
        assert_eq!(required_sets("test", Some(apple)), [Set::Ios], "{apple}");
    }
    assert_eq!(required_sets("run", Some("android")), [Set::Android]);
    assert_eq!(required_sets("package", Some("android")), [Set::Android]);
    for desktop in [
        Some("macos"),
        Some("linux"),
        Some("windows"),
        Some("gpui"),
        None,
    ] {
        assert!(required_sets("run", desktop).is_empty(), "{desktop:?}");
        assert!(required_sets("test", desktop).is_empty(), "{desktop:?}");
    }
    for command in [
        "build",
        "check",
        "init",
        "keygen",
        "certify",
        "publish",
        "verify",
        "revoke",
        "symbolicate",
    ] {
        assert!(required_sets(command, Some("web")).is_empty(), "{command}");
        assert!(
            required_sets(command, Some("android")).is_empty(),
            "{command}"
        );
    }
}

#[test]
fn the_target_is_read_from_nshs_arguments_before_any_double_dash() {
    assert_eq!(target(&words(&["--target", "web"])).as_deref(), Some("web"));
    assert_eq!(
        target(&words(&[".", "--target=ios", "--port", "1"])).as_deref(),
        Some("ios")
    );
    assert_eq!(
        target(&words(&["--", "--target", "web"])),
        None,
        "app arguments are not nsh's"
    );
    assert_eq!(target(&words(&["--port", "4000"])), None);
    assert!(has_browser(&words(&["--browser", "/x"])));
    assert!(has_browser(&words(&["--browser=/x"])));
    assert!(!has_browser(&words(&["--", "--browser", "/x"])));
}

#[test]
fn the_setup_command_runs_in_the_directory_and_names_each_set_but_the_base() {
    let dir = Path::new("/work/app");
    assert_eq!(setup_command(dir, &[]), "cd /work/app && rayx setup");
    assert_eq!(
        setup_command(dir, &[Set::Base, Set::Web, Set::Test]),
        "cd /work/app && rayx setup --web --test"
    );
    assert_eq!(
        setup_command(dir, &[Set::Android]),
        "cd /work/app && rayx setup --android"
    );
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
        Path::new("/work/app"),
        &options,
        &mut runner,
        &machine,
        &mut path,
        &mut Vec::new(),
        &mut |_| false,
    )
    .expect_err("a bare machine misses the base set");

    let text = format!("{error:#}");
    assert!(
        text.contains("cd /work/app && rayx setup --web --test"),
        "{text}"
    );
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
        Path::new("/work/app"),
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

#[test]
fn on_a_host_setup_cannot_install_for_the_pinned_nsh_is_still_ensured() {
    let home = tempfile::tempdir().expect("temp dir");
    let mut env = env();
    env.host.distro = Some(Distro::parse(
        "ID=fedora\nVERSION_ID=\"41\"\nPRETTY_NAME=\"Fedora Linux 41\"\n",
    ));
    env.nsh = Some(Ok(rayx_cli::nsh::pin::default_pin(Path::new("/work/app"))));
    let mut runner = Runner::record().with_os(Os::Linux).with_root(false);
    let machine = FakeMachine::new()
        .with_home("/home/dev")
        .with_env("RAYX_HOME", &home.path().display().to_string());
    let mut path = user_path();
    let mut out = Vec::new();

    let error = ensure(
        &env,
        &[Set::Web],
        Path::new("/work/app"),
        &Options {
            no_install: true,
            interactive: false,
        },
        &mut runner,
        &machine,
        &mut path,
        &mut out,
        &mut |_| false,
    )
    .expect_err("the pin is missing")
    .to_string();
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(text.contains("prerequisites not checked"), "{text}");
    assert!(error.contains("nsh fork-0.2.0-4 (327bb9e)"), "{error}");
    assert!(
        error.contains("`rayx nsh install -C /work/app`") && !error.contains("rayx setup"),
        "the command that works on this host: {error}"
    );
}

#[test]
fn the_pinned_nsh_installs_without_asking_like_any_silent_step() {
    let mut env = env();
    let pin = rayx_cli::nsh::pin::default_pin(Path::new("/work/app"));
    env.nsh = Some(Ok(pin.clone()));
    let plan = Plan {
        steps: vec![rayx_cli::setup::nsh::step(&pin)],
    };
    assert_eq!(plan.steps[0].privilege, Privilege::None);
    assert!(!plan.steps[0].prompts && !plan.steps[0].optional_for_builds);
    let home = tempfile::tempdir().expect("temp dir");
    let mut runner = Runner::record().with_os(Os::Linux).with_root(false);
    let machine = FakeMachine::new()
        .with_home("/home/dev")
        .with_env("RAYX_HOME", &home.path().display().to_string());
    let mut path = user_path();
    let mut out = Vec::new();
    let mut asked = Vec::new();
    let result = {
        let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
        ensure_plan(&plan, "rayx setup", &batch(), &mut cx, &mut out, &mut |q| {
            asked.push(q.to_string());
            false
        })
    };
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(
        text.contains(
            "rayx: installing missing prerequisites: nsh fork-0.2.0-4 (327bb9e) (default)"
        ),
        "{text}"
    );
    assert!(asked.is_empty(), "nothing is asked: {asked:?}");
    // The recorded `curl` answers nothing, so the install itself stops; what matters is that it
    // was attempted without a question.
    let error = format!("{:#}", result.expect_err("no download in a recording"));
    assert!(!error.contains("administrator rights"), "{error}");
    assert!(
        runner.lines().iter().any(|l| l.starts_with("curl ")),
        "{:?}",
        runner.lines()
    );
}

fn mac_env() -> PlanEnv {
    let mut env = env();
    env.host.os = Os::MacOs;
    env.host.arch = Arch::Arm64;
    env.host.distro = None;
    env.project_root = None;
    env
}

/// A Mac with the test set's Chromium in Playwright's cache.
fn mac_with_chromium() -> (FakeMachine, PathBuf) {
    let chromium = [
        "/Users/dev/Library/Caches/ms-playwright/chromium-1217",
        "chrome-mac-arm64",
        "Google Chrome for Testing.app",
        "Contents",
        "MacOS",
        "Google Chrome for Testing",
    ]
    .iter()
    .fold(PathBuf::new(), |p, part| p.join(part));
    (
        FakeMachine::new()
            .with_home("/Users/dev")
            .with_file(&chromium),
        chromium,
    )
}

fn browser(command: &str, args: &[&str], machine: &FakeMachine) -> Vec<OsString> {
    let env = mac_env();
    let mut runner = Runner::record();
    let mut path = user_path();
    let cx = Cx::new(&env, &mut runner, machine, &mut path);
    browser_args(command, &words(args), &cx)
}

#[test]
fn test_on_the_web_without_a_browser_gets_the_test_sets_chromium() {
    let (machine, chromium) = mac_with_chromium();
    assert_eq!(
        browser("test", &["--target", "web"], &machine),
        [OsString::from("--browser"), chromium.into_os_string()]
    );
}

#[test]
fn no_browser_is_added_when_one_is_given_installed_or_not_needed() {
    let (machine, _) = mac_with_chromium();
    assert!(browser("test", &["--target", "web", "--browser", "/x"], &machine).is_empty());
    assert!(
        browser("run", &["--target", "web"], &machine).is_empty(),
        "only test"
    );
    assert!(
        browser("test", &["--target", "macos"], &machine).is_empty(),
        "only the web"
    );
    let chrome = machine.clone().with_file(PathBuf::from(
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ));
    assert!(
        browser("test", &["--target", "web"], &chrome).is_empty(),
        "nsh finds Chrome itself"
    );
    let bare = FakeMachine::new().with_home("/Users/dev");
    assert!(
        browser("test", &["--target", "web"], &bare).is_empty(),
        "without the test set's Chromium nsh reports what is missing"
    );
}

#[test]
fn the_system_browser_is_found_where_nsh_looks() {
    let edge = FakeMachine::new()
        .with_env("ProgramFiles(x86)", r"C:\Program Files (x86)")
        .with_file(
            PathBuf::from(r"C:\Program Files (x86)")
                .join("Microsoft")
                .join("Edge")
                .join("Application")
                .join("msedge.exe"),
        );
    assert!(system_browser(&edge, Os::Windows).is_some());
    let chromium = FakeMachine::new().with_program("chromium-browser", "/usr/bin/chromium-browser");
    assert_eq!(
        system_browser(&chromium, Os::Linux),
        Some(PathBuf::from("/usr/bin/chromium-browser"))
    );
    let home_app = FakeMachine::new()
        .with_home("/Users/dev")
        .with_file(PathBuf::from(
            "/Users/dev/Applications/Chromium.app/Contents/MacOS/Chromium",
        ));
    assert!(system_browser(&home_app, Os::MacOs).is_some());
    assert_eq!(system_browser(&FakeMachine::new(), Os::Linux), None);
}
