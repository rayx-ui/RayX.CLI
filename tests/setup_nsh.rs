//! The base set's `nsh` step and doctor's nsh line (NSHCMD-2, design §4.5): the step comes last in
//! the base set, after the project's toolchain; its probe is the receipt plus `nsh --version`; it
//! asks nothing; `--check` prints what it would run and installs nothing, not even a missing lock;
//! a pin that cannot be read fails the step and nothing stands in for it; `setup --update` runs
//! it per recorded project, an app without a Cargo workspace recorded by its directory; doctor
//! shows one line per pin.

mod common;

use std::path::PathBuf;

use common::Workspace;
use rayx_cli::cli::SetupArgs;
use rayx_cli::doctor::report_for;
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Outcome, Runner};
use rayx_cli::nsh::pin::{Pin, default_pin};
use rayx_cli::nsh::store::{Kind, Receipt, Store};
use rayx_cli::project::Pins;
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, environment_for, plan, run_plan};

const ID: &str = "327bb9ea65d0";
const UBUNTU: &str = "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";

fn host(os: Os, arch: Arch) -> HostFacts {
    HostFacts {
        os,
        arch,
        emulated: false,
        wsl: false,
        distro: (os == Os::Linux).then(|| Distro::parse(UBUNTU)),
    }
}

fn env(os: Os, nsh: Option<Pin>) -> PlanEnv {
    PlanEnv {
        host: host(os, Arch::Arm64),
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
        nsh: nsh.map(Ok),
    }
}

/// A machine whose nsh store is `home/rayx-home/nsh`.
fn machine(home: &Workspace) -> FakeMachine {
    FakeMachine::new()
        .with_home("/home/dev")
        .with_env("RAYX_HOME", &home.path("rayx-home").display().to_string())
        .with_env("ProgramFiles", r"C:\Program Files")
        .with_env("ProgramFiles(x86)", r"C:\Program Files (x86)")
        .with_env("LOCALAPPDATA", r"C:\Users\dev\AppData\Local")
        .with_env("TEMP", r"C:\Temp")
}

fn install_entry(home: &Workspace) -> PathBuf {
    home.write(&format!("rayx-home/nsh/{ID}/nsh/bin/nsh"), "#!");
    Store::at(home.path("rayx-home/nsh"))
        .complete(&Receipt {
            version: 1,
            id: ID.into(),
            kind: Kind::Distribution,
            url: "https://github.com/dXnSix/nsh/releases/download/fork-0.2.0-4/nsh-macos-arm64.tar.gz"
                .into(),
            reference: Some("fork-0.2.0-4".into()),
            commit: Some("327bb9ea65d08ffac4af498cc7fbbfa18a1b3623".into()),
            nsh: "0.2.0".into(),
            cli: "nsh/bin/nsh".into(),
            installed: "2026-10-24T01:30:00Z".into(),
        })
        .expect("receipt");
    home.path(&format!("rayx-home/nsh/{ID}/nsh/bin/nsh"))
}

fn check(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> (u8, String) {
    let plan = plan(env, &[Set::Base]).expect("plan");
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut out = Vec::new();
    let mut cx = Cx::new(env, runner, machine, &mut path);
    let code = run_plan(
        &SetupArgs {
            check: true,
            ..SetupArgs::default()
        },
        &plan,
        &mut cx,
        &mut out,
    );
    (code, String::from_utf8(out).expect("UTF-8"))
}

/// A recording Linux runner that reads pins from the fixtures with real Cargo queries.
fn reading_cargo() -> Runner {
    common::reading_cargo().with_os(Os::Linux)
}

fn versioned(line: &'static str) -> Runner {
    Runner::record().responder(move |spec| {
        (spec.args == ["--version"]).then(|| Outcome::success().with_stdout(line))
    })
}

#[test]
fn the_nsh_step_ends_the_base_set_after_the_toolchain_on_every_host() {
    let home = Workspace::empty();
    for os in [Os::MacOs, Os::Linux, Os::Windows] {
        let env = env(os, Some(default_pin(&home.root)));
        let ids: Vec<&str> = plan(&env, &[Set::Base, Set::Web])
            .expect("plan")
            .steps
            .iter()
            .filter(|s| s.set == Set::Base)
            .map(|s| s.id)
            .collect();
        assert_eq!(ids.last(), Some(&"nsh"), "{os:?}: {ids:?}");
        let toolchain = ids
            .iter()
            .position(|id| *id == "rust-toolchain")
            .expect("toolchain");
        assert!(toolchain < ids.len() - 1, "{os:?}: {ids:?}");
        let step = plan(&env, &[Set::Base])
            .expect("plan")
            .steps
            .pop()
            .expect("step");
        assert!(!step.prompts, "the nsh step asks nothing");
        assert_eq!(step.title, "nsh fork-0.2.0-4 (327bb9e) (default)");
    }
    // Without a pin (a guest's inner plan before discovery, tests) there is no nsh step.
    let ids: Vec<&str> = plan(&env(Os::Linux, None), &[Set::Base])
        .expect("plan")
        .steps
        .iter()
        .map(|s| s.id)
        .collect();
    assert!(!ids.contains(&"nsh"), "{ids:?}");
}

#[test]
fn check_prints_the_download_and_the_source_fallback_and_installs_nothing() {
    let home = Workspace::empty();
    let env = env(Os::MacOs, Some(default_pin(&home.root)));
    let mut runner = Runner::record();
    let (code, text) = check(&env, &machine(&home), &mut runner);
    assert_eq!(code, 1, "{text}");
    let store = home.path("rayx-home/nsh");
    assert!(
        text.contains(&format!(
            "[missing] base: nsh fork-0.2.0-4 (327bb9e) (default) ({ID} is not in {})",
            store.display()
        )),
        "{text}"
    );
    assert!(
        text.contains("download https://github.com/dXnSix/nsh/releases/download/fork-0.2.0-4/nsh-macos-arm64.tar.gz (and its .sha256)"),
        "{text}"
    );
    assert!(
        text.contains("else git fetch --depth 1 https://github.com/dXnSix/nsh 327bb9ea65d08ffac4af498cc7fbbfa18a1b3623"),
        "{text}"
    );
    assert!(
        text.contains(
            "cargo build --release --locked -p nsh-cli --target-dir target (without CARGO and RUSTUP_TOOLCHAIN)"
        ),
        "{text}"
    );
    assert!(
        !store.exists(),
        "--check writes nothing, not even the store"
    );
    assert!(
        !runner.lines().iter().any(|l| l.starts_with("curl")),
        "{:?}",
        runner.lines()
    );
}

#[test]
fn an_installed_pin_is_present_when_its_cli_answers_and_missing_when_it_does_not() {
    let home = Workspace::empty();
    let cli = install_entry(&home);
    let env = env(Os::MacOs, Some(default_pin(&home.root)));
    let (_, text) = check(
        &env,
        &machine(&home),
        &mut versioned("nsh 0.2.0 (327bb9e)\n"),
    );
    assert!(
        text.contains("[ok     ] base: nsh fork-0.2.0-4 (327bb9e) (default) (found distribution 0.2.0 (327bb9e))"),
        "{text}"
    );
    let mut runner = versioned("nsh 0.2.0 (327bb9e)\n");
    check(&env, &machine(&home), &mut runner);
    assert!(
        runner
            .lines()
            .contains(&format!("{} --version", cli.display())),
        "the probe runs the entry's CLI: {:?}",
        runner.lines()
    );
    let (_, text) = check(
        &env,
        &machine(&home),
        &mut versioned("nsh 0.2.0 (5f0c2a9)\n"),
    );
    assert!(text.contains("[missing] base: nsh fork-0.2.0-4"), "{text}");
    assert!(text.contains("built from 5f0c2a9"), "{text}");
}

#[test]
fn an_unfinished_entry_is_reported_as_unfinished() {
    let home = Workspace::empty();
    home.write(&format!("rayx-home/nsh/{ID}/source/.git/HEAD"), "x");
    let env = env(Os::Linux, Some(default_pin(&home.root)));
    let (_, text) = check(
        &env,
        &machine(&home),
        &mut Runner::record().with_os(Os::Linux),
    );
    assert!(
        text.contains(&format!("the store entry {ID} is unfinished")),
        "{text}"
    );
}

#[test]
fn yes_does_not_change_the_step() {
    let home = Workspace::empty();
    let mut quiet = env(Os::MacOs, Some(default_pin(&home.root)));
    let (_, without) = check(&quiet, &machine(&home), &mut Runner::record());
    quiet.yes = true;
    let (_, with) = check(&quiet, &machine(&home), &mut Runner::record());
    let line = |text: &str| -> Vec<String> {
        text.lines()
            .skip_while(|l| !l.contains("] base: nsh "))
            .take(2)
            .map(str::to_string)
            .collect()
    };
    assert_eq!(line(&without), line(&with));
}

#[test]
fn doctor_shows_one_line_per_pin_and_the_same_fields_in_json() {
    let home = Workspace::empty();
    install_entry(&home);
    let app = Workspace::fixture("nsh-fork-app");
    let pin = rayx_cli::nsh::pin::resolve(&mut Runner::print(), &app.root).expect("pin");
    let env = env(Os::MacOs, Some(pin));
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let report = report_for(
        &env,
        &mut versioned("nsh 0.2.0 (327bb9e)\n"),
        &machine(&home),
        &mut path,
        None,
    );
    let text = report.to_text();
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("nsh  "))
        .unwrap_or_else(|| panic!("an nsh line:\n{text}"));
    assert!(
        line.trim_start()
            .starts_with("nsh  fork-0.2.0-4 (327bb9e)  project  installed  "),
        "{line}"
    );
    assert!(line.ends_with(" B"), "with its size: {line}");
    let json = report.to_json();
    let nsh = &json["nsh"];
    assert_eq!(nsh["value"], "fork-0.2.0-4 (327bb9e)");
    assert_eq!(nsh["source"], "project");
    assert_eq!(nsh["state"], "installed");
    assert_eq!(nsh["id"], ID);
    assert!(nsh["bytes"].as_u64().is_some(), "{nsh}");
    assert_eq!(json["pins"]["nsh"]["value"], "fork-0.2.0-4 (327bb9e)");
    assert_eq!(json["pins"]["nsh"]["source"], "project");
}

#[test]
fn setup_update_installs_the_pin_of_every_recorded_project() {
    use rayx_cli::setup::update::update_host;

    let home = Workspace::empty();
    let fork = Workspace::fixture("nsh-fork-app");
    let branch = Workspace::fixture("nsh-fork-app");
    let lock = branch.read("Cargo.lock").replace(
        "?tag=fork-0.2.0-4#327bb9ea65d08ffac4af498cc7fbbfa18a1b3623",
        "?branch=main#5f0c2a91be4700000000000000000000000000aa",
    );
    branch.write("Cargo.lock", &lock);
    let config = home.path("config");
    let record = format!(
        r#"{{"version":1,"sets":["base"],"roots":[{:?},{:?}]}}"#,
        fork.root.display().to_string(),
        branch.root.display().to_string()
    );
    let machine = machine(&home)
        .with_env("XDG_CONFIG_HOME", &config.display().to_string())
        .with_text(config.join("rayx/setup.json"), &record)
        .with_dir(&fork.root)
        .with_dir(&branch.root);
    let mut runner = reading_cargo();
    let mut out = Vec::new();
    let args = SetupArgs {
        check: true,
        update: true,
        ..SetupArgs::default()
    };
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let result = update_host(
        &args,
        &host(Os::Linux, Arch::X64),
        &mut runner,
        &machine,
        &mut path,
        &mut out,
        None,
    );
    assert!(result.recorded);
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(
        text.contains("base: nsh fork-0.2.0-4 (327bb9e) (project)"),
        "{text}"
    );
    assert!(
        text.contains("base: nsh main (5f0c2a9) (project)"),
        "{text}"
    );
}

#[test]
fn an_unreadable_pin_fails_its_step_and_nothing_stands_in_for_it() {
    let home = Workspace::empty();
    install_entry(&home);
    let app = Workspace::fixture("nsh-fork-app");
    app.append(
        "Cargo.lock",
        "\n[[package]]\nname = \"nsh-platform-gpui\"\nversion = \"0.2.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
    );
    let env = environment_for(
        host(Os::MacOs, Arch::Arm64),
        None,
        &app.root,
        false,
        &mut Runner::print(),
    );
    let error = match &env.nsh {
        Some(Err(error)) => error.clone(),
        other => panic!("an unreadable pin: {other:?}"),
    };
    assert!(error.reason.contains("two sources"), "{error}");

    let plan = plan(&env, &[Set::Base]).expect("plan");
    let last = plan.steps.last().expect("steps");
    assert_eq!(last.id, "nsh");
    assert_eq!(last.title, "nsh (the project's pin cannot be read)");
    let (code, text) = check(
        &env,
        &machine(&home),
        &mut versioned("nsh 0.2.0 (327bb9e)\n"),
    );
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("two sources"), "{text}");
    assert!(
        !text.contains("fork-0.2.0-4 (327bb9e) (default)"),
        "the default never stands in: {text}"
    );

    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut runner = Runner::record();
    let fake = machine(&home);
    let mut cx = Cx::new(&env, &mut runner, &fake, &mut path);
    let rayx_cli::setup::Install::Actions(build) = &last.install else {
        panic!("an action builder");
    };
    let installed = build(
        &mut cx,
        &rayx_cli::setup::Probed::missing(error.reason.clone()),
    );
    assert!(
        installed.is_err_and(|e| e.to_string().contains("the nsh pin cannot be read")),
        "its install fails with the reason"
    );

    let report = report_for(
        &env,
        &mut versioned("nsh 0.2.0 (327bb9e)\n"),
        &machine(&home),
        &mut UserPath::Windows(Box::new(MemoryPathStore::new(None))),
        None,
    );
    let text = report.to_text();
    assert!(
        text.contains("  nsh  the pin cannot be read: ") && text.contains("two sources"),
        "{text}"
    );
    assert!(
        report.to_json()["nsh"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("two sources"))
    );
}

#[test]
fn check_and_doctor_write_no_missing_lock_and_setup_does() {
    let app = Workspace::fixture("nsh-path-app");
    let dir = app.path("app");
    let env = environment_for(
        host(Os::MacOs, Arch::Arm64),
        None,
        &dir,
        false,
        &mut Runner::print(),
    );
    assert!(
        matches!(&env.nsh, Some(Err(error)) if error.reason.contains("has no Cargo.lock yet")),
        "{:?}",
        env.nsh
    );
    assert!(!app.path("Cargo.lock").exists(), "a dry run wrote nothing");
    let env = environment_for(
        host(Os::MacOs, Arch::Arm64),
        None,
        &dir,
        false,
        &mut Runner::execute(),
    );
    assert!(matches!(env.nsh, Some(Ok(_))), "{:?}", env.nsh);
    assert!(
        app.path("Cargo.lock").is_file(),
        "a real run writes the lock"
    );
}

#[test]
fn an_app_without_a_cargo_workspace_is_recorded_by_its_directory_and_updated() {
    use rayx_cli::setup::after_run;
    use rayx_cli::setup::update::update_host;

    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-ts-app");
    let config = home.path("config");
    let machine = machine(&home)
        .with_env("XDG_CONFIG_HOME", &config.display().to_string())
        .with_dir(&app.root);
    let env = environment_for(
        host(Os::Linux, Arch::X64),
        None,
        &app.root,
        false,
        &mut Runner::print(),
    );
    assert_eq!(env.project_root, None, "no Cargo workspace");
    after_run(&SetupArgs::default(), 0, &env, &[Set::Base], &machine);
    // The record is written to the configuration directory; the fake machine reads what it is given.
    let written = std::fs::read_to_string(config.join("rayx/setup.json")).expect("recorded");
    let setup: rayx_cli::setup::records::SetupFile =
        serde_json::from_str(&written).expect("setup.json");
    assert_eq!(setup.roots, [app.root.display().to_string()]);
    let machine = machine.with_text(config.join("rayx/setup.json"), &written);

    let mut out = Vec::new();
    let result = update_host(
        &SetupArgs {
            check: true,
            update: true,
            ..SetupArgs::default()
        },
        &host(Os::Linux, Arch::X64),
        &mut reading_cargo(),
        &machine,
        &mut UserPath::Windows(Box::new(MemoryPathStore::new(None))),
        &mut out,
        None,
    );
    assert!(result.recorded);
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(
        text.contains("base: nsh fork-0.2.0-4 (327bb9e) (rayx.toml)"),
        "--update reads the recorded app's pin: {text}"
    );
    let keep = rayx_cli::nsh::kept_ids(&mut Runner::print(), &machine, &host(Os::Linux, Arch::X64))
        .expect("pins read");
    assert_eq!(
        keep,
        [ID],
        "prune keeps the app's entry (here also the default)"
    );
}

#[test]
fn a_pin_unreadable_when_planned_is_read_again_when_its_step_runs() {
    use rayx_cli::nsh::pin::Unreadable;
    use rayx_cli::setup::{Action, Install};

    // A fresh machine: the plan was made before Rust was installed, so the pin was unknown; by
    // the time the step runs, the project's own pin reads.
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    let mut env = env(Os::MacOs, None);
    env.nsh = Some(Err(Unreadable {
        dir: app.root.clone(),
        reason: "cargo could not run".into(),
    }));
    let plan = plan(&env, &[Set::Base]).expect("plan");
    let last = plan.steps.last().expect("steps");
    assert_eq!(last.id, "nsh");
    let mut runner = versioned("nsh 0.2.0 (327bb9e)\n");
    let fake = machine(&home);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = Cx::new(&env, &mut runner, &fake, &mut path);
    let probed = (last.probe)(&mut cx);
    assert!(!probed.satisfied);
    assert!(
        probed.detail.contains("327bb9ea65d0 is not in"),
        "the probe reads the pin again: {probed:?}"
    );
    let Install::Actions(build) = &last.install else {
        panic!("an action builder");
    };
    let actions = build(&mut cx, &probed).expect("the pin reads now");
    assert!(
        matches!(actions.as_slice(), [Action::Custom { description, .. }]
            if description.contains("releases/download/fork-0.2.0-4/nsh-macos-arm64.tar.gz")),
        "the project's pin is installed"
    );
}

#[test]
fn right_after_rustup_is_installed_the_step_finds_rust_in_cargo_bin() {
    use rayx_cli::nsh::pin::{Reference, Source};
    use rayx_cli::setup::{Action, Install};

    // The first `rayx setup` on a fresh machine: rustup went into ~/.cargo/bin, which this
    // process's PATH does not have (`which` finds nothing).
    let home = Workspace::empty();
    let pin = Pin {
        source: Source::Git {
            url: "https://github.com/dXnSix/nsh".into(),
            reference: Reference::Branch("main".into()),
            commit: "327bb9ea65d08ffac4af498cc7fbbfa18a1b3623".into(),
        },
        ..default_pin(&home.root)
    };
    let env = env(Os::MacOs, Some(pin.clone()));
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("RAYX_HOME", &home.path("rayx-home").display().to_string())
        .with_file("/Users/dev/.cargo/bin/rustup")
        .with_file("/Users/dev/.cargo/bin/cargo");
    let checked_out = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = checked_out.clone();
    let mut runner = Runner::record()
        .observe(move |spec| {
            if spec.program == "git" && spec.args.first().is_some_and(|a| a == "checkout") {
                seen.set(true);
            }
        })
        .responder(move |spec| {
            if spec.args == ["--version"] {
                return Some(Outcome::success().with_stdout("nsh 0.2.0 (327bb9e)\n"));
            }
            (spec.program == "git" && spec.args == ["rev-parse", "HEAD"]).then(|| {
                if checked_out.get() {
                    Outcome::success().with_stdout("327bb9ea65d08ffac4af498cc7fbbfa18a1b3623\n")
                } else {
                    Outcome::failure(128)
                }
            })
        });
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let step = rayx_cli::setup::nsh::step(&pin);
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    let probed = (step.probe)(&mut cx);
    let Install::Actions(build) = &step.install else {
        panic!("an action builder");
    };
    let actions = build(&mut cx, &probed).expect("actions");
    let [Action::Custom { run, .. }] = actions.as_slice() else {
        panic!("one install action");
    };
    run(&mut cx).expect("installs");
    // Compared as paths, so `\\` on Windows matches.
    let programs: Vec<PathBuf> = runner
        .specs()
        .iter()
        .map(|spec| PathBuf::from(&spec.program))
        .filter(|program| program.ends_with("rustup") || program.ends_with("cargo"))
        .collect();
    assert_eq!(
        programs,
        [
            PathBuf::from("/Users/dev/.cargo/bin/rustup"),
            PathBuf::from("/Users/dev/.cargo/bin/cargo")
        ],
        "the rustup and cargo setup just installed"
    );
}
