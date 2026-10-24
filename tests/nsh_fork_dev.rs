//! Fork development (RUN-3, design §4.6): `RAYX_NSH=<path>` runs that binary instead of the pin's,
//! with a warning on every command, and a path dependency on an NSH checkout builds and runs that
//! checkout's CLI. Neither touches the store.

mod common;

use std::path::Path;

use clap::Parser;
use common::Workspace;
use rayx_cli::app::{Launch, prepare};
use rayx_cli::cli::{AppArgs, Cli, Command};
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, HostFacts, Os, Runner};
use rayx_cli::nsh::pin::resolve;
use rayx_cli::setup::{FakeMachine, Machine, PlanEnv, Set};

fn app_args(argv: &[&str]) -> AppArgs {
    let mut full = vec!["rayx", "app"];
    full.extend_from_slice(argv);
    match Cli::try_parse_from(full).expect("parses").command {
        Command::App(args) => args,
        other => panic!("{other:?}"),
    }
}

fn mac() -> HostFacts {
    HostFacts {
        os: Os::MacOs,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

/// What `prepare` handed the prerequisite step: whether the plan carried an nsh pin.
struct Prepared {
    launch: Launch,
    nsh_step: Vec<bool>,
    warnings: String,
    runner: Runner,
}

fn run_prepare(args: &AppArgs, dir: &Path, machine: &FakeMachine) -> Prepared {
    let mut nsh_step = Vec::new();
    let mut ensure =
        |env: &PlanEnv, _: &[Set], _: &Path, _: &mut Runner, _: &dyn Machine, _: &mut UserPath| {
            nsh_step.push(env.nsh.is_some());
            Ok(0)
        };
    // The pin is read through the command's runner; this one answers Cargo's queries for real.
    let mut runner = common::reading_cargo();
    let mut err = Vec::new();
    let launch = prepare(
        args,
        dir,
        &mac(),
        &mut runner,
        machine,
        &mut UserPath::Windows(Box::new(MemoryPathStore::new(None))),
        &mut ensure,
        &mut err,
    )
    .expect("prepared");
    Prepared {
        launch,
        nsh_step,
        warnings: String::from_utf8(err).expect("UTF-8"),
        runner,
    }
}

#[test]
fn rayx_nsh_runs_that_binary_and_warns_naming_the_pin_it_replaces() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    // An absolute path on every host (`/Users/dev/…` has no drive on Windows).
    let built = home.path("nsh/target/release/nsh");
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("RAYX_HOME", &home.path("rayx-home").display().to_string())
        .with_env("RAYX_NSH", &built.display().to_string());
    for command in ["build", "run"] {
        let prepared = run_prepare(&app_args(&[command, "."]), &app.root, &machine);
        assert_eq!(prepared.launch.program, built);
        assert_eq!(
            prepared.warnings.trim(),
            format!(
                "rayx: RAYX_NSH={} replaces the project's nsh 327bb9ea65d0",
                built.display()
            ),
            "{command}: the warning comes with every command"
        );
        assert_eq!(
            prepared.nsh_step,
            [false],
            "the pin is neither installed nor checked"
        );
        assert_eq!(
            prepared.launch.unset,
            ["CARGO"],
            "the rest of the launch is the same"
        );
    }
    assert!(
        !home.path("rayx-home").exists(),
        "the store is never touched"
    );
}

#[test]
fn an_empty_rayx_nsh_is_no_replacement() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    home.write("rayx-home/nsh/327bb9ea65d0/nsh/bin/nsh", "#!");
    rayx_cli::nsh::store::Store::at(home.path("rayx-home/nsh"))
        .complete(&rayx_cli::nsh::store::Receipt {
            version: 1,
            id: "327bb9ea65d0".into(),
            kind: rayx_cli::nsh::store::Kind::Distribution,
            url: "u".into(),
            reference: None,
            commit: None,
            nsh: "0.2.0".into(),
            cli: "nsh/bin/nsh".into(),
            installed: "2026-10-24T01:30:00Z".into(),
        })
        .expect("receipt");
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("RAYX_HOME", &home.path("rayx-home").display().to_string())
        .with_env("RAYX_NSH", "  ");
    let prepared = run_prepare(&app_args(&["build"]), &app.root, &machine);
    assert_eq!(
        prepared.launch.program,
        home.path("rayx-home/nsh/327bb9ea65d0/nsh/bin/nsh")
    );
    assert!(prepared.warnings.is_empty(), "{}", prepared.warnings);
    assert_eq!(prepared.nsh_step, [true]);
}

#[test]
fn a_path_pin_builds_the_checkouts_cli_and_runs_it_writing_nothing_to_the_store() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-path-app");
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("RAYX_HOME", &home.path("rayx-home").display().to_string());
    // The fixture has no lock; a real `rayx app` writes it first (an executing runner).
    resolve(&mut Runner::execute(), &app.path("app")).expect("the lock is written");
    let prepared = run_prepare(
        &app_args(&["run", "--target", "macos"]),
        &app.path("app"),
        &machine,
    );
    let checkout = app.path("nsh");
    assert_eq!(
        prepared.launch.program,
        checkout.join("target").join("release").join("nsh")
    );
    assert_eq!(
        prepared.launch.cwd,
        app.path("app"),
        "nsh runs where the pin was read"
    );
    let build = prepared
        .runner
        .specs()
        .iter()
        .find(|spec| spec.program == "cargo" && spec.args.first().is_some_and(|a| a == "build"))
        .expect("a build");
    assert_eq!(
        build.args,
        [
            "build",
            "--release",
            "-p",
            "nsh-cli",
            "--target-dir",
            "target"
        ],
        "the CLI lands where rayx runs it, whatever CARGO_TARGET_DIR says"
    );
    assert_eq!(build.cwd.as_deref(), Some(checkout.as_path()));
    assert!(build.unset.contains(&"CARGO".to_string()));
    assert!(
        build.unset.contains(&"RUSTUP_TOOLCHAIN".to_string()),
        "the checkout builds with its own toolchain"
    );
    assert_eq!(prepared.nsh_step, [true]);
    assert!(
        prepared
            .warnings
            .contains("building the nsh CLI of the checkout"),
        "{}",
        prepared.warnings
    );
    assert!(
        !home.path("rayx-home").exists(),
        "a path checkout is never copied into the store"
    );
}

#[test]
fn the_setup_step_of_a_path_pin_needs_nothing_installed() {
    let app = Workspace::fixture("nsh-path-app");
    let pin = resolve(&mut Runner::execute(), &app.path("app")).expect("pin");
    let step = rayx_cli::setup::nsh::step(&pin);
    let env = PlanEnv {
        host: mac(),
        pins: rayx_cli::project::Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
        nsh: Some(Ok(pin)),
    };
    let mut runner = Runner::record();
    let machine = FakeMachine::new().with_home("/Users/dev");
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = rayx_cli::setup::Cx::new(&env, &mut runner, &machine, &mut path);
    let probed = (step.probe)(&mut cx);
    assert!(probed.satisfied, "{probed:?}");
    assert!(probed.found.expect("found").starts_with("checkout "));
}

#[test]
fn a_relative_rayx_nsh_is_read_against_where_rayx_started_not_c_or_path() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    let started = Workspace::empty();
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("RAYX_HOME", &home.path("rayx-home").display().to_string())
        .with_env("RAYX_NSH", "build/nsh");
    let args = app_args(&["-C", app.root.to_str().expect("UTF-8"), "build"]);
    let prepared = run_prepare(&args, &started.root, &machine);
    assert_eq!(
        prepared.launch.program,
        started.root.join("build").join("nsh")
    );
    assert_eq!(prepared.launch.cwd, app.root, "nsh still runs in -C");
    let bare = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("RAYX_NSH", "nsh");
    let prepared = run_prepare(&app_args(&["build"]), &app.root, &bare);
    assert_eq!(
        prepared.launch.program,
        app.path("nsh"),
        "a bare name is a file here, never the nsh on PATH"
    );
}

#[test]
fn rayx_nsh_path_prints_the_replacement_with_the_warning() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    let assert = assert_cmd::cargo::cargo_bin_cmd!("rayx")
        .args(["nsh", "path"])
        .current_dir(&app.root)
        .env("RAYX_HOME", home.path("rayx-home"))
        .env("RAYX_NSH", "nsh-dev")
        .env_remove("CARGO")
        .assert()
        .success();
    let output = assert.get_output();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        app.path("nsh-dev").display().to_string()
    );
    let warning = String::from_utf8_lossy(&output.stderr);
    assert!(
        warning.contains(&format!(
            "rayx: RAYX_NSH={} replaces the project's nsh 327bb9ea65d0",
            app.path("nsh-dev").display()
        )),
        "{warning}"
    );
    assert!(!home.path("rayx-home").exists(), "nothing installed");
}
