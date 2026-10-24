//! `setup.json` and `rayx setup --update` on the host: the recorded sets re-run once per recorded
//! project root with that project's pins, a vanished root dropped, nothing recorded reported, and
//! no `apt-get upgrade` on a native Linux host.

use std::path::{Path, PathBuf};

use clap::Parser;
use rayx_cli::cli::{Cli, Command, SetupArgs};
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::records::{SetupFile, read_setup, record_setup};
use rayx_cli::setup::update::{HostUpdate, recorded_sets, update_host};
use rayx_cli::setup::{FakeMachine, PlanEnv, Set, after_run};

const UBUNTU: &str = "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";

fn linux() -> HostFacts {
    HostFacts {
        os: Os::Linux,
        arch: Arch::X64,
        emulated: false,
        wsl: false,
        distro: Some(Distro::parse(UBUNTU)),
    }
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .display()
        .to_string()
}

fn parse(args: &[&str]) -> Result<SetupArgs, clap::Error> {
    let mut argv = vec!["rayx", "setup"];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv).map(|cli| match cli.command {
        Command::Setup(args) => args,
        other => panic!("not setup: {other:?}"),
    })
}

fn env() -> PlanEnv {
    PlanEnv {
        host: linux(),
        pins: Pins::resolve(None),
        project_root: Some(PathBuf::from("/work/app")),
        tools_node: None,
        yes: false,
    }
}

fn machine_with(config: &Path, record: Option<&str>) -> FakeMachine {
    let mut machine = FakeMachine::new()
        .with_home("/home/dev")
        .with_env("XDG_CONFIG_HOME", &config.display().to_string())
        .with_program("rustup", "/home/dev/.cargo/bin/rustup")
        .with_program("cargo", "/home/dev/.cargo/bin/cargo")
        .with_dir(fixture("pinned-app"))
        .with_dir(fixture("plain-app"));
    if let Some(record) = record {
        machine = machine.with_text(config.join("rayx/setup.json"), record);
    }
    machine
}

fn user_path() -> UserPath {
    UserPath::Windows(Box::new(MemoryPathStore::new(None)))
}

fn update(args: &SetupArgs, machine: &FakeMachine, runner: &mut Runner) -> (HostUpdate, String) {
    let mut out = Vec::new();
    let result = update_host(
        args,
        &linux(),
        runner,
        machine,
        &mut user_path(),
        &mut out,
        None,
    );
    (result, String::from_utf8(out).expect("UTF-8"))
}

fn check_args() -> SetupArgs {
    SetupArgs {
        check: true,
        update: true,
        ..SetupArgs::default()
    }
}

#[test]
fn a_successful_run_records_its_sets_and_project_and_a_second_run_adds_to_them() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = machine_with(dir.path(), None);
    let web = SetupArgs {
        web: true,
        ..SetupArgs::default()
    };

    after_run(&web, 0, &env(), &[Set::Base, Set::Web], &machine);
    let mut other = env();
    other.project_root = Some(PathBuf::from("/work/other"));
    after_run(
        &SetupArgs::default(),
        0,
        &other,
        &[Set::Base, Set::Test],
        &machine,
    );
    after_run(
        &SetupArgs::default(),
        0,
        &env(),
        &[Set::Base, Set::Web],
        &machine,
    );

    let text = std::fs::read_to_string(dir.path().join("rayx/setup.json")).expect("record");
    let setup: SetupFile = serde_json::from_str(&text).expect("JSON");
    assert_eq!(setup.version, 1);
    assert_eq!(setup.sets, ["base", "web", "test"], "a union, each once");
    assert_eq!(setup.roots, ["/work/app", "/work/other"]);
}

#[test]
fn check_update_and_failed_runs_record_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = machine_with(dir.path(), None);

    after_run(
        &SetupArgs {
            check: true,
            ..SetupArgs::default()
        },
        0,
        &env(),
        &[Set::Base],
        &machine,
    );
    after_run(
        &SetupArgs {
            update: true,
            ..SetupArgs::default()
        },
        0,
        &env(),
        &[Set::Base],
        &machine,
    );
    after_run(&SetupArgs::default(), 1, &env(), &[Set::Base], &machine);

    assert!(!dir.path().join("rayx").exists());
}

#[test]
fn a_run_outside_a_project_records_the_sets_without_a_root() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut bare = env();
    bare.project_root = None;

    after_run(
        &SetupArgs::default(),
        0,
        &bare,
        &[Set::Base],
        &machine_with(dir.path(), None),
    );

    let setup: SetupFile = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("rayx/setup.json")).expect("record"),
    )
    .expect("JSON");
    assert_eq!(setup.sets, ["base"]);
    assert!(setup.roots.is_empty());
}

#[test]
fn an_unparsable_record_is_left_untouched() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("rayx/setup.json");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(&path, "[1, 2").expect("write");
    let machine = machine_with(dir.path(), Some("[1, 2"));

    let error = record_setup(&machine, Os::Linux, &["base".into()], None, false).expect_err("bad");

    assert!(
        error.to_string().contains("left untouched") || error.to_string().contains("not a valid")
    );
    assert_eq!(std::fs::read_to_string(&path).expect("file"), "[1, 2");
    assert!(read_setup(&machine, Os::Linux).is_err());
}

#[test]
fn update_runs_the_recorded_sets_once_per_root_with_each_roots_pins() {
    let dir = tempfile::tempdir().expect("temp dir");
    let record = format!(
        r#"{{"version":1,"sets":["base","web"],"roots":["{}","{}"]}}"#,
        fixture("pinned-app").replace('\\', "\\\\"),
        fixture("plain-app").replace('\\', "\\\\")
    );
    let machine = machine_with(dir.path(), Some(&record));
    let mut runner = Runner::record().with_os(Os::Linux);

    let (result, out) = update(&check_args(), &machine, &mut runner);

    assert!(result.recorded);
    let (pinned, plain) = out
        .split_once(&format!("==> setup --update: {}", fixture("plain-app")))
        .expect("two sections");
    assert!(
        pinned.contains(&format!("==> setup --update: {}", fixture("pinned-app"))),
        "{out}"
    );
    assert!(
        pinned.contains("wasm32-unknown-unknown target on 1.95.0")
            && pinned.contains("wasm-bindgen-cli 0.2.129"),
        "the pinned app's own toolchain and wasm-bindgen: {pinned}"
    );
    assert!(
        !pinned.contains("wasm32-unknown-unknown target on stable"),
        "the pinned root does not also get the default: {pinned}"
    );
    assert!(
        plain.contains("wasm32-unknown-unknown target on stable"),
        "the built-in default: {plain}"
    );
    assert!(
        !plain.contains("1.95.0") && !plain.contains("wasm-bindgen"),
        "{plain}"
    );
}

#[test]
fn without_a_recorded_root_it_runs_once_without_a_project() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = machine_with(
        dir.path(),
        Some(r#"{"version":1,"sets":["base","web"],"roots":[]}"#),
    );
    let mut runner = Runner::record().with_os(Os::Linux);

    let (result, out) = update(&check_args(), &machine, &mut runner);

    assert!(result.recorded);
    assert_eq!(out.matches("==> setup --update:").count(), 1, "{out}");
    assert!(out.contains("==> setup --update: no project"), "{out}");
    assert!(
        out.contains("wasm32-unknown-unknown target on stable"),
        "the defaults: {out}"
    );
}

#[test]
fn a_vanished_root_is_dropped_from_the_record_and_the_rest_still_runs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let record = format!(
        r#"{{"version":1,"sets":["base"],"roots":["/gone/app","{}"]}}"#,
        fixture("plain-app").replace('\\', "\\\\")
    );
    let machine = machine_with(dir.path(), Some(&record));
    let mut runner = Runner::record().with_os(Os::Linux);
    let args = SetupArgs {
        update: true,
        yes: true,
        ..SetupArgs::default()
    };

    let (result, out) = update(&args, &machine, &mut runner);

    assert!(result.recorded);
    assert_eq!(out.matches("==> setup --update:").count(), 1, "{out}");
    assert!(!out.contains("/gone/app"));
    let kept: SetupFile = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("rayx/setup.json")).expect("rewritten"),
    )
    .expect("JSON");
    assert_eq!(kept.roots, [fixture("plain-app")]);
}

#[test]
fn nothing_recorded_runs_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut runner = Runner::record().with_os(Os::Linux);

    let (result, out) = update(&check_args(), &machine_with(dir.path(), None), &mut runner);

    assert_eq!(
        result,
        HostUpdate {
            code: 0,
            recorded: false,
            unreadable: false
        }
    );
    assert!(out.is_empty());
    assert!(runner.specs().is_empty());
}

#[test]
fn a_native_linux_host_gets_no_apt_get_upgrade() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = machine_with(
        dir.path(),
        Some(r#"{"version":1,"sets":["base","web","test"],"roots":[]}"#),
    );
    let mut runner = Runner::record().with_os(Os::Linux).with_root(true);
    let args = SetupArgs {
        update: true,
        yes: true,
        ..SetupArgs::default()
    };

    update(&args, &machine, &mut runner);

    let lines = runner.lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("apt-get") && l.contains("install")),
        "missing packages are still installed: {lines:?}"
    );
    assert!(
        lines.iter().all(|l| !l.contains("upgrade")),
        "no upgrade on the host: {lines:?}"
    );
}

#[test]
fn only_the_recorded_sets_that_apply_to_the_host_run() {
    let setup = SetupFile {
        version: 1,
        sets: vec!["web".into(), "ios".into(), "bogus".into(), "web".into()],
        roots: Vec::new(),
    };
    assert_eq!(
        recorded_sets(&setup, &linux()),
        [Set::Base, Set::Web],
        "ios is macOS only"
    );
}

#[test]
fn the_command_line_takes_update_alone_with_yes_and_check() {
    assert!(parse(&["--update"]).expect("alone").update);
    assert!(parse(&["--update", "--yes", "--check"]).is_ok());
    for conflicting in [
        vec!["--update", "--web"],
        vec!["--update", "--all"],
        vec!["--update", "--wsl"],
        vec!["--update", "--lima"],
        vec!["--update", "--clone"],
        vec!["--update", "--vm-cpus", "4"],
        vec!["--update", "--vm-home-writable"],
    ] {
        assert!(parse(&conflicting).is_err(), "{conflicting:?}");
    }
}

/// `rayx setup --update` with nothing recorded says so and exits 0, whatever the host.
#[test]
fn the_command_says_when_nothing_is_recorded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rayx"))
        .args(["setup", "--update", "--check"])
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("LOCALAPPDATA", dir.path().join("local"))
        .current_dir(dir.path())
        .output()
        .expect("runs rayx");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("nothing recorded: run `rayx setup` first"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn an_unparsable_setup_record_is_an_error_not_nothing_recorded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = machine_with(dir.path(), Some("{ nope"));
    let mut runner = Runner::record().with_os(Os::Linux);

    let (result, out) = update(&check_args(), &machine, &mut runner);

    assert_eq!(
        result,
        HostUpdate {
            code: 1,
            recorded: false,
            unreadable: true
        }
    );
    assert!(out.is_empty());
    assert!(runner.specs().is_empty());
}

#[test]
fn the_command_reports_a_broken_record_instead_of_saying_nothing_is_recorded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config = dir.path().join("config/rayx");
    std::fs::create_dir_all(&config).expect("dir");
    std::fs::write(config.join("setup.json"), "{ nope").expect("write");
    std::fs::write(config.join("guests.json"), "{ nope").expect("write");
    let local = dir.path().join("local/rayx");
    std::fs::create_dir_all(&local).expect("dir");
    std::fs::write(local.join("setup.json"), "{ nope").expect("write");
    std::fs::write(local.join("guests.json"), "{ nope").expect("write");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rayx"))
        .args(["setup", "--update", "--check"])
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("LOCALAPPDATA", dir.path().join("local"))
        .current_dir(dir.path())
        .output()
        .expect("runs rayx");

    assert_eq!(output.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("nothing recorded"));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not a valid guest record")
            || String::from_utf8_lossy(&output.stderr).contains("not a valid")
    );
}
