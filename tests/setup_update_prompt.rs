//! `rayx setup` asks before it updates something that is installed but outdated or unusable (an
//! old Node, a Homebrew that crashes on this macOS), and never before a first install. `--yes`
//! updates without asking; without a terminal the update is declined and reported.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::UserPath;
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::{
    Cx, FakeMachine, PlanEnv, Probed, Report, Set, StepOutcome, execute, plan, run_plan,
};

const NVM_NODE: &str = "/Users/dev/.nvm/versions/node/v17.2.0/bin/node";
const BREW: &str = "/opt/homebrew/bin/brew";

fn env(yes: bool) -> PlanEnv {
    PlanEnv {
        host: HostFacts {
            os: Os::MacOs,
            arch: Arch::Arm64,
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

fn user_path() -> UserPath {
    UserPath::Profiles {
        files: vec![PathBuf::from("/Users/dev/.zprofile")],
        home: PathBuf::from("/Users/dev"),
    }
}

/// A Mac with only Node 17 installed, which is older than the pinned 22.
fn old_node_machine() -> FakeMachine {
    FakeMachine::new()
        .with_home("/Users/dev")
        .with_program("node", NVM_NODE)
        .with_file(NVM_NODE)
}

fn runner_with_node_17() -> Runner {
    Runner::record()
        .with_os(Os::MacOs)
        .with_root(false)
        .responder(|spec| match spec.program.as_str() {
            NVM_NODE => Some(Outcome::success().with_stdout("v17.2.0\n")),
            _ => None,
        })
}

fn ran(runner: &Runner, needle: &str) -> bool {
    runner.lines().iter().any(|line| line.contains(needle))
}

type Questions = Rc<RefCell<Vec<String>>>;

/// Runs `execute` with `prompt` as the update prompt, in a scope of its own so the runner can be
/// inspected afterwards.
fn run(
    env: &PlanEnv,
    sets: &[Set],
    machine: &FakeMachine,
    runner: &mut Runner,
    mut prompt: impl FnMut(&str) -> bool,
) -> Report {
    let plan = plan(env, sets).expect("plan");
    let mut path = user_path();
    let mut cx = Cx::new(env, runner, machine, &mut path).with_update_prompt(&mut prompt);
    execute(&plan, &mut cx, &mut Vec::new())
}

#[test]
fn declining_the_update_of_an_outdated_requirement_installs_nothing_and_is_reported() {
    let env = env(false);
    let machine = old_node_machine();
    let mut runner = runner_with_node_17();
    let asked: Questions = Rc::default();
    let sink = asked.clone();
    let report = run(&env, &[Set::Test], &machine, &mut runner, move |q| {
        sink.borrow_mut().push(q.to_string());
        false
    });

    let questions = asked.borrow();
    assert_eq!(questions.len(), 1, "{questions:?}");
    assert!(
        questions[0].contains("Node.js 22") && questions[0].contains("found 17"),
        "{questions:?}"
    );
    assert!(!ran(&runner, "node@22"), "{:?}", runner.lines());
    let declined = report.declined();
    assert_eq!(declined.len(), 1, "{declined:?}");
    assert!(
        declined[0].contains("brew install node@22") && declined[0].contains("--yes"),
        "the fix is spelled out: {declined:?}"
    );
    assert!(!report.succeeded());
    let node = report
        .steps
        .iter()
        .find(|(id, _)| *id == "node")
        .expect("node step");
    assert!(matches!(node.1, StepOutcome::Declined(_)), "{node:?}");
}

#[test]
fn run_plan_exits_nonzero_when_an_update_was_declined() {
    let env = env(false);
    let plan = plan(&env, &[Set::Test]).expect("plan");
    let machine = old_node_machine();
    let mut runner = runner_with_node_17();
    let mut path = user_path();
    let mut decline = |_: &str| false;
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path).with_update_prompt(&mut decline);
    let code = run_plan(
        &SetupArgs {
            test: true,
            ..SetupArgs::default()
        },
        &plan,
        &mut cx,
        &mut Vec::new(),
    );
    assert_eq!(code, 1);
}

#[test]
fn accepting_runs_the_update() {
    let env = env(false);
    let machine = old_node_machine();
    let mut runner = runner_with_node_17();
    let report = run(&env, &[Set::Test], &machine, &mut runner, |_| true);
    assert!(report.declined().is_empty());
    assert!(ran(&runner, "brew install node@22"), "{:?}", runner.lines());
}

#[test]
fn yes_updates_without_asking() {
    let env = env(true);
    let machine = old_node_machine();
    let mut runner = runner_with_node_17();
    let report = run(&env, &[Set::Test], &machine, &mut runner, |q| {
        panic!("--yes must not ask: {q}")
    });
    assert!(report.declined().is_empty());
    assert!(ran(&runner, "brew install node@22"), "{:?}", runner.lines());
}

#[test]
fn a_first_install_is_never_asked_about() {
    let env = env(false);
    let machine = FakeMachine::new().with_home("/Users/dev");
    let mut runner = Runner::record().with_os(Os::MacOs).with_root(false);
    let report = run(&env, &[Set::Test], &machine, &mut runner, |q| {
        panic!("a missing requirement is not an update: {q}")
    });
    assert!(report.declined().is_empty());
    assert!(ran(&runner, "brew install node@22"), "{:?}", runner.lines());
}

fn brew_machine() -> FakeMachine {
    FakeMachine::new().with_home("/Users/dev").with_file(BREW)
}

fn runner_with_brew(config_works: bool) -> Runner {
    Runner::record()
        .with_os(Os::MacOs)
        .with_root(false)
        .responder(move |spec| {
            (spec.program == BREW && spec.args == ["config"]).then(|| {
                if config_works {
                    Outcome::success().with_stdout("HOMEBREW_VERSION: 7.0.9\n")
                } else {
                    let mut failed = Outcome::failure(1);
                    failed.stderr = "/opt/homebrew/Library/Homebrew/os/mac/version.rb:41:in \
                         `initialize': unknown or unsupported macOS version: \"26.6.2\" \
                         (MacOSVersionError)\n"
                        .to_string();
                    failed
                }
            })
        })
}

fn homebrew_probe(runner: &mut Runner, machine: &FakeMachine, env: &PlanEnv) -> Probed {
    let plan = plan(env, &[Set::Base]).expect("plan");
    let step = plan
        .steps
        .iter()
        .find(|step| step.id == "homebrew")
        .expect("homebrew step");
    let mut path = user_path();
    let mut cx = Cx::new(env, runner, machine, &mut path);
    (step.probe)(&mut cx)
}

#[test]
fn a_homebrew_that_cannot_run_is_outdated_not_present() {
    let env = env(false);
    let machine = brew_machine();

    let working = homebrew_probe(&mut runner_with_brew(true), &machine, &env);
    assert!(working.satisfied && !working.is_outdated(), "{working:?}");

    let broken = homebrew_probe(&mut runner_with_brew(false), &machine, &env);
    assert!(!broken.satisfied && broken.is_outdated(), "{broken:?}");
    assert!(
        broken.detail.contains("unsupported macOS version"),
        "the reason is shown: {broken:?}"
    );

    let absent = homebrew_probe(
        &mut runner_with_brew(true),
        &FakeMachine::new().with_home("/Users/dev"),
        &env,
    );
    assert!(!absent.satisfied && !absent.is_outdated(), "{absent:?}");
}

#[test]
fn an_outdated_homebrew_is_updated_in_place_after_the_answer() {
    let env = env(false);
    let machine = brew_machine();

    let mut declined_runner = runner_with_brew(false);
    let report = run(
        &env,
        &[Set::Base],
        &machine,
        &mut declined_runner,
        |question| {
            assert!(
                question.contains("Homebrew is outdated or unusable"),
                "{question}"
            );
            false
        },
    );
    let homebrew = report
        .steps
        .iter()
        .find(|(id, _)| *id == "homebrew")
        .expect("homebrew");
    assert!(matches!(&homebrew.1, StepOutcome::Declined(m) if m.contains("brew update --force")));
    assert!(!ran(&declined_runner, "update --force"));

    let mut accepted_runner = runner_with_brew(false);
    run(&env, &[Set::Base], &machine, &mut accepted_runner, |_| true);
    assert!(
        ran(&accepted_runner, "brew update --force"),
        "{:?}",
        accepted_runner.lines()
    );
    assert!(
        !ran(&accepted_runner, "Homebrew/install"),
        "an existing Homebrew is not reinstalled: {:?}",
        accepted_runner.lines()
    );
}
