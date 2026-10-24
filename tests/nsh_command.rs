//! `rayx nsh status|path|install|prune` (NSHCMD-1, STORE-2): the pin and its store entry, the CLI's
//! path, installing, and pruning. The binary runs against a temporary `RAYX_HOME`; nothing is
//! downloaded (installs use a recorded runner).

mod common;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use common::Workspace;
use predicates::prelude::*;
use predicates::str::contains;
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::nsh::install::RustTools;
use rayx_cli::nsh::store::{Kind, Receipt, Store};
use rayx_cli::nsh::{install_in, path_in};

const ID: &str = "327bb9ea65d0";

/// `rayx` with its store and records in `home`.
fn rayx(home: &Workspace) -> Command {
    let mut command = cargo_bin_cmd!("rayx");
    command
        .env("RAYX_HOME", home.path("rayx-home"))
        .env("XDG_CONFIG_HOME", home.path("config"))
        .env("LOCALAPPDATA", home.path("config"));
    command
}

/// A complete store entry for `id` with a CLI file.
fn installed(home: &Workspace, id: &str, kind: Kind) -> PathBuf {
    let store = Store::at(home.path("rayx-home/nsh"));
    let cli = match kind {
        Kind::Distribution => "nsh/bin/nsh",
        Kind::Source => "source/target/release/nsh",
    };
    home.write(&format!("rayx-home/nsh/{id}/{cli}"), &"x".repeat(2048));
    store
        .complete(&Receipt {
            version: 1,
            id: id.into(),
            kind,
            url: "https://github.com/dXnSix/nsh/releases/download/fork-0.2.0-4/nsh-macos-arm64.tar.gz"
                .into(),
            reference: Some("fork-0.2.0-4".into()),
            commit: Some("327bb9ea65d08ffac4af498cc7fbbfa18a1b3623".into()),
            nsh: "0.2.0".into(),
            cli: cli.into(),
            installed: "2026-10-24T01:30:00Z".into(),
        })
        .expect("receipt");
    home.path(&format!("rayx-home/nsh/{id}/{cli}"))
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

#[test]
fn status_shows_the_pin_its_source_the_entry_its_size_and_the_cli() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    let cli = installed(&home, ID, Kind::Distribution);
    let output = rayx(&home)
        .args(["nsh", "status"])
        .current_dir(&app.root)
        .assert()
        .success()
        .get_output()
        .clone();
    let text = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(text.starts_with("nsh fork-0.2.0-4 (327bb9e)\n"), "{text}");
    for line in [
        "  source     project\n".to_string(),
        format!("  id         {ID}\n"),
        "  state      installed (distribution)\n".to_string(),
        format!("  cli        {}\n", cli.display()),
    ] {
        assert!(text.contains(&line), "{line:?} in\n{text}");
    }
    assert!(text.contains("  size       2."), "{text}");
}

#[test]
fn status_json_has_the_same_fields() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    installed(&home, ID, Kind::Source);
    let output = rayx(&home)
        .args(["nsh", "status", "--json", "-C"])
        .arg(&app.root)
        .assert()
        .success()
        .get_output()
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["value"], "fork-0.2.0-4 (327bb9e)");
    assert_eq!(json["source"], "project");
    assert_eq!(json["id"], ID);
    assert_eq!(json["kind"], "source");
    assert_eq!(json["state"], "installed");
    assert_eq!(json["installed"], true);
    assert!(json["bytes"].as_u64().is_some_and(|b| b >= 2048), "{json}");
    assert!(
        json["cli"]
            .as_str()
            .is_some_and(|c| Path::new(c).ends_with(Path::new("source/target/release/nsh"))),
        "{json}"
    );
    assert_eq!(json["commit"], "327bb9ea65d08ffac4af498cc7fbbfa18a1b3623");
    assert_eq!(json["version"], "0.2.0");
}

#[test]
fn status_of_a_missing_pin_and_of_each_source() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-ts-app");
    let output = rayx(&home)
        .args(["nsh", "status", "--json"])
        .current_dir(&app.root)
        .assert()
        .success()
        .get_output()
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["source"], "rayx.toml");
    assert_eq!(json["state"], "missing");
    assert_eq!(json["installed"], false);
    assert!(json["cli"].is_null() && json["bytes"].is_null(), "{json}");

    // Outside any app: the default, with no warning.
    let empty = Workspace::empty();
    rayx(&home)
        .args(["nsh", "status"])
        .current_dir(&empty.root)
        .assert()
        .success()
        .stdout(contains("  source     default\n").and(contains("  state      missing\n")))
        .stderr(contains("pins no nsh").not());

    // An app that pins nothing is warned on stderr, with the lines that pin it.
    std::fs::remove_file(app.path("rayx.toml")).expect("remove");
    rayx(&home)
        .args(["nsh", "status"])
        .current_dir(&app.root)
        .assert()
        .success()
        .stderr(contains("pins no nsh").and(contains("tag = \"fork-0.2.0-4\"")));
}

#[test]
fn path_prints_the_installed_cli() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    let cli = installed(&home, ID, Kind::Distribution);
    rayx(&home)
        .args(["nsh", "path"])
        .current_dir(&app.root)
        .assert()
        .success()
        .stdout(predicate::str::diff(format!("{}\n", cli.display())));
}

#[test]
fn path_without_install_on_a_missing_pin_exits_1_with_the_setup_command() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-fork-app");
    rayx(&home)
        .args(["nsh", "path", "--no-install"])
        .current_dir(&app.root)
        .assert()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(
            contains("nsh fork-0.2.0-4 (327bb9e) is not installed")
                .and(contains(format!("cd {} && rayx setup", app.root.display()))),
        );
    assert!(
        !home.path("rayx-home/nsh").join(ID).exists(),
        "nothing was installed"
    );
}

/// A recorded runner that serves upstream's v0.2.0 archive for macOS and answers `--version`.
fn upstream_runner() -> (Runner, Rc<RefCell<usize>>) {
    let archive = {
        let mut bytes = Vec::new();
        {
            let encoder = flate2::write::GzEncoder::new(&mut bytes, flate2::Compression::fast());
            let mut tar = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(2);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, "nsh/bin/nsh", &b"#!"[..])
                .expect("append");
            tar.into_inner().expect("tar").finish().expect("gzip");
        }
        bytes
    };
    let digest: String = {
        use sha2::Digest;
        sha2::Sha256::digest(&archive)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let downloads = Rc::new(RefCell::new(0));
    let counted = downloads.clone();
    let runner = Runner::record().responder(move |spec| {
        if spec.program == "curl" {
            *counted.borrow_mut() += 1;
            let url = spec.args.last().expect("url");
            let out = spec
                .args
                .iter()
                .position(|a| a == "-o")
                .map(|i| PathBuf::from(&spec.args[i + 1]))
                .expect("-o");
            let body = if url.ends_with(".sha256") {
                format!("{digest}  nsh-macos-arm64.tar.gz\n").into_bytes()
            } else {
                archive.clone()
            };
            std::fs::write(out, body).expect("write");
            return Some(Outcome::success().with_stdout("200"));
        }
        (spec.args == ["--version"]).then(|| Outcome::success().with_stdout("nsh 0.2.0\n"))
    });
    (runner, downloads)
}

#[test]
fn path_installs_a_missing_pin_first_then_prints_its_cli() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-ts-app");
    app.write("rayx.toml", "[nsh]\nversion = \"0.2.0\"\n");
    let store = Store::at(home.path("rayx-home/nsh"));
    let (mut runner, downloads) = upstream_runner();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = path_in(
        &mut runner,
        &store,
        &mac(),
        &RustTools::on_path(),
        &app.root,
        false,
        None,
        &mut out,
        &mut err,
    )
    .expect("path");
    assert_eq!(code, 0);
    let printed = String::from_utf8(out).expect("UTF-8");
    let cli = home.path("rayx-home/nsh/v0.2.0/nsh/bin/nsh");
    assert_eq!(
        printed,
        format!("{}\n", cli.display()),
        "only the path on stdout"
    );
    assert!(cli.is_file());
    assert_eq!(*downloads.borrow(), 2);
    assert!(
        String::from_utf8(err)
            .expect("UTF-8")
            .contains("==> nsh: downloading"),
        "progress goes to stderr"
    );
}

#[test]
fn install_installs_the_pin_and_again_changes_nothing() {
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-ts-app");
    app.write("rayx.toml", "[nsh]\nversion = \"0.2.0\"\n");
    let store = Store::at(home.path("rayx-home/nsh"));
    let (mut runner, downloads) = upstream_runner();
    let mut out = Vec::new();
    assert_eq!(
        install_in(
            &mut runner,
            &store,
            &mac(),
            &RustTools::on_path(),
            &app.root,
            &mut out,
        )
        .expect("install"),
        0
    );
    assert!(store.receipt("v0.2.0").is_some());
    assert_eq!(*downloads.borrow(), 2);

    let mut out = Vec::new();
    assert_eq!(
        install_in(
            &mut runner,
            &store,
            &mac(),
            &RustTools::on_path(),
            &app.root,
            &mut out,
        )
        .expect("again"),
        0
    );
    assert_eq!(*downloads.borrow(), 2, "nothing is downloaded again");
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .contains("nsh v0.2.0 is already installed"),
    );
}

#[test]
fn prune_with_yes_removes_what_neither_a_project_nor_the_default_uses() {
    let home = Workspace::empty();
    installed(&home, ID, Kind::Distribution);
    installed(&home, "5f0c2a91be47", Kind::Source);
    rayx(&home)
        .args(["nsh", "prune", "--yes"])
        .assert()
        .success()
        .stdout(contains("5f0c2a91be47").and(contains("Removed 5f0c2a91be47.")));
    assert!(
        home.path("rayx-home/nsh").join(ID).is_dir(),
        "the default pin is kept"
    );
    assert!(!home.path("rayx-home/nsh/5f0c2a91be47").exists());

    // Without a terminal and without --yes nothing is removed.
    installed(&home, "5f0c2a91be47", Kind::Source);
    rayx(&home)
        .args(["nsh", "prune"])
        .assert()
        .code(1)
        .stdout(contains("rayx nsh prune --yes"));
    assert!(home.path("rayx-home/nsh/5f0c2a91be47").is_dir());
}

#[test]
fn prune_reads_the_recorded_projects_without_writing_into_them() {
    let home = Workspace::empty();
    let project = Workspace::fixture("nsh-fork-app");
    std::fs::remove_file(project.path("Cargo.lock")).expect("no lock");
    home.write(
        "config/rayx/setup.json",
        &format!(
            "{{\"version\":1,\"sets\":[\"base\"],\"roots\":[{:?}]}}",
            project.root.display().to_string()
        ),
    );
    rayx(&home)
        .args(["nsh", "prune", "--yes"])
        .assert()
        .code(1)
        .stderr(contains("cannot tell which nsh").and(contains("nothing is pruned")));
    assert!(
        !project.path("Cargo.lock").exists(),
        "a prune writes no lock into a recorded project"
    );
}
