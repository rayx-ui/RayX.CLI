//! The nsh store (STORE-1, STORE-2, design §4.2): entries under `$RAYX_HOME/nsh/<id>/`, a receipt
//! written last, unfinished entries, the install lock, and prune.

mod common;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, UNIX_EPOCH};

use common::Workspace;
use rayx_cli::host::{Os, Runner};
use rayx_cli::nsh::kept_ids;
use rayx_cli::nsh::store::{
    Kind, RECEIPT, Receipt, Store, prune, prune_candidates, rfc3339, size_of,
};
use rayx_cli::setup::FakeMachine;

fn receipt(id: &str, kind: Kind) -> Receipt {
    Receipt {
        version: 1,
        id: id.to_string(),
        kind,
        url: "https://github.com/dXnSix/nsh/releases/download/fork-0.2.0-4/nsh-macos-arm64.tar.gz"
            .into(),
        reference: Some("fork-0.2.0-4".into()),
        commit: Some("327bb9ea65d08ffac4af498cc7fbbfa18a1b3623".into()),
        nsh: "0.2.0".into(),
        cli: match kind {
            Kind::Distribution => "nsh/bin/nsh".into(),
            Kind::Source => "source/target/release/nsh".into(),
        },
        installed: "2026-10-24T01:30:00Z".into(),
    }
}

/// A store in a temporary directory with entries of `bytes` bytes each.
fn store_with(entries: &[(&str, Option<Kind>, usize)]) -> (Workspace, Store) {
    let home = Workspace::empty();
    let store = Store::at(home.path("nsh"));
    for (id, kind, bytes) in entries {
        home.write(&format!("nsh/{id}/nsh/bin/nsh"), &"x".repeat(*bytes));
        if let Some(kind) = kind {
            store.complete(&receipt(id, *kind)).expect("receipt");
        }
    }
    (home, store)
}

#[test]
fn the_store_is_rayx_home_nsh_else_home_rayx_nsh() {
    let with_home = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("RAYX_HOME", "/opt/rayx");
    assert_eq!(
        Store::locate(&with_home).expect("store").root,
        PathBuf::from("/opt/rayx").join("nsh")
    );
    let plain = FakeMachine::new().with_home("/Users/dev");
    assert_eq!(
        Store::locate(&plain).expect("store").root,
        PathBuf::from("/Users/dev").join(".rayx").join("nsh")
    );
    let empty_var = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("RAYX_HOME", "");
    assert_eq!(
        Store::locate(&empty_var).expect("store").root,
        PathBuf::from("/Users/dev").join(".rayx").join("nsh")
    );
    assert!(
        Store::locate(&FakeMachine::new()).is_none(),
        "no home, no store"
    );
    assert_eq!(
        Store::at("/s").dir("327bb9ea65d0"),
        PathBuf::from("/s").join("327bb9ea65d0")
    );
}

#[test]
fn a_receipt_written_last_makes_an_entry_complete() {
    let (home, store) = store_with(&[("327bb9ea65d0", None, 10)]);
    assert_eq!(
        store.receipt("327bb9ea65d0"),
        None,
        "unfinished without a receipt"
    );

    let written = receipt("327bb9ea65d0", Kind::Distribution);
    store.complete(&written).expect("complete");
    assert_eq!(store.receipt("327bb9ea65d0"), Some(written.clone()));
    let names: Vec<String> = std::fs::read_dir(home.path("nsh/327bb9ea65d0"))
        .expect("entry")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&RECEIPT.to_string()), "{names:?}");
    assert!(
        !names.iter().any(|n| n.ends_with(".tmp")),
        "no temporary file is left: {names:?}"
    );

    let json: serde_json::Value =
        serde_json::from_str(&home.read("nsh/327bb9ea65d0/receipt.json")).expect("JSON");
    for field in [
        "version",
        "id",
        "kind",
        "url",
        "ref",
        "commit",
        "nsh",
        "cli",
        "installed",
    ] {
        assert!(
            json.get(field).is_some(),
            "receipt.json has `{field}`: {json}"
        );
    }
    assert_eq!(json["version"], 1);
    assert_eq!(json["kind"], "distribution");
    assert_eq!(json["ref"], "fork-0.2.0-4");
    assert_eq!(
        store.cli(&written),
        home.path("nsh/327bb9ea65d0")
            .join("nsh")
            .join("bin")
            .join("nsh")
    );
}

#[test]
fn a_receipt_of_another_version_or_id_or_a_broken_one_leaves_the_entry_unfinished() {
    let (home, store) = store_with(&[("327bb9ea65d0", None, 1)]);
    let mut other = receipt("327bb9ea65d0", Kind::Source);
    other.version = 2;
    home.write(
        "nsh/327bb9ea65d0/receipt.json",
        &serde_json::to_string(&other).expect("JSON"),
    );
    assert_eq!(store.receipt("327bb9ea65d0"), None);
    home.write(
        "nsh/327bb9ea65d0/receipt.json",
        &serde_json::to_string(&receipt("5f0c2a91be47", Kind::Source)).expect("JSON"),
    );
    assert_eq!(
        store.receipt("327bb9ea65d0"),
        None,
        "another entry's receipt"
    );
    home.write("nsh/327bb9ea65d0/receipt.json", "{\"version\": 1");
    assert_eq!(store.receipt("327bb9ea65d0"), None);
}

#[test]
fn entries_list_complete_and_unfinished_ones_with_their_sizes() {
    let (home, store) = store_with(&[
        ("327bb9ea65d0", Some(Kind::Distribution), 1000),
        ("5f0c2a91be47", None, 300),
        ("v0.2.0", Some(Kind::Distribution), 50),
    ]);
    // A lock file and a stray file are not entries.
    home.write("nsh/327bb9ea65d0.lock", "");
    home.write("nsh/notes.txt", "x");
    let entries = store.entries();
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["327bb9ea65d0", "5f0c2a91be47", "v0.2.0"]);
    assert!(entries[0].receipt.is_some() && entries[1].receipt.is_none());
    assert!(entries[0].bytes >= 1000, "{}", entries[0].bytes);
    assert_eq!(entries[1].bytes, 300);
    assert_eq!(size_of(&home.path("nsh/5f0c2a91be47")), 300);
}

#[test]
fn a_second_install_waits_for_the_lock_says_so_and_then_proceeds() {
    let (_home, store) = store_with(&[]);
    let first = store
        .lock("327bb9ea65d0", &mut Vec::new())
        .expect("first lock");
    let (sender, receiver) = mpsc::channel();
    let waiting = store.clone();
    let second = std::thread::spawn(move || {
        let mut out = Vec::new();
        let lock = waiting.lock("327bb9ea65d0", &mut out).expect("second lock");
        sender.send(()).expect("send");
        drop(lock);
        String::from_utf8(out).expect("UTF-8")
    });
    assert!(
        receiver.recv_timeout(Duration::from_millis(300)).is_err(),
        "the second install waits while the first holds the lock"
    );
    drop(first);
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the second install proceeds once the lock is free");
    let said = second.join().expect("thread");
    assert!(
        said.contains("another rayx is installing nsh 327bb9ea65d0; waiting for it to finish"),
        "{said}"
    );

    // Another id does not wait.
    let _a = store.lock("327bb9ea65d0", &mut Vec::new()).expect("lock");
    let mut out = Vec::new();
    let _b = store.lock("v0.2.0", &mut out).expect("other lock");
    assert!(out.is_empty());
}

#[test]
fn prune_lists_what_no_project_and_no_default_uses_and_removes_it_after_a_yes() {
    let (home, store) = store_with(&[
        ("327bb9ea65d0", Some(Kind::Distribution), 2048),
        ("5f0c2a91be47", Some(Kind::Source), 4096),
        ("aaaaaaaaaaaa", None, 10),
    ]);
    let keep = vec!["327bb9ea65d0".to_string()];
    let ids: Vec<String> = prune_candidates(&store, &keep)
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids, ["5f0c2a91be47", "aaaaaaaaaaaa"]);

    let mut asked = Vec::new();
    let mut yes = |question: &str| {
        asked.push(question.to_string());
        true
    };
    let mut out = Vec::new();
    let code = prune(&store, &keep, false, Some(&mut yes), &mut out);
    let text = String::from_utf8(out).expect("UTF-8");
    assert_eq!(code, 0, "{text}");
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("5f0c2a91be47"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        line.contains(" KiB  source 0.2.0"),
        "the size and what it is: {line}"
    );
    assert!(text.contains("aaaaaaaaaaaa  10 B  unfinished"), "{text}");
    assert!(
        !text.contains("327bb9ea65d0  "),
        "the kept entry is not listed: {text}"
    );
    assert_eq!(asked.len(), 1);
    assert!(asked[0].starts_with("Remove these ("), "{asked:?}");
    assert!(home.path("nsh/327bb9ea65d0").is_dir());
    assert!(!home.path("nsh/5f0c2a91be47").exists());
    assert!(!home.path("nsh/aaaaaaaaaaaa").exists());
}

#[test]
fn a_no_keeps_everything() {
    let (home, store) = store_with(&[("5f0c2a91be47", Some(Kind::Source), 10)]);
    let mut no = |_: &str| false;
    let mut out = Vec::new();
    assert_eq!(prune(&store, &[], false, Some(&mut no), &mut out), 0);
    assert!(home.path("nsh/5f0c2a91be47").is_dir());
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .contains("Nothing removed.")
    );
}

#[test]
fn yes_removes_without_asking() {
    let (home, store) = store_with(&[("5f0c2a91be47", Some(Kind::Source), 10)]);
    let mut never = |question: &str| -> bool { panic!("--yes asks nothing: {question}") };
    assert_eq!(
        prune(&store, &[], true, Some(&mut never), &mut Vec::new()),
        0
    );
    assert!(!home.path("nsh/5f0c2a91be47").exists());
}

#[test]
fn without_a_terminal_and_without_yes_nothing_is_removed_and_the_code_is_1() {
    let (home, store) = store_with(&[("5f0c2a91be47", Some(Kind::Source), 10)]);
    let mut out = Vec::new();
    assert_eq!(prune(&store, &[], false, None, &mut out), 1);
    assert!(home.path("nsh/5f0c2a91be47").is_dir());
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(text.contains("rayx nsh prune --yes"), "{text}");

    let (_empty_home, empty) = store_with(&[]);
    let mut out = Vec::new();
    assert_eq!(
        prune(&empty, &[], false, None, &mut out),
        0,
        "nothing to prune is no failure"
    );
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .contains("Nothing to prune")
    );
}

#[test]
fn prune_keeps_the_default_pin_and_every_recorded_projects_pin() {
    let config = Workspace::empty();
    // A recorded project pinned to a branch, and a vanished one.
    let project = Workspace::fixture("nsh-fork-app");
    let lock = project.read("Cargo.lock").replace(
        "?tag=fork-0.2.0-4#327bb9ea65d08ffac4af498cc7fbbfa18a1b3623",
        "?branch=main#5f0c2a91be4700000000000000000000000000aa",
    );
    project.write("Cargo.lock", &lock);
    config.write(
        "rayx/setup.json",
        &format!(
            "{{\"version\":1,\"sets\":[\"base\"],\"roots\":[{:?},\"/nowhere/gone\"]}}",
            project.root.display().to_string()
        ),
    );
    let machine = FakeMachine::new()
        .with_home("/home/dev")
        .with_env("XDG_CONFIG_HOME", &config.root.display().to_string())
        .with_text(
            config.path("rayx/setup.json"),
            &config.read("rayx/setup.json"),
        );
    let host = rayx_cli::host::HostFacts {
        os: Os::Linux,
        arch: rayx_cli::host::Arch::X64,
        emulated: false,
        wsl: false,
        distro: None,
    };
    let keep = kept_ids(&mut Runner::print(), &machine, &host).expect("every pin reads");
    assert_eq!(
        keep,
        ["327bb9ea65d0", "5f0c2a91be47"],
        "the default first, then the project's"
    );
}

#[test]
fn receipt_times_are_utc_rfc_3339() {
    assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    assert_eq!(
        rfc3339(UNIX_EPOCH + Duration::from_secs(1_792_805_400)),
        "2026-10-24T01:30:00Z"
    );
    assert_eq!(
        rfc3339(UNIX_EPOCH + Duration::from_secs(951_782_400)),
        "2000-02-29T00:00:00Z",
        "a leap day"
    );
}

#[test]
fn prune_keeps_an_entry_another_rayx_is_installing_and_every_lock_file() {
    let (home, store) = store_with(&[
        ("aaaaaaaaaaaa", Some(Kind::Distribution), 10),
        ("bbbbbbbbbbbb", None, 10),
    ]);
    let held = store
        .lock("bbbbbbbbbbbb", &mut Vec::new())
        .expect("an install holds it");
    let mut out = Vec::new();
    assert_eq!(prune(&store, &[], true, None, &mut out), 0);
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(
        text.contains("rayx: another rayx is installing nsh bbbbbbbbbbbb; it is kept"),
        "{text}"
    );
    assert!(
        !text.contains("bbbbbbbbbbbb  "),
        "not listed for removal: {text}"
    );
    assert!(
        store.dir("bbbbbbbbbbbb").is_dir(),
        "the install's entry stays"
    );
    assert!(!store.dir("aaaaaaaaaaaa").exists(), "the unused entry went");
    assert!(home.path("nsh/bbbbbbbbbbbb.lock").is_file());
    assert!(
        home.path("nsh/aaaaaaaaaaaa.lock").is_file(),
        "a removed entry's lock file stays, for an install that waits on it"
    );
    drop(held);
}

/// Records `project` in `setup.json` (in `config`) with its lock changed by `edit`; the machine's
/// store is `home/nsh`.
fn recorded(
    home: &Workspace,
    config: &Workspace,
    project: &Workspace,
    edit: impl Fn(String) -> String,
) -> FakeMachine {
    let lock = edit(project.read("Cargo.lock"));
    project.write("Cargo.lock", &lock);
    config.write(
        "rayx/setup.json",
        &format!(
            "{{\"version\":1,\"sets\":[\"base\"],\"roots\":[{:?}]}}",
            project.root.display().to_string()
        ),
    );
    FakeMachine::new()
        .with_home("/home/dev")
        .with_env("RAYX_HOME", &home.root.display().to_string())
        .with_env("XDG_CONFIG_HOME", &config.root.display().to_string())
        .with_text(
            config.path("rayx/setup.json"),
            &config.read("rayx/setup.json"),
        )
}

fn linux() -> rayx_cli::host::HostFacts {
    rayx_cli::host::HostFacts {
        os: Os::Linux,
        arch: rayx_cli::host::Arch::X64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

#[test]
fn nothing_is_pruned_while_a_recorded_projects_pin_cannot_be_read() {
    let (home, store) = store_with(&[("aaaaaaaaaaaa", Some(Kind::Distribution), 10)]);
    let config = Workspace::empty();
    let project = Workspace::fixture("nsh-fork-app");
    // A second NSH source in the lock: which entry the project uses is unknown.
    let machine = recorded(&home, &config, &project, |lock| {
        lock + "\n[[package]]\nname = \"nsh-platform-gpui\"\nversion = \"0.2.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
    });
    let error = kept_ids(&mut Runner::print(), &machine, &linux())
        .expect_err("unreadable")
        .to_string();
    assert!(
        error.contains("cannot tell which nsh") && error.contains("nothing is pruned"),
        "{error}"
    );
    let mut out = Vec::new();
    rayx_cli::nsh::offer_prune(
        &mut Runner::print(),
        &machine,
        &linux(),
        true,
        None,
        &mut out,
    );
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(text.contains("nothing is pruned"), "{text}");
    assert!(store.dir("aaaaaaaaaaaa").is_dir(), "nothing removed");
}

#[test]
fn setup_update_offers_prune_after_its_runs_unless_it_only_checks_or_targets_guests() {
    use rayx_cli::cli::SetupArgs;
    use rayx_cli::setup::update::after_update;

    let (home, store) = store_with(&[
        ("327bb9ea65d0", Some(Kind::Distribution), 10),
        ("5f0c2a91be47", Some(Kind::Source), 10),
        ("cccccccccccc", None, 10),
    ]);
    let config = Workspace::empty();
    let project = Workspace::fixture("nsh-fork-app");
    let machine = recorded(&home, &config, &project, |lock| {
        lock.replace(
            "?tag=fork-0.2.0-4#327bb9ea65d08ffac4af498cc7fbbfa18a1b3623",
            "?branch=main#5f0c2a91be4700000000000000000000000000aa",
        )
    });
    let update = |extra: fn(&mut SetupArgs)| {
        let mut args = SetupArgs {
            update: true,
            yes: true,
            ..SetupArgs::default()
        };
        extra(&mut args);
        let mut out = Vec::new();
        after_update(
            &args,
            &mut Runner::print(),
            &machine,
            &linux(),
            None,
            &mut out,
        );
        String::from_utf8(out).expect("UTF-8")
    };
    assert_eq!(
        update(|args| args.check = true),
        "",
        "--check leaves the store alone"
    );
    assert_eq!(
        update(|args| args.vm = vec!["app".into()]),
        "",
        "a run for guests leaves the store alone"
    );
    assert!(store.dir("cccccccccccc").is_dir());

    let text = update(|_| {});
    assert!(text.contains("cccccccccccc"), "{text}");
    assert!(text.contains("Removed cccccccccccc."), "{text}");
    assert!(!store.dir("cccccccccccc").exists());
    assert!(
        store.dir("327bb9ea65d0").is_dir() && store.dir("5f0c2a91be47").is_dir(),
        "the default and the recorded project's pin stay"
    );
}
