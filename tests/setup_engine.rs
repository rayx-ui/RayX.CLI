//! The setup engine over synthetic plans: status and exit codes, execute-only-missing,
//! idempotence, `--yes`, `--all` per host, and root-step batching.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use predicates::prelude::*;
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
        tools_node: None,
        nsh: None,
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
        ["base", "web", "test", "android"]
    );
    assert_eq!(
        names(host(Os::MacOs, Arch::Arm64)),
        ["base", "web", "test", "android", "ios"]
    );
    assert_eq!(
        names(host(Os::Linux, Arch::X64)),
        ["base", "web", "test", "android"]
    );
    assert_eq!(
        names(host(Os::Linux, Arch::Arm64)),
        ["base", "web", "test"],
        "no Android set on Linux ARM64"
    );

    let web_only = SetupArgs {
        web: true,
        ios: true,
        ..SetupArgs::default()
    };
    let sets = selected_sets(&web_only, &host(Os::Linux, Arch::X64));
    assert_eq!(sets, [Set::Base, Set::Web, Set::Ios]);
}

#[test]
fn there_is_no_gpu_set() {
    assert_eq!(
        Set::ALL,
        [Set::Base, Set::Web, Set::Test, Set::Android, Set::Ios]
    );
    let names: Vec<&str> = Set::ALL.iter().map(|set| set.name()).collect();
    assert_eq!(names, ["base", "web", "test", "android", "ios"]);
    assert_eq!(Set::from_name("gpu"), None);
    for set in Set::ALL {
        assert_eq!(Set::from_name(set.name()), Some(set));
    }
}

#[test]
fn setup_gpu_is_an_unknown_argument() {
    use assert_cmd::cargo::cargo_bin_cmd;
    use predicates::str::contains;

    for args in [&["setup", "--gpu"][..], &["setup", "--all", "--gpu"][..]] {
        cargo_bin_cmd!("rayx")
            .args(args)
            .assert()
            .code(2)
            .stderr(contains("unexpected argument '--gpu'").and(contains("Usage:")));
    }
    use clap::Parser;
    assert!(rayx_cli::cli::Cli::try_parse_from(["rayx", "setup", "--gpu"]).is_err());
}

#[test]
fn the_source_has_no_gpu_set_and_no_app_pipeline() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(!src.join("setup/gpu.rs").exists(), "no setup::gpu");
    // The old app pipeline's modules stay behind (D13); `rayx app` forwards to nsh.
    for module in [
        "descriptor",
        "graph",
        "desktop",
        "wasm",
        "android",
        "ios",
        "assets",
        "package_assets",
        "playwright",
        "args",
        "context",
        "process",
        "fs_util",
        "test_support",
    ] {
        assert!(
            !src.join("app").join(format!("{module}.rs")).exists(),
            "src/app/{module}.rs is a module of the dropped pipeline"
        );
    }
    let setup = std::fs::read_to_string(src.join("setup/mod.rs")).expect("setup/mod.rs");
    assert!(!setup.contains("mod gpu"), "setup declares no gpu module");
    assert!(!setup.contains("Gpu"), "setup names no Gpu set");
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

// ---------------------------------------------------------------------------------------------
// setup.json: what a run records, at its 1.x location and in schema version 1

mod record {
    use std::path::{Path, PathBuf};

    use rayx_cli::cli::SetupArgs;
    use rayx_cli::host::{Distro, HostFacts, Os};
    use rayx_cli::project::Pins;
    use rayx_cli::setup::records::{SetupFile, read_setup, record_setup, setup_path};
    use rayx_cli::setup::{FakeMachine, PlanEnv, Set, after_run};

    fn env(root: Option<&str>) -> PlanEnv {
        PlanEnv {
            host: HostFacts {
                os: Os::Linux,
                arch: rayx_cli::host::Arch::X64,
                emulated: false,
                wsl: false,
                distro: Some(Distro::parse("ID=ubuntu\nVERSION_ID=\"24.04\"\n")),
            },
            pins: Pins::resolve(None),
            project_root: root.map(PathBuf::from),
            tools_node: None,
            nsh: None,
            yes: false,
        }
    }

    fn machine(config: &Path) -> FakeMachine {
        FakeMachine::new()
            .with_home("/home/dev")
            .with_env("XDG_CONFIG_HOME", &config.display().to_string())
    }

    fn recorded(config: &Path) -> SetupFile {
        let text = std::fs::read_to_string(config.join("rayx/setup.json")).expect("setup.json");
        serde_json::from_str(&text).expect("JSON")
    }

    #[test]
    fn a_successful_run_records_its_sets_and_root_in_version_1_and_later_runs_add_to_them() {
        let dir = tempfile::tempdir().expect("temp dir");
        let machine = machine(dir.path());
        let web = SetupArgs {
            web: true,
            ..SetupArgs::default()
        };
        after_run(
            &web,
            0,
            &env(Some("/work/app")),
            &[Set::Base, Set::Web],
            &machine,
        );
        after_run(
            &SetupArgs::default(),
            0,
            &env(Some("/work/other")),
            &[Set::Base, Set::Test],
            &machine,
        );
        after_run(
            &web,
            0,
            &env(Some("/work/app")),
            &[Set::Base, Set::Web],
            &machine,
        );

        let setup = recorded(dir.path());
        assert_eq!(setup.version, 1);
        assert_eq!(setup.sets, ["base", "web", "test"], "a union, each once");
        assert_eq!(setup.roots, ["/work/app", "/work/other"]);
    }

    #[test]
    fn check_update_and_failed_runs_record_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let machine = machine(dir.path());
        for (args, code) in [
            (
                SetupArgs {
                    check: true,
                    ..SetupArgs::default()
                },
                0,
            ),
            (
                SetupArgs {
                    update: true,
                    ..SetupArgs::default()
                },
                0,
            ),
            (SetupArgs::default(), 1),
        ] {
            after_run(&args, code, &env(Some("/work/app")), &[Set::Base], &machine);
        }
        assert!(!dir.path().join("rayx").exists());
    }

    #[test]
    fn a_run_outside_a_project_records_the_sets_without_a_root() {
        let dir = tempfile::tempdir().expect("temp dir");
        after_run(
            &SetupArgs::default(),
            0,
            &env(None),
            &[Set::Base],
            &machine(dir.path()),
        );
        let setup = recorded(dir.path());
        assert_eq!(setup.sets, ["base"]);
        assert!(setup.roots.is_empty());
    }

    #[test]
    fn an_unparsable_record_is_reported_and_left_untouched() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("rayx/setup.json");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(&path, "[1, 2").expect("write");
        let machine = machine(dir.path()).with_text(&path, "[1, 2");

        let error =
            record_setup(&machine, Os::Linux, &["base".into()], None, false).expect_err("bad");
        assert!(error.to_string().contains("setup.json"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).expect("file"), "[1, 2");
        assert!(read_setup(&machine, Os::Linux).is_err());
    }

    #[test]
    fn the_record_lives_where_rayx_1_x_keeps_it() {
        let xdg = FakeMachine::new()
            .with_home("/home/dev")
            .with_env("XDG_CONFIG_HOME", "/home/dev/.xdg");
        assert_eq!(
            setup_path(&xdg, Os::Linux),
            Some(PathBuf::from("/home/dev/.xdg/rayx/setup.json"))
        );
        let home = FakeMachine::new().with_home("/Users/dev");
        for os in [Os::Linux, Os::MacOs] {
            assert_eq!(
                setup_path(&home, os),
                Some(PathBuf::from("/Users/dev/.config/rayx/setup.json")),
                "{os:?}"
            );
        }
        let windows = FakeMachine::new()
            .with_home(r"C:\Users\dev")
            .with_env("LOCALAPPDATA", r"C:\Users\dev\AppData\Local");
        assert_eq!(
            setup_path(&windows, Os::Windows),
            Some(
                PathBuf::from(r"C:\Users\dev\AppData\Local")
                    .join("rayx")
                    .join("setup.json")
            )
        );
    }

    #[test]
    fn a_dry_run_writes_nothing_and_a_write_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let machine = machine(dir.path());
        record_setup(
            &machine,
            Os::Linux,
            &["base".into()],
            Some("/work/app"),
            true,
        )
        .expect("dry run");
        assert!(
            !dir.path().join("rayx").exists(),
            "a dry run writes nothing"
        );

        for _ in 0..3 {
            record_setup(
                &machine,
                Os::Linux,
                &["base".into()],
                Some("/work/app"),
                false,
            )
            .expect("write");
        }
        let names: Vec<String> = std::fs::read_dir(dir.path().join("rayx"))
            .expect("rayx dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            ["setup.json"],
            "the write goes through a renamed temporary file"
        );
        let text = std::fs::read_to_string(dir.path().join("rayx/setup.json")).expect("file");
        assert!(text.ends_with("}\n"), "{text}");
        assert_eq!(recorded(dir.path()).sets, ["base"]);
    }
}
