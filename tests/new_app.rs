//! `rayx new` (NEW-1, design §4.7): the default pin's `nsh init`, then the pin written into the
//! app: `.cargo/config.toml` patches and `cargo generate-lockfile` for `mixed` and `rust`,
//! `rayx.toml` for `ts`, and the next steps. `nsh init` and Cargo are recorded; the recorded
//! `nsh init` writes what nsh's templates write.

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use clap::Parser;
use common::Workspace;
use predicates::prelude::*;
use predicates::str::contains;
use rayx_cli::cli::{Cli, Command, NewArgs};
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::new::{cargo_config, create, rayx_toml};
use rayx_cli::nsh::pin::{Origin, resolve};
use rayx_cli::nsh::store::{Kind, Receipt, Store};
use rayx_cli::project::pins::DEFAULTS;
use rayx_cli::setup::FakeMachine;

const ID: &str = "327bb9ea65d0";
const FORK_SOURCE: &str =
    "git+https://github.com/dXnSix/nsh?tag=fork-0.2.0-4#327bb9ea65d08ffac4af498cc7fbbfa18a1b3623";

fn new_args(argv: &[&str]) -> NewArgs {
    let mut full = vec!["rayx", "new"];
    full.extend_from_slice(argv);
    match Cli::try_parse_from(full).expect("parses").command {
        Command::New(args) => args,
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

/// A store in `home` with the default pin installed.
fn with_default(home: &Workspace) -> PathBuf {
    home.write(&format!("rayx-home/nsh/{ID}/nsh/bin/nsh"), "#!");
    Store::at(home.path("rayx-home/nsh"))
        .complete(&Receipt {
            version: 1,
            id: ID.into(),
            kind: Kind::Distribution,
            url: "u".into(),
            reference: Some("fork-0.2.0-4".into()),
            commit: Some(DEFAULTS.nsh.commit.into()),
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

/// A recorded runner whose `nsh init` writes a template's files as nsh names them (the crate
/// `nsh-app-hello`), or fails when `refuse`; whose
/// `cargo generate-lockfile` writes a lock with the patched NSH source, and whose CLI answers its
/// version.
fn runner(refuse: bool) -> Runner {
    Runner::record().responder(move |spec| {
        if spec.args == ["--version"] {
            return Some(Outcome::success().with_stdout("nsh 0.2.0 (327bb9e)\n"));
        }
        if spec.args.first().is_some_and(|a| a == "init") {
            if refuse {
                return Some(Outcome {
                    code: Some(1),
                    stdout: String::new(),
                    stderr: "error: the directory is not empty".into(),
                });
            }
            let dir = PathBuf::from(&spec.args[1]);
            let template = &spec.args[spec.args.iter().position(|a| a == "--template").expect("template") + 1];
            std::fs::create_dir_all(&dir).expect("dir");
            std::fs::write(dir.join("nsh.json"), "{\"name\":\"hello\"}").expect("nsh.json");
            if template != "ts" {
                std::fs::write(
                    dir.join("Cargo.toml"),
                    "[package]\nname = \"nsh-app-hello\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nnsh-core = \"0.2.0\"\n\n[workspace]\n",
                )
                .expect("Cargo.toml");
                std::fs::create_dir_all(dir.join("src")).expect("src");
                std::fs::write(dir.join("src/lib.rs"), "").expect("lib");
            }
            return Some(Outcome::success());
        }
        if spec.program == "cargo" && spec.args == ["generate-lockfile"] {
            let dir = spec.cwd.clone().expect("cwd");
            std::fs::write(
                dir.join("Cargo.lock"),
                format!(
                    "version = 4\n\n[[package]]\nname = \"nsh-app-hello\"\nversion = \"0.1.0\"\ndependencies = [\"nsh-core\"]\n\n[[package]]\nname = \"nsh-core\"\nversion = \"0.2.0\"\nsource = \"{FORK_SOURCE}\"\n"
                ),
            )
            .expect("lock");
            return Some(Outcome::success());
        }
        None
    })
}

struct Created {
    result: anyhow::Result<()>,
    out: String,
    runner: Runner,
}

fn run(argv: &[&str], cwd: &Path, home: &Workspace, os: Os, refuse: bool) -> Created {
    let mut runner = runner(refuse);
    let mut out = Vec::new();
    let result = create(
        &new_args(argv),
        cwd,
        &host(os),
        &machine(home),
        &mut runner,
        &mut out,
    );
    Created {
        result,
        out: String::from_utf8(out).expect("UTF-8"),
        runner,
    }
}

#[test]
fn a_mixed_app_is_made_by_the_default_pins_nsh_init_and_pinned_through_its_lock() {
    let home = Workspace::empty();
    let cli = with_default(&home);
    let work = Workspace::empty();
    let created = run(&["hello"], &work.root, &home, Os::MacOs, false);
    created.result.expect("created");
    let app = work.path("hello");

    let init = &created.runner.specs()[1];
    assert_eq!(
        PathBuf::from(&init.program),
        cli,
        "the default pin's CLI, from the store"
    );
    assert_eq!(
        init.args,
        [
            "init",
            app.display().to_string().as_str(),
            "--template",
            "mixed"
        ]
    );
    assert!(init.unset.contains(&"CARGO".to_string()));

    let config = std::fs::read_to_string(app.join(".cargo/config.toml")).expect("config");
    assert_eq!(config, cargo_config(&DEFAULTS.nsh));
    let patched: Vec<&str> = config
        .lines()
        .filter(|l| {
            l.contains("= { git = \"https://github.com/dXnSix/nsh\", tag = \"fork-0.2.0-4\" }")
        })
        .collect();
    assert_eq!(patched.len(), DEFAULTS.nsh.crates.len());
    for name in DEFAULTS.nsh.crates {
        assert!(config.contains(&format!("\n{name} = {{ git")), "{name}");
    }
    let lock = created
        .runner
        .specs()
        .iter()
        .find(|s| s.program == "cargo")
        .expect("generate-lockfile");
    assert_eq!(lock.args, ["generate-lockfile"]);
    assert_eq!(lock.cwd.as_deref(), Some(app.as_path()));

    // The app's pin is now the default, from its own lock.
    let pin = resolve(&mut Runner::print(), &app).expect("pin");
    assert_eq!(pin.origin, Origin::Project);
    assert_eq!(pin.id().as_deref(), Some(ID));
    assert!(!app.join("rayx.toml").exists());
}

#[test]
fn a_rust_app_gets_the_same_patches_and_lock() {
    let home = Workspace::empty();
    with_default(&home);
    let work = Workspace::empty();
    let created = run(
        &["svc", "--template", "rust", "--name", "Service"],
        &work.root,
        &home,
        Os::Linux,
        false,
    );
    created.result.expect("created");
    let init = &created.runner.specs()[1];
    assert_eq!(init.args[2..], ["--template", "rust", "--name", "Service"]);
    assert!(work.path("svc/.cargo/config.toml").is_file());
    assert!(
        created
            .runner
            .specs()
            .iter()
            .any(|s| s.args == ["generate-lockfile"])
    );
}

#[test]
fn a_ts_app_gets_rayx_toml_with_the_default_table_and_no_cargo_files() {
    let home = Workspace::empty();
    with_default(&home);
    let work = Workspace::empty();
    let created = run(
        &["web", "--template", "ts"],
        &work.root,
        &home,
        Os::MacOs,
        false,
    );
    created.result.expect("created");
    let app = work.path("web");
    assert_eq!(
        std::fs::read_to_string(app.join("rayx.toml")).expect("rayx.toml"),
        rayx_toml(&DEFAULTS.nsh)
    );
    assert!(!app.join(".cargo").exists());
    assert!(!created.runner.specs().iter().any(|s| s.program == "cargo"));
    let pin = resolve(&mut Runner::print(), &app).expect("pin");
    assert_eq!(pin.origin, Origin::RayxToml);
    assert_eq!(pin.id().as_deref(), Some(ID));
}

#[test]
fn the_next_steps_are_printed_with_ios_only_on_a_mac() {
    let home = Workspace::empty();
    with_default(&home);
    let work = Workspace::empty();
    let mac = run(&["hello"], &work.root, &home, Os::MacOs, false);
    assert!(
        mac.out
            .contains("Created hello, pinned to nsh fork-0.2.0-4 (327bb9e). Next:"),
        "{}",
        mac.out
    );
    for line in [
        "  cd hello",
        "  rayx setup --web --ios",
        "  rayx app run --target web",
    ] {
        assert!(mac.out.contains(line), "{line:?} in {}", mac.out);
    }
    let other = Workspace::empty();
    let linux = run(&["hello"], &other.root, &home, Os::Linux, false);
    assert!(linux.out.contains("  rayx setup --web\n"), "{}", linux.out);
    assert!(!linux.out.contains("--ios"), "{}", linux.out);
}

#[test]
fn a_directory_nsh_refuses_is_left_unchanged() {
    let home = Workspace::empty();
    with_default(&home);
    let work = Workspace::empty();
    work.write("taken/README.md", "mine");
    let created = run(&["taken"], &work.root, &home, Os::MacOs, true);
    let error = format!("{:#}", created.result.expect_err("refused"));
    assert!(error.contains("nsh init"), "{error}");
    let names: Vec<String> = std::fs::read_dir(work.path("taken"))
        .expect("dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["README.md"], "nothing was written");
    assert!(!created.runner.specs().iter().any(|s| s.program == "cargo"));
}

#[test]
fn a_missing_default_pin_is_installed_first() {
    let home = Workspace::empty();
    let work = Workspace::empty();
    let created = run(&["hello"], &work.root, &home, Os::MacOs, false);
    // The recording serves no archive, so the install stops at the download; it came first.
    assert!(created.result.is_err());
    let first = created.runner.lines().first().cloned().unwrap_or_default();
    assert!(
        first.contains("releases/download/fork-0.2.0-4/nsh-macos-arm64.tar.gz.sha256"),
        "{first}"
    );
    assert!(
        !created
            .runner
            .specs()
            .iter()
            .any(|s| s.args.first().is_some_and(|a| a == "init"))
    );
}

#[test]
fn an_unknown_template_is_rejected_listing_the_three() {
    cargo_bin_cmd!("rayx")
        .args(["new", "app", "--template", "rayx"])
        .assert()
        .code(2)
        .stderr(
            contains("invalid value 'rayx'")
                .and(contains("mixed"))
                .and(contains("ts"))
                .and(contains("rust")),
        );
}

#[test]
fn the_files_rayx_writes_name_the_default_pin() {
    let config = cargo_config(&DEFAULTS.nsh);
    assert!(
        config.starts_with("# The NSH this app is built with"),
        "{config}"
    );
    assert!(config.contains("[patch.crates-io]\n"), "{config}");
    assert!(config.contains("fork-0.2.0-4 at 327bb9e"), "{config}");
    let table = rayx_toml(&DEFAULTS.nsh);
    for line in [
        "[nsh]",
        "git = \"https://github.com/dXnSix/nsh\"",
        "tag = \"fork-0.2.0-4\"",
        "commit = \"327bb9ea65d08ffac4af498cc7fbbfa18a1b3623\"",
    ] {
        assert!(table.contains(line), "{table}");
    }
}

#[test]
fn a_pin_file_already_there_is_never_replaced() {
    let home = Workspace::empty();
    with_default(&home);
    for (template, file) in [
        ("mixed", ".cargo/config.toml"),
        ("rust", ".cargo/config.toml"),
        ("ts", "rayx.toml"),
    ] {
        let work = Workspace::empty();
        work.write(&format!("app/{file}"), "mine");
        let created = run(
            &["app", "--template", template],
            &work.root,
            &home,
            Os::MacOs,
            false,
        );
        let error = format!("{:#}", created.result.expect_err("refused"));
        // The path as this host writes it (`\\` on Windows).
        let shown = file
            .split('/')
            .fold(PathBuf::new(), |path, part| path.join(part));
        assert!(
            error.contains(&format!("{} exists", shown.display()))
                && error.contains("move it away first"),
            "{template}: {error}"
        );
        assert_eq!(work.read(&format!("app/{file}")), "mine", "{template}");
        assert!(
            created.runner.specs().is_empty(),
            "{template}: nothing ran, not even nsh init"
        );
    }
}
