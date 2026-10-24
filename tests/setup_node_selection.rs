//! Which Node the test set uses when several are installed: an old `node` earlier on PATH (nvm's)
//! must not hide the pinned Homebrew `node@22`, which is keg-only and never on PATH. Found on a
//! Mac that had Node 17 from nvm first on PATH: the Node step failed right after installing 22.

use std::path::PathBuf;

use rayx_cli::host::path_env::UserPath;
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::test_tools::{find_tool, node_command, select_node};
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, plan};

const NVM_NODE: &str = "/Users/dev/.nvm/versions/node/v17.2.0/bin/node";
const KEG_NODE: &str = "/opt/homebrew/opt/node@22/bin/node";

fn mac_env() -> PlanEnv {
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
        yes: false,
    }
}

/// Each node answers with its own version, keyed on the absolute path it is started by.
fn runner() -> Runner {
    Runner::record()
        .with_os(Os::MacOs)
        .with_root(false)
        .responder(|spec| {
            // Paths are joined with the host's separator: compare them with forward slashes.
            match spec.program.replace('\\', "/").as_str() {
                NVM_NODE => Some(Outcome::success().with_stdout("v17.2.0\n")),
                KEG_NODE => Some(Outcome::success().with_stdout("v22.23.3\n")),
                _ => None,
            }
        })
}

fn user_path() -> UserPath {
    UserPath::Profiles {
        files: vec![PathBuf::from("/Users/dev/.zprofile")],
        home: PathBuf::from("/Users/dev"),
    }
}

#[test]
fn the_pinned_keg_node_wins_over_an_older_node_earlier_on_path() {
    let env = mac_env();
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_program("node", NVM_NODE)
        .with_file(NVM_NODE)
        .with_file(KEG_NODE);
    let mut runner = runner();
    let mut path = user_path();
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);

    let (node, major) = select_node(&mut cx).expect("a node");
    assert_eq!(node, PathBuf::from(KEG_NODE));
    assert_eq!(major, Some(22));
    assert_eq!(find_tool(&mut cx, "node"), Some(PathBuf::from(KEG_NODE)));

    // Tools started through node_command see the selected Node first on their PATH.
    let spec = node_command(&mut cx, "corepack", &["enable"]);
    let path_value = spec
        .env
        .iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.clone())
        .expect("PATH");
    assert!(
        path_value
            .replace('\\', "/")
            .starts_with("/opt/homebrew/opt/node@22/bin"),
        "{path_value}"
    );
}

#[test]
fn an_old_path_node_alone_is_still_reported_with_its_version() {
    let env = mac_env();
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_program("node", NVM_NODE)
        .with_file(NVM_NODE);
    let mut runner = runner();
    let mut path = user_path();
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);

    assert_eq!(
        select_node(&mut cx),
        Some((PathBuf::from(NVM_NODE), Some(17)))
    );
    let plan = plan(&env, &[Set::Test]).expect("plan");
    let step = plan.steps.iter().find(|s| s.id == "node").expect("node");
    let probed = (step.probe)(&mut cx);
    assert!(!probed.satisfied);
    assert_eq!(probed.found.as_deref(), Some("17"));
}

#[test]
fn the_node_step_is_met_by_the_keg_node_behind_an_old_one() {
    let env = mac_env();
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_program("node", NVM_NODE)
        .with_file(NVM_NODE)
        .with_file(KEG_NODE);
    let mut runner = runner();
    let mut path = user_path();
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    let plan = plan(&env, &[Set::Test]).expect("plan");
    let step = plan.steps.iter().find(|s| s.id == "node").expect("node");
    let probed = (step.probe)(&mut cx);
    assert!(probed.satisfied, "{probed:?}");
    assert_eq!(probed.found.as_deref(), Some("22"));
}
