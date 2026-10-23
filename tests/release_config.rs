//! Release build configuration: `dist-workspace.toml` and the `release.yml` dist generated from it.

use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn dist_config() -> toml::Table {
    let path = root().join("dist-workspace.toml");
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .parse()
        .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn dist_table(config: &toml::Table) -> &toml::Table {
    config["dist"].as_table().expect("[dist] table")
}

fn strings(value: &toml::Value) -> Vec<&str> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("string"))
        .collect()
}

fn release_workflow() -> String {
    let path = root().join(".github/workflows/release.yml");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn dist_is_pinned_and_targets_installers_and_hosting_are_unchanged() {
    let config = dist_config();
    let dist = dist_table(&config);

    assert_eq!(dist["cargo-dist-version"].as_str(), Some("0.33.0"));
    assert_eq!(dist["ci"].as_str(), Some("github"));
    assert_eq!(strings(&dist["installers"]), ["shell", "powershell"]);
    assert_eq!(dist["checksum"].as_str(), Some("sha256"));
    assert_eq!(dist["install-updater"].as_bool(), Some(false));
    assert_eq!(dist["hosting"].as_str(), Some("github"));

    let mut targets = strings(&dist["targets"]);
    targets.sort_unstable();
    assert_eq!(
        targets,
        [
            "aarch64-apple-darwin",
            "aarch64-pc-windows-msvc",
            "aarch64-unknown-linux-musl",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-musl",
        ]
    );
}

#[test]
fn installers_use_rayx_home_with_attestations_and_pull_request_uploads() {
    let config = dist_config();
    let dist = dist_table(&config);

    assert_eq!(
        strings(&dist["install-path"]),
        ["$RAYX_HOME/bin", "~/.rayx/bin"]
    );
    assert_eq!(dist["github-attestations"].as_bool(), Some(true));
    assert_eq!(dist["pr-run-mode"].as_str(), Some("upload"));
}

#[test]
fn windows_arm64_builds_on_a_hosted_windows_x64_runner() {
    let config = dist_config();
    let dist = dist_table(&config);

    let runners = dist["github-custom-runners"]
        .as_table()
        .expect("[dist.github-custom-runners] table");
    let arm64 = runners["aarch64-pc-windows-msvc"]
        .as_table()
        .expect("aarch64-pc-windows-msvc runner");
    assert_eq!(arm64["runner"].as_str(), Some("windows-2022"));
    assert_eq!(arm64["host"].as_str(), Some("x86_64-pc-windows-msvc"));
    assert!(
        !arm64.contains_key("container"),
        "the Windows ARM64 build uses no cargo-xwin container"
    );
}

#[test]
fn generated_workflow_is_for_the_pinned_dist_and_is_not_marked_dirty() {
    let config = dist_config();
    let dist = dist_table(&config);
    assert!(
        !dist.contains_key("allow-dirty"),
        "release.yml changes only through dist generate"
    );

    let workflow = release_workflow();
    assert!(
        workflow.contains("cargo-dist/releases/download/v0.33.0/"),
        "release.yml installs the pinned dist version"
    );
    assert!(workflow.contains("actions/attest"), "attestation step");
}

#[test]
fn release_tags_still_trigger_the_workflow() {
    let workflow = release_workflow();
    assert!(
        workflow.contains("push:\n    tags:\n      - '**[0-9]+.[0-9]+.[0-9]+*'"),
        "the version-tag trigger stays"
    );
}
