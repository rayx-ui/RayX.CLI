//! Behaviour that spans commands, repaired after the epic's final review and its first runs on
//! Linux, Windows and a Lima guest (FIX-1 to FIX-7): prune and an unreadable record, where setup
//! records an app, the default-pin warning in doctor, the `PATH` nsh gets, the Chromium the README
//! describes, Playwright's ARM64 Linux layout, and the usage's binary name.

mod common;

use std::path::PathBuf;

use clap::Parser;
use common::Workspace;
use rayx_cli::cli::{AppArgs, Cli, Command, SetupArgs};
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, HostFacts, Os, Runner};
use rayx_cli::nsh::pin::default_pin;
use rayx_cli::nsh::store::{Kind, Receipt, Store};
use rayx_cli::nsh::{kept_ids, offer_prune};
use rayx_cli::project::{Pins, Project};
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, after_run, environment_for};

const ID: &str = "327bb9ea65d0";

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

fn receipt(id: &str) -> Receipt {
    Receipt {
        version: 1,
        id: id.into(),
        kind: Kind::Distribution,
        url: "u".into(),
        reference: None,
        commit: None,
        nsh: "0.2.0".into(),
        cli: "nsh/bin/nsh".into(),
        installed: "2026-10-24T01:30:00Z".into(),
    }
}

/// FIX-1: a record of setups that cannot be read stops prune with the reason.
#[test]
fn nothing_is_pruned_while_the_record_of_setups_cannot_be_read() {
    let home = Workspace::empty();
    home.write("nsh/aaaaaaaaaaaa/nsh/bin/nsh", "#!");
    let store = Store::at(home.path("nsh"));
    store.complete(&receipt("aaaaaaaaaaaa")).expect("receipt");
    let config = Workspace::empty();
    let machine = FakeMachine::new()
        .with_home("/home/dev")
        .with_env("RAYX_HOME", &home.root.display().to_string())
        .with_env("XDG_CONFIG_HOME", &config.root.display().to_string())
        .with_env("LOCALAPPDATA", &config.root.display().to_string())
        .with_text(config.path("rayx/setup.json"), "{ not json");
    let error = kept_ids(&mut Runner::print(), &machine, &host(Os::Linux))
        .expect_err("unreadable")
        .to_string();
    assert!(
        error.contains("setup.json") && error.contains("nothing is pruned"),
        "{error}"
    );
    let mut out = Vec::new();
    offer_prune(
        &mut Runner::print(),
        &machine,
        &host(Os::Linux),
        true,
        None,
        &mut out,
    );
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(text.contains("nothing is pruned"), "{text}");
    assert!(store.dir("aaaaaaaaaaaa").is_dir(), "the entry stays");
}

/// FIX-2: an app is recorded by the directory its pin is read in, also inside a Cargo workspace.
#[test]
fn a_rayx_toml_app_inside_a_cargo_workspace_is_recorded_by_its_own_directory() {
    let home = Workspace::empty();
    let work = Workspace::empty();
    // A Cargo workspace whose lock names no NSH crate, with a TSX app inside it.
    work.write(
        "Cargo.toml",
        "[package]\nname = \"ws\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    );
    work.write("src/lib.rs", "");
    work.write(
        "Cargo.lock",
        "version = 4\n\n[[package]]\nname = \"ws\"\nversion = \"0.1.0\"\n",
    );
    let fixture = Workspace::fixture("nsh-ts-app");
    for name in ["nsh.json", "rayx.toml"] {
        work.write(&format!("app/{name}"), &fixture.read(name));
    }
    let app = work.path("app");
    let project = Project::discover(&mut Runner::print(), &app, None).expect("the workspace");
    let env = environment_for(
        host(Os::Linux),
        Some(&project),
        &app,
        false,
        &mut Runner::print(),
    );
    assert!(env.project_root.is_some(), "inside a Cargo workspace");
    let config = home.path("config");
    let machine = FakeMachine::new()
        .with_home("/home/dev")
        .with_env("XDG_CONFIG_HOME", &config.display().to_string())
        .with_env("LOCALAPPDATA", &config.display().to_string());
    after_run(&SetupArgs::default(), 0, &env, &[Set::Base], &machine);
    let written = std::fs::read_to_string(config.join("rayx/setup.json")).expect("recorded");
    let setup: rayx_cli::setup::records::SetupFile =
        serde_json::from_str(&written).expect("setup.json");
    assert_eq!(
        setup.roots,
        [app.display().to_string()],
        "the directory its rayx.toml pin is read in, not the workspace root"
    );
}

/// FIX-3: doctor (like setup, which reads the pin the same way) warns in an app that pins no nsh.
#[test]
fn doctor_warns_in_an_app_that_pins_no_nsh() {
    let home = Workspace::empty();
    let app = Workspace::empty();
    app.write("nsh.json", "{\"name\":\"plain\"}");
    let assert = assert_cmd::cargo::cargo_bin_cmd!("rayx")
        .arg("doctor")
        .current_dir(&app.root)
        .env("RAYX_HOME", home.path("rayx-home"))
        .env("XDG_CONFIG_HOME", home.path("config"))
        .env("LOCALAPPDATA", home.path("config"))
        .env_remove("CARGO")
        .assert();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("pins no nsh, so rayx's default fork-0.2.0-4 (327bb9e) applies")
            && stderr.contains("[nsh]"),
        "{stderr}"
    );
}

fn app_args(argv: &[&str]) -> AppArgs {
    let mut full = vec!["rayx", "app"];
    full.extend_from_slice(argv);
    match Cli::try_parse_from(full).expect("parses").command {
        Command::App(args) => args,
        other => panic!("{other:?}"),
    }
}

/// The variables `rayx app build` sets for nsh on `machine`, with the default pin installed.
fn launch_set(machine: &FakeMachine, home: &Workspace) -> Vec<(String, String)> {
    home.write(&format!("rayx-home/nsh/{ID}/nsh/bin/nsh"), "#!");
    Store::at(home.path("rayx-home/nsh"))
        .complete(&receipt(ID))
        .expect("receipt");
    let env = env(Os::MacOs);
    let mut runner = Runner::record().with_os(Os::MacOs);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut cx = Cx::new(&env, &mut runner, machine, &mut path);
    let pin = default_pin(&home.root);
    rayx_cli::app::launch_for(
        &app_args(&["build"]),
        &home.root,
        Some(&pin),
        None,
        &mut cx,
        &mut Vec::new(),
    )
    .expect("launch")
    .set
}

/// FIX-4: nsh finds the cargo a prerequisite step just installed into `~/.cargo/bin`.
#[test]
fn nsh_finds_the_cargo_a_prerequisite_step_just_installed() {
    let home = Workspace::empty();
    let bin = PathBuf::from("/Users/dev").join(".cargo").join("bin");
    let machine = |path: &std::ffi::OsStr, with_bin: bool| {
        let machine = FakeMachine::new()
            .with_home("/Users/dev")
            .with_env("RAYX_HOME", &home.path("rayx-home").display().to_string())
            .with_env("PATH", &path.to_string_lossy());
        if with_bin {
            machine.with_dir(&bin)
        } else {
            machine
        }
    };
    let bare = std::env::join_paths(["/usr/bin", "/bin"]).expect("PATH");
    let set = launch_set(&machine(&bare, true), &home);
    let (_, value) = set
        .iter()
        .find(|(name, _)| name == "PATH")
        .expect("PATH is set for nsh");
    let dirs: Vec<PathBuf> = std::env::split_paths(value).collect();
    assert_eq!(
        dirs,
        [
            bin.clone(),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin")
        ],
        "~/.cargo/bin first, the rest kept"
    );
    // A PATH that has it already, or a machine without it, passes through.
    let with = std::env::join_paths([bin.clone(), PathBuf::from("/usr/bin")]).expect("PATH");
    assert!(
        !launch_set(&machine(&with, true), &home)
            .iter()
            .any(|(name, _)| name == "PATH")
    );
    assert!(
        !launch_set(&machine(&bare, false), &home)
            .iter()
            .any(|(name, _)| name == "PATH")
    );
    // An empty `PATH` stays empty: nsh keeps the system's default search path, and no empty
    // entry puts the current directory on it.
    assert!(
        !launch_set(&machine(std::ffi::OsStr::new(""), true), &home)
            .iter()
            .any(|(name, _)| name == "PATH")
    );
    // Empty entries of a set `PATH` are dropped, not kept.
    let gappy =
        std::ffi::OsString::from(format!("/usr/bin{}", if cfg!(windows) { ";" } else { ":" }));
    let set = launch_set(&machine(&gappy, true), &home);
    let (_, value) = set.iter().find(|(name, _)| name == "PATH").expect("PATH");
    assert_eq!(
        std::env::split_paths(value).collect::<Vec<_>>(),
        [bin.clone(), PathBuf::from("/usr/bin")]
    );
}

/// FIX-5: the README says which Chromium rayx passes and how an app without a Playwright package
/// gets one.
#[test]
fn the_readme_says_where_the_web_tests_chromium_comes_from() {
    let readme =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
            .expect("README");
    let text = readme.split_whitespace().collect::<Vec<_>>().join(" ");
    for needle in [
        "`--browser` with Playwright's Chromium (see below)",
        "the one `--test` installs for the project's Playwright package",
        "else the newest in Playwright's per-user cache",
        "An app without a Playwright package, such as one `rayx new` makes, gets none from `--test`",
        "`pnpm dlx playwright install chromium`",
        "puts `~/.cargo/bin` first on `PATH`",
    ] {
        assert!(text.contains(needle), "the README says {needle}");
    }
}

/// FIX-6: Playwright's ARM64 Linux Chromium (`chrome-linux-arm64`) is found.
#[test]
fn the_arm64_linux_chromium_is_found() {
    let cache = PathBuf::from("/home/dev")
        .join(".cache")
        .join("ms-playwright");
    let chrome = cache
        .join("chromium-1248")
        .join("chrome-linux-arm64")
        .join("chrome");
    let machine = FakeMachine::new()
        .with_home("/home/dev")
        .with_dir(cache.join("chromium-1248"))
        .with_file(&chrome);
    let env = env(Os::Linux);
    let mut runner = Runner::record().with_os(Os::Linux);
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let cx = Cx::new(&env, &mut runner, &machine, &mut path);
    assert_eq!(
        rayx_cli::setup::test_tools::playwright_chromium(&cx),
        Some(chrome)
    );
}

/// FIX-7: the usage names `rayx` on every host, never `rayx.exe`.
#[test]
fn the_usage_names_rayx_on_every_host() {
    assert_cmd::cargo::cargo_bin_cmd!("rayx")
        .arg("app")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("Usage: rayx app"));
}
