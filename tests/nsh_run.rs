//! `rayx app` runs the pin's nsh (RUN-1, design §4.4): every argument after the command unchanged,
//! in the directory the pin was read in, without `CARGO`, with the wasm32 C tools and the pinned
//! NDK set only when unset, and on Unix by replacing itself with nsh.

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::Parser;
use common::Workspace;
use rayx_cli::app::{Launch, launch_for, prepare};
#[cfg(unix)]
use rayx_cli::app::{Launcher, ProcessLauncher};
use rayx_cli::cli::{AppArgs, Cli, Command};
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::nsh::pin::{Pin, default_pin, resolve};
use rayx_cli::nsh::store::{Kind, Receipt, Store};
use rayx_cli::project::Pins;
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set};

const ID: &str = "327bb9ea65d0";

fn app_args(argv: &[&str]) -> AppArgs {
    let mut full = vec!["rayx", "app"];
    full.extend_from_slice(argv);
    match Cli::try_parse_from(full).expect("parses").command {
        Command::App(args) => args,
        other => panic!("{other:?}"),
    }
}

fn host(os: Os) -> HostFacts {
    HostFacts {
        os,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn env(os: Os) -> PlanEnv {
    PlanEnv {
        host: host(os),
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
        nsh: None,
    }
}

/// A store in `home` with the default pin installed; its CLI.
fn installed(home: &Workspace) -> PathBuf {
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

fn machine(home: &Workspace) -> FakeMachine {
    FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("RAYX_HOME", &home.path("rayx-home").display().to_string())
}

/// `launch_for` with `pin`, on `machine`.
fn launch(args: &AppArgs, dir: &Path, pin: &Pin, machine: &FakeMachine, os: Os) -> Launch {
    let env = env(os);
    let mut runner = Runner::record().with_os(os);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = Cx::new(&env, &mut runner, machine, &mut path);
    launch_for(args, dir, Some(pin), None, &mut cx, &mut Vec::new()).expect("launch")
}

fn os(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn every_argument_after_the_command_reaches_nsh_unchanged() {
    let home = Workspace::empty();
    installed(&home);
    let app = Workspace::fixture("nsh-fork-app");
    let pin = resolve(&mut Runner::print(), &app.root).expect("pin");
    let args = app_args(&[
        "run",
        ".",
        "--target",
        "web",
        "--strict-port",
        "--port",
        "4000",
        "-C",
        "x",
        "--no-install",
        "--",
        "--app-flag",
        "--target",
        "ios",
    ]);
    let launch = launch(&args, &app.root, &pin, &machine(&home), Os::MacOs);
    assert_eq!(
        launch.args,
        os(&[
            "run",
            ".",
            "--target",
            "web",
            "--strict-port",
            "--port",
            "4000",
            "-C",
            "x",
            "--no-install",
            "--",
            "--app-flag",
            "--target",
            "ios"
        ])
    );
}

#[test]
fn the_cli_is_the_pins_store_entry_never_the_nsh_on_path() {
    let home = Workspace::empty();
    let cli = installed(&home);
    let app = Workspace::fixture("nsh-fork-app");
    let pin = resolve(&mut Runner::print(), &app.root).expect("pin");
    let on_path = machine(&home).with_program("nsh", "/opt/homebrew/bin/nsh");
    let launch = launch(&app_args(&["build"]), &app.root, &pin, &on_path, Os::MacOs);
    assert_eq!(launch.program, cli);
}

#[test]
fn nsh_runs_in_the_directory_the_pin_was_read_in_and_without_cargo() {
    let home = Workspace::empty();
    installed(&home);
    let app = Workspace::fixture("nsh-fork-app");
    let pin = resolve(&mut Runner::print(), &app.root).expect("pin");
    let launch = launch(
        &app_args(&["check", "."]),
        &app.root,
        &pin,
        &machine(&home),
        Os::MacOs,
    );
    assert_eq!(launch.cwd, app.root);
    assert_eq!(launch.unset, ["CARGO"]);
    assert!(
        launch.set.is_empty(),
        "nothing else is set here: {:?}",
        launch.set
    );
}

#[test]
fn the_wasm32_c_tools_are_the_web_sets_llvm_and_only_when_unset() {
    let home = Workspace::empty();
    installed(&home);
    let dir = home.root.clone();
    let pin = default_pin(&dir);
    let llvm = PathBuf::from("/opt/homebrew/opt/llvm/bin");
    let mac = machine(&home)
        .with_file("/opt/homebrew/bin/brew")
        .with_file(llvm.join("clang"))
        .with_file(llvm.join("llvm-ar"));
    let launch_mac = launch(
        &app_args(&["run", "--target", "web"]),
        &dir,
        &pin,
        &mac,
        Os::MacOs,
    );
    // Compared as paths, so the separators of this host do not matter.
    let as_paths = |set: &[(String, String)]| -> Vec<(String, PathBuf)> {
        set.iter()
            .map(|(name, value)| (name.clone(), PathBuf::from(value)))
            .collect()
    };
    assert_eq!(
        as_paths(&launch_mac.set),
        [
            ("CC_wasm32_unknown_unknown".to_string(), llvm.join("clang")),
            (
                "AR_wasm32_unknown_unknown".to_string(),
                llvm.join("llvm-ar")
            ),
        ]
    );
    // A value the user set is theirs.
    let own = mac
        .clone()
        .with_env("CC_wasm32_unknown_unknown", "/usr/local/bin/clang-19");
    let launch_own = launch(&app_args(&["run"]), &dir, &pin, &own, Os::MacOs);
    let names: Vec<&str> = launch_own.set.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, ["AR_wasm32_unknown_unknown"]);

    // Linux: the distribution's LLVM, from the base set.
    let linux = machine(&home)
        .with_file("/usr/bin/clang")
        .with_file("/usr/bin/llvm-ar");
    let launch_linux = launch(&app_args(&["run"]), &dir, &pin, &linux, Os::Linux);
    assert_eq!(launch_linux.set[0].1, "/usr/bin/clang");
    // Without that LLVM nothing is set; nsh says what is missing.
    let bare = launch(&app_args(&["run"]), &dir, &pin, &machine(&home), Os::Linux);
    assert!(bare.set.is_empty());
}

#[test]
fn android_ndk_home_is_the_pinned_ndk_only_when_it_is_unset_and_installed() {
    let home = Workspace::empty();
    installed(&home);
    let dir = home.root.clone();
    let pin = default_pin(&dir);
    let sdk = PathBuf::from("/Users/dev/Library/Android/sdk");
    let ndk = sdk.join("ndk").join("28.0.12674087");
    let with_ndk = machine(&home)
        .with_env("ANDROID_HOME", &sdk.display().to_string())
        .with_dir(&sdk)
        .with_dir(sdk.join("ndk").join("29.0.1"))
        .with_dir(&ndk);
    let launch_android = launch(
        &app_args(&["run", "--target", "android"]),
        &dir,
        &pin,
        &with_ndk,
        Os::MacOs,
    );
    assert!(
        launch_android
            .set
            .contains(&("ANDROID_NDK_HOME".to_string(), ndk.display().to_string())),
        "the pin, not the newer NDK beside it: {:?}",
        launch_android.set
    );
    let chosen = with_ndk.clone().with_env("ANDROID_NDK_HOME", "/opt/ndk");
    let launch_chosen = launch(&app_args(&["run"]), &dir, &pin, &chosen, Os::MacOs);
    assert!(
        !launch_chosen
            .set
            .iter()
            .any(|(k, _)| k == "ANDROID_NDK_HOME")
    );
    let without = machine(&home)
        .with_env("ANDROID_HOME", &sdk.display().to_string())
        .with_dir(&sdk)
        .with_dir(sdk.join("ndk").join("29.0.1"));
    let launch_without = launch(&app_args(&["run"]), &dir, &pin, &without, Os::MacOs);
    assert!(
        !launch_without
            .set
            .iter()
            .any(|(k, _)| k == "ANDROID_NDK_HOME")
    );
}

#[test]
fn a_pin_that_is_not_installed_names_the_setup_command() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    let pin = resolve(&mut Runner::print(), &app.root).expect("pin");
    let env = env(Os::MacOs);
    let mut runner = Runner::record();
    let machine = machine(&home);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    let error = launch_for(
        &app_args(&["build"]),
        &app.root,
        Some(&pin),
        None,
        &mut cx,
        &mut Vec::new(),
    )
    .expect_err("not installed")
    .to_string();
    assert!(
        error.contains("nsh fork-0.2.0-4 (327bb9e) is not installed"),
        "{error}"
    );
    assert!(error.contains("rayx setup"), "{error}");
}

#[test]
fn prepare_reads_the_pin_in_c_runs_the_prerequisites_of_the_target_then_launches() {
    let home = Workspace::empty();
    let cli = installed(&home);
    let app = Workspace::fixture("nsh-fork-app");
    let elsewhere = Workspace::empty();
    let args = app_args(&[
        "-C",
        app.root.to_str().expect("UTF-8"),
        "test",
        "--target",
        "web",
    ]);
    let mut seen: Vec<(Option<String>, Vec<Set>, PathBuf)> = Vec::new();
    let mut ensure = |env: &PlanEnv,
                      sets: &[Set],
                      dir: &Path,
                      _: &mut Runner,
                      _: &dyn rayx_cli::setup::Machine,
                      _: &mut UserPath| {
        seen.push((
            env.nsh
                .as_ref()
                .and_then(|pin| pin.as_ref().ok())
                .and_then(Pin::id),
            sets.to_vec(),
            dir.to_path_buf(),
        ));
        Ok(0)
    };
    // The pin is read through the command's runner; this one answers Cargo's queries for real.
    let mut runner = common::reading_cargo();
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut err = Vec::new();
    let launch = prepare(
        &args,
        &elsewhere.root,
        &host(Os::MacOs),
        &mut runner,
        &machine(&home).with_file("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        &mut path,
        &mut ensure,
        &mut err,
    )
    .expect("prepared");
    assert_eq!(
        seen,
        [(
            Some(ID.to_string()),
            vec![Set::Web, Set::Test],
            app.root.clone()
        )]
    );
    assert_eq!(launch.program, cli);
    assert_eq!(launch.cwd, app.root);
    assert_eq!(launch.args, os(&["test", "--target", "web"]));
    assert!(err.is_empty(), "a pinned app gets no warning");
}

#[test]
fn a_failed_prerequisite_starts_nothing() {
    let home = Workspace::empty();
    installed(&home);
    let app = Workspace::fixture("nsh-fork-app");
    let mut ensure = |_: &PlanEnv,
                      _: &[Set],
                      _: &Path,
                      _: &mut Runner,
                      _: &dyn rayx_cli::setup::Machine,
                      _: &mut UserPath| {
        Err(anyhow::anyhow!(
            "missing: the wasm32-unknown-unknown target. Run `rayx setup --web`"
        ))
    };
    let error = prepare(
        &app_args(&["run", "--target", "web"]),
        &app.root,
        &host(Os::MacOs),
        &mut Runner::record(),
        &machine(&home),
        &mut UserPath::Windows(Box::new(MemoryPathStore::new(None))),
        &mut ensure,
        &mut Vec::new(),
    )
    .expect_err("refused")
    .to_string();
    assert!(error.contains("rayx setup --web"), "{error}");
}

/// Runs as a child of the test binary: replaces itself with `/bin/sh`, which prints a marker with
/// its process id, then its directory, its arguments and whether `CARGO` reached it, and
/// exits 7. The test harness prints its own words before the marker.
#[cfg(unix)]
#[test]
fn exec_helper() {
    let Some(dir) = std::env::var_os("RAYX_EXEC_HELPER") else {
        return;
    };
    let launch = Launch {
        program: PathBuf::from("/bin/sh"),
        args: os(&[
            "-c",
            "echo \"nsh-pid=$$\"; pwd; echo \"$@\"; echo \"cargo=${CARGO-unset} set=$SET_BY_RAYX\"; exit 7",
            "sh",
            "run",
            "--",
            "--app",
        ]),
        cwd: PathBuf::from(dir),
        set: vec![("SET_BY_RAYX".to_string(), "yes".to_string())],
        unset: vec!["CARGO".to_string()],
    };
    let result = ProcessLauncher.launch(&launch);
    panic!("exec returned: {result:?}");
}

#[cfg(unix)]
#[test]
fn on_unix_rayx_becomes_nsh_and_its_exit_code_is_nshs() {
    let dir = Workspace::empty();
    let child = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "exec_helper", "--nocapture", "--test-threads=1"])
        .env("RAYX_EXEC_HELPER", &dir.root)
        .env("CARGO", "/should/not/reach/nsh")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    let output = child.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(7), "nsh's exit code");
    let text = String::from_utf8(output.stdout).expect("UTF-8");
    // The harness's `test exec_helper ... ` has no line end, so the marker can follow it.
    let start = text.find("nsh-pid=").unwrap_or(text.len());
    let mut lines = text[start..].lines();
    assert_eq!(
        lines.next(),
        Some(format!("nsh-pid={pid}").as_str()),
        "the same process: exec, not a child\n{text}"
    );
    assert_eq!(
        lines.next().map(|l| std::fs::canonicalize(l).expect("dir")),
        Some(dir.root.clone())
    );
    assert_eq!(lines.next(), Some("run -- --app"));
    assert_eq!(lines.next(), Some("cargo=unset set=yes"));
}

#[test]
fn the_recorded_version_answer_is_not_needed_to_launch() {
    // launch_for reads the receipt; it runs nothing on the way.
    let home = Workspace::empty();
    installed(&home);
    let dir = home.root.clone();
    let env = env(Os::MacOs);
    let mut runner = Runner::record().respond(|_| true, Outcome::failure(1));
    let machine = machine(&home);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    launch_for(
        &app_args(&["build"]),
        &dir,
        Some(&default_pin(&dir)),
        None,
        &mut cx,
        &mut Vec::new(),
    )
    .expect("launch");
    assert!(runner.lines().is_empty(), "{:?}", runner.lines());
}
