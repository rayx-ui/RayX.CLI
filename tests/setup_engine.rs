//! The setup engine over synthetic plans: status and exit codes, execute-only-missing,
//! idempotence, `--yes`, `--all` per host, and root-step batching.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::{MemoryPathStore, PathChange, UserPath};
use rayx_cli::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Privilege, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::{
    Action, Cx, FakeMachine, Plan, PlanEnv, Probed, Set, Step, StepOutcome, execute, run_plan,
    selected_sets,
};

fn host(os: Os, arch: Arch) -> HostFacts {
    HostFacts {
        os,
        arch,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn env(yes: bool) -> PlanEnv {
    PlanEnv {
        host: host(Os::Linux, Arch::X64),
        pins: Pins::resolve(None),
        project_root: None,
        yes,
    }
}

/// Programs the fake "machine" has installed; commands named `install-<tool>` install `<tool>`.
type Installed = Rc<RefCell<BTreeSet<String>>>;

fn installed(initial: &[&str]) -> Installed {
    Rc::new(RefCell::new(
        initial.iter().map(|s| s.to_string()).collect(),
    ))
}

/// A recording runner whose `install-<tool>` commands add `<tool>` to `state`.
fn runner(state: &Installed) -> Runner {
    let observed = state.clone();
    Runner::record().observe(move |spec| {
        if let Some(tool) = spec.program.strip_prefix("install-") {
            observed.borrow_mut().insert(tool.to_string());
        }
    })
}

/// A step that is met when `tool` is installed and installs it with `install-<tool>`.
fn tool_step(state: &Installed, tool: &'static str) -> Step {
    let probe_state = state.clone();
    Step::new(
        tool,
        Set::Base,
        format!("{tool} tool"),
        Privilege::None,
        move |_| {
            if probe_state.borrow().contains(tool) {
                Probed::ok()
            } else {
                Probed::missing(format!("{tool} not found"))
            }
        },
        move |cx, _| {
            let mut install = CommandSpec::new(format!("install-{tool}"));
            if cx.env.yes {
                install = install.arg("--yes");
            }
            Ok(vec![Action::Run(install)])
        },
    )
}

struct Fixture {
    env: PlanEnv,
    machine: FakeMachine,
    path: UserPath,
}

impl Fixture {
    fn new(yes: bool) -> Self {
        Self {
            env: env(yes),
            machine: FakeMachine::new(),
            path: UserPath::Windows(Box::new(MemoryPathStore::new(None))),
        }
    }
}

fn args(check: bool) -> SetupArgs {
    SetupArgs {
        check,
        ..SetupArgs::default()
    }
}

fn run(fixture: &mut Fixture, runner: &mut Runner, plan: &Plan, check: bool) -> (u8, String) {
    let mut out = Vec::new();
    let mut cx = Cx::new(&fixture.env, runner, &fixture.machine, &mut fixture.path);
    let code = run_plan(&args(check), plan, &mut cx, &mut out);
    (code, String::from_utf8(out).expect("UTF-8"))
}

#[test]
fn check_prints_each_status_and_exits_1_only_when_something_is_missing() {
    let state = installed(&["git"]);
    let plan = Plan {
        steps: vec![tool_step(&state, "git"), tool_step(&state, "cmake")],
    };
    let mut fixture = Fixture::new(false);
    let (code, text) = run(&mut fixture, &mut runner(&state), &plan, true);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("[ok     ] base: git tool"), "{text}");
    assert!(
        text.contains("[missing] base: cmake tool (cmake not found)"),
        "{text}"
    );
    assert!(
        text.contains("install-cmake"),
        "the exact command is printed:\n{text}"
    );
    assert!(
        !text.contains("install-git"),
        "a met step prints no command:\n{text}"
    );

    state.borrow_mut().insert("cmake".into());
    let (code, text) = run(&mut fixture, &mut runner(&state), &plan, true);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Everything is installed."), "{text}");
}

#[test]
fn check_installs_nothing() {
    let state = installed(&[]);
    let plan = Plan {
        steps: vec![tool_step(&state, "cmake")],
    };
    let mut fixture = Fixture::new(false);
    let mut runner = runner(&state);
    run(&mut fixture, &mut runner, &plan, true);
    assert!(runner.specs().is_empty(), "no command ran");
    assert!(state.borrow().is_empty());
}

#[test]
fn a_run_executes_only_the_missing_steps_and_a_second_run_executes_nothing() {
    let state = installed(&["git"]);
    let plan = Plan {
        steps: vec![
            tool_step(&state, "git"),
            tool_step(&state, "cmake"),
            tool_step(&state, "clang"),
        ],
    };
    let mut fixture = Fixture::new(false);

    let mut first = runner(&state);
    let (code, text) = run(&mut fixture, &mut first, &plan, false);
    assert_eq!(code, 0, "{text}");
    let programs: Vec<&str> = first.specs().iter().map(|s| s.program.as_str()).collect();
    assert_eq!(programs, ["install-cmake", "install-clang"]);
    assert!(text.contains("Installed 2 step(s)."), "{text}");

    let mut second = runner(&state);
    let (code, text) = run(&mut fixture, &mut second, &plan, false);
    assert_eq!(code, 0, "{text}");
    assert!(second.specs().is_empty(), "nothing ran the second time");
    assert!(text.contains("Nothing to do"), "{text}");
}

#[test]
fn a_failing_step_stops_the_run_and_exits_1() {
    let state = installed(&[]);
    let plan = Plan {
        steps: vec![tool_step(&state, "cmake"), tool_step(&state, "clang")],
    };
    let mut fixture = Fixture::new(false);
    let mut failing = Runner::record()
        .responder(|spec| (spec.program == "install-cmake").then(|| Outcome::failure(7)));
    let mut cx = Cx::new(
        &fixture.env,
        &mut failing,
        &fixture.machine,
        &mut fixture.path,
    );
    let report = execute(&plan, &mut cx, &mut Vec::new());
    assert!(!report.succeeded());
    assert!(report.failure().expect("failure").contains("code 7"));
    assert_eq!(
        report.steps[1].1,
        StepOutcome::NotRun,
        "later steps do not run"
    );

    let mut failing = Runner::record()
        .responder(|spec| (spec.program == "install-cmake").then(|| Outcome::failure(7)));
    let (code, _) = run(&mut fixture, &mut failing, &plan, false);
    assert_eq!(code, 1);
}

#[test]
fn a_step_that_stays_missing_after_its_install_is_a_failure() {
    let step = Step::new(
        "stuck",
        Set::Base,
        "stuck",
        Privilege::None,
        |_| Probed::missing("still absent"),
        |_, _| {
            Ok(vec![Action::Run(
                CommandSpec::new("rustc").arg("--version"),
            )])
        },
    );
    let plan = Plan { steps: vec![step] };
    let mut fixture = Fixture::new(false);
    // The install succeeds, but the re-probe still finds the requirement missing.
    let mut real = Runner::execute();
    let mut cx = Cx::new(&fixture.env, &mut real, &fixture.machine, &mut fixture.path);
    let report = execute(&plan, &mut cx, &mut Vec::new());
    let message = report.failure().expect("the re-probe fails the step");
    assert!(message.contains("still missing"), "{message}");
}

#[test]
fn yes_reaches_the_installer_arguments() {
    let state = installed(&[]);
    for (yes, expected) in [(false, vec![]), (true, vec!["--yes"])] {
        let plan = Plan {
            steps: vec![tool_step(&state, "cmake")],
        };
        state.borrow_mut().clear();
        let mut fixture = Fixture::new(yes);
        let mut recorder = runner(&state);
        run(&mut fixture, &mut recorder, &plan, false);
        assert_eq!(recorder.specs()[0].args, expected, "yes = {yes}");
    }
}

#[test]
fn all_selects_the_sets_that_apply_to_each_host() {
    let all = SetupArgs {
        all: true,
        ..SetupArgs::default()
    };
    let names = |host: HostFacts| -> Vec<&'static str> {
        selected_sets(&all, &host)
            .iter()
            .map(|s| s.name())
            .collect()
    };
    assert_eq!(
        names(host(Os::Windows, Arch::X64)),
        ["base", "web", "test", "android", "gpu"]
    );
    assert_eq!(
        names(host(Os::MacOs, Arch::Arm64)),
        ["base", "web", "test", "android", "ios", "gpu"]
    );
    assert_eq!(
        names(host(Os::Linux, Arch::X64)),
        ["base", "web", "test", "android", "gpu"]
    );
    assert_eq!(
        names(host(Os::Linux, Arch::Arm64)),
        ["base", "web", "test", "gpu"],
        "no Android set on Linux ARM64"
    );

    let web_only = SetupArgs {
        web: true,
        gpu: true,
        ..SetupArgs::default()
    };
    let sets = selected_sets(&web_only, &host(Os::Linux, Arch::X64));
    assert_eq!(sets, [Set::Base, Set::Web, Set::Gpu]);
}

#[test]
fn missing_apt_packages_of_several_steps_install_in_one_batched_root_command() {
    // dpkg-query reports `<package>\t<status>` for installed packages.
    let present: Installed = installed(&["git"]);
    let responder_state = present.clone();
    let observed = present.clone();
    let mut recorder = Runner::record()
        .with_root(false)
        .with_os(Os::Linux)
        .responder(move |spec| {
            if spec.program == "dpkg-query" {
                let stdout: String = spec
                    .args
                    .iter()
                    .filter(|a| !a.starts_with('-'))
                    .filter(|a| responder_state.borrow().contains(*a))
                    .map(|name| format!("{name}\tii \n"))
                    .collect();
                return Some(Outcome::success().with_stdout(stdout));
            }
            None
        })
        .observe(move |spec| {
            if spec.program == "apt-get" && spec.args.first().is_some_and(|a| a == "install") {
                for package in spec.args.iter().filter(|a| !a.starts_with('-')).skip(1) {
                    observed.borrow_mut().insert(package.clone());
                }
            }
        });

    let plan = Plan {
        steps: vec![
            Step::apt(
                "tools",
                Set::Base,
                "build tools",
                &["build-essential", "git"],
            ),
            Step::apt("libs", Set::Base, "libraries", &["libssl-dev", "cmake"]),
        ],
    };
    let mut fixture = Fixture::new(true);
    let (code, text) = run(&mut fixture, &mut recorder, &plan, false);
    assert_eq!(code, 0, "{text}");

    let apt: Vec<String> = recorder
        .lines()
        .iter()
        .filter(|l| l.contains("apt-get"))
        .cloned()
        .collect();
    assert_eq!(
        apt,
        [
            "sudo apt-get update",
            "sudo apt-get install -y build-essential libssl-dev cmake"
        ],
        "one update and one install for every missing package, git already present"
    );
    assert!(
        recorder
            .specs()
            .iter()
            .filter(|s| s.privilege == Privilege::Root)
            .count()
            == 2
    );

    // The plan check lists the batch once, under the first missing apt step.
    present.borrow_mut().clear();
    present.borrow_mut().insert("git".into());
    let mut dry = Runner::record()
        .with_os(Os::Linux)
        .with_root(false)
        .responder({
            let state = present.clone();
            move |spec| {
                (spec.program == "dpkg-query").then(|| {
                    let stdout: String = spec
                        .args
                        .iter()
                        .filter(|a| state.borrow().contains(*a))
                        .map(|name| format!("{name}\tii \n"))
                        .collect();
                    Outcome::success().with_stdout(stdout)
                })
            }
        });
    let (code, text) = run(&mut fixture, &mut dry, &plan, true);
    assert_eq!(code, 1);
    assert_eq!(text.matches("sudo apt-get install").count(), 1, "{text}");
    assert!(
        text.contains("(installed by the apt command above)"),
        "{text}"
    );
}

#[test]
fn a_path_step_runs_once_and_reports_the_new_shell_notice() {
    let dir = PathBuf::from(r"C:\Users\dev\.cargo\bin");
    let probe_dir = dir.clone();
    let action_dir = dir.clone();
    let plan = Plan {
        steps: vec![Step::new(
            "cargo-path",
            Set::Base,
            "cargo bin on PATH",
            Privilege::None,
            move |cx| match cx.user_path.add(&probe_dir, true) {
                Ok(PathChange::AlreadyPresent) => Probed::ok(),
                _ => Probed::missing("not on PATH"),
            },
            move |_, _| Ok(vec![Action::AddToPath(action_dir.clone())]),
        )],
    };
    let mut fixture = Fixture::new(false);
    let (code, text) = run(&mut fixture, &mut Runner::execute(), &plan, false);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Open a new shell"), "{text}");

    let (code, text) = run(&mut fixture, &mut Runner::execute(), &plan, false);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Nothing to do"), "{text}");
    assert!(
        !text.contains("Open a new shell"),
        "no change, no notice: {text}"
    );
}
