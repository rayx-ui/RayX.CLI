//! Real installs of nsh (INST-3), ignored by default: they download upstream's release and build
//! the fork from source, which takes minutes and needs the network.
//!
//! ```text
//! cargo test --test proof_nsh_install -- --ignored --test-threads=1
//! ```
//!
//! Each test installs into a temporary `RAYX_HOME`, through the `rayx` binary.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::cargo_bin_cmd;
use common::Workspace;
use rayx_cli::project::pins::DEFAULTS;

/// The macOS (Apple silicon) archive is what the release proof downloads.
fn on_this_mac() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// `rayx nsh <args>` in `dir` with the store in `home`; the printed standard output.
fn rayx_nsh(home: &Path, dir: &Path, args: &[&str]) -> String {
    let output = cargo_bin_cmd!("rayx")
        .arg("nsh")
        .args(args)
        .current_dir(dir)
        .env("RAYX_HOME", home)
        .env_remove("CARGO")
        .output()
        .expect("rayx");
    assert!(
        output.status.success(),
        "rayx nsh {args:?} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8")
}

fn version_of(cli: &Path) -> String {
    let output = Command::new(cli)
        .arg("--version")
        .output()
        .expect("nsh --version");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("UTF-8")
        .trim()
        .to_string()
}

/// A tree's files and their modification times, to show a second install changes nothing.
fn snapshot(dir: &Path) -> Vec<(PathBuf, std::time::SystemTime)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, std::time::SystemTime)>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                // A source entry's target directory is Cargo's; the receipt and the checkout tell.
                if path.ends_with("target") {
                    continue;
                }
                walk(&path, out);
            } else if let Ok(modified) = meta.modified() {
                out.push((path, modified));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

#[test]
#[ignore = "downloads upstream's v0.2.0 release (network, about 150 MB)"]
fn upstream_v0_2_0_installs_from_its_release_and_again_changes_nothing() {
    if !on_this_mac() {
        eprintln!("skipped: the release proof runs on an Apple silicon Mac");
        return;
    }
    let home = Workspace::empty();
    let app = Workspace::fixture("nsh-ts-app");
    app.write("rayx.toml", "[nsh]\nversion = \"0.2.0\"\n");
    let cli = PathBuf::from(rayx_nsh(&home.root, &app.root, &["path"]).trim());
    assert_eq!(cli, home.path("nsh/v0.2.0/nsh/bin/nsh"));
    let version = version_of(&cli);
    assert!(version.starts_with("nsh 0.2.0"), "{version}");
    let receipt: serde_json::Value =
        serde_json::from_str(&home.read("nsh/v0.2.0/receipt.json")).expect("receipt");
    assert_eq!(receipt["kind"], "distribution");
    assert_eq!(
        receipt["url"],
        "https://github.com/longbridge/nsh/releases/download/v0.2.0/nsh-macos-arm64.tar.gz"
    );
    assert!(
        home.path("nsh/v0.2.0/nsh/runtime").is_dir(),
        "the prebuilt runtimes"
    );

    let before = snapshot(&home.path("nsh"));
    let again = rayx_nsh(&home.root, &app.root, &["install"]);
    assert!(again.contains("already installed"), "{again}");
    assert_eq!(snapshot(&home.path("nsh")), before, "nothing changed");
}

#[test]
#[ignore = "builds the fork's nsh-cli from source (network, several minutes)"]
fn fork_0_2_0_4_installs_from_source_and_again_changes_nothing() {
    let home = Workspace::empty();
    // fork-0.2.0-4's commit as a rev pin, which has no release whatever the fork publishes, so it
    // always builds from source.
    let app = Workspace::fixture("nsh-ts-app");
    app.write(
        "rayx.toml",
        &format!(
            "[nsh]\ngit = \"{}\"\nrev = \"{}\"\ncommit = \"{}\"\n",
            DEFAULTS.nsh.git, DEFAULTS.nsh.commit, DEFAULTS.nsh.commit
        ),
    );
    let id: String = DEFAULTS.nsh.commit.chars().take(12).collect();
    let cli = PathBuf::from(rayx_nsh(&home.root, &app.root, &["path"]).trim());
    assert_eq!(
        cli,
        home.path(&format!("nsh/{id}/source/target/release/nsh"))
    );
    assert_eq!(version_of(&cli), "nsh 0.2.0 (327bb9e)");
    let receipt: serde_json::Value =
        serde_json::from_str(&home.read(&format!("nsh/{id}/receipt.json"))).expect("receipt");
    assert_eq!(receipt["kind"], "source");
    assert_eq!(receipt["commit"], DEFAULTS.nsh.commit);

    let before = snapshot(&home.path("nsh"));
    let again = rayx_nsh(&home.root, &app.root, &["install"]);
    assert!(again.contains("already installed"), "{again}");
    assert_eq!(snapshot(&home.path("nsh")), before, "nothing changed");

    // The default pin's crate list is the `crates/nsh-*` of the fork at that commit.
    let mut crates: Vec<String> = std::fs::read_dir(home.path(&format!("nsh/{id}/source/crates")))
        .expect("crates")
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("nsh-"))
        .collect();
    crates.sort();
    let mut listed: Vec<String> = DEFAULTS.nsh.crates.iter().map(|c| c.to_string()).collect();
    listed.sort();
    assert_eq!(listed, crates);
}

#[test]
#[ignore = "asks the fork for the default tag (network)"]
fn the_default_tag_still_names_the_default_commit() {
    let output = Command::new("git")
        .args([
            "ls-remote",
            DEFAULTS.nsh.git,
            &format!("refs/tags/{}^{{}}", DEFAULTS.nsh.tag),
        ])
        .output()
        .expect("git");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(
        text.starts_with(DEFAULTS.nsh.commit),
        "the default tag still names the default commit: {text}"
    );
}
