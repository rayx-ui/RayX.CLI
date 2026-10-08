//! Package layout: targets, dependency sources, toolchain pin and repository files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn metadata() -> serde_json::Value {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root())
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata emits JSON")
}

fn manifest() -> toml::Table {
    read_toml(&root().join("Cargo.toml"))
}

fn read_toml(path: &Path) -> toml::Table {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .parse()
        .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn package(metadata: &serde_json::Value) -> &serde_json::Value {
    let packages = metadata["packages"].as_array().expect("packages array");
    assert_eq!(packages.len(), 1, "the repository root is one package");
    &packages[0]
}

#[test]
fn package_has_library_and_binary_targets() {
    let metadata = metadata();
    let package = package(&metadata);
    assert_eq!(package["name"], "rayx-cli");
    assert_eq!(package["edition"], "2024");
    assert_eq!(package["license"], "Apache-2.0");
    assert_eq!(package["repository"], "https://github.com/rayx-ui/RayX.CLI");

    let targets = package["targets"].as_array().expect("targets array");
    let has = |kind: &str, name: &str| {
        targets.iter().any(|t| {
            t["name"] == name
                && t["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| k == kind))
        })
    };
    assert!(has("lib", "rayx_cli"), "library target rayx_cli");
    assert!(has("bin", "rayx"), "binary target rayx");
}

#[test]
fn main_is_a_thin_wrapper_over_the_library() {
    let main = fs::read_to_string(root().join("src/main.rs")).expect("read main.rs");
    assert!(
        main.contains("rayx_cli::run()"),
        "main.rs calls rayx_cli::run"
    );
    assert!(
        main.lines().filter(|l| !l.trim().is_empty()).count() <= 6,
        "main.rs stays a thin wrapper:\n{main}"
    );
}

#[test]
fn dependencies_come_from_crates_io_only() {
    let metadata = metadata();
    let package = package(&metadata);
    for dependency in package["dependencies"]
        .as_array()
        .expect("dependencies array")
    {
        let name = dependency["name"].as_str().unwrap_or("?");
        let source = dependency["source"].as_str().unwrap_or("");
        assert!(
            source.starts_with("registry+"),
            "dependency {name} must come from a registry, found source {source:?}"
        );
        assert!(
            dependency.get("path").is_none_or(|p| p.is_null()),
            "dependency {name} must not be a path dependency"
        );
    }

    let manifest = manifest();
    assert!(!manifest.contains_key("patch"), "no [patch] section");
    assert!(!manifest.contains_key("replace"), "no [replace] section");
    assert_registry_only(&toml::Value::Table(manifest), false);
}

/// Walks a manifest and fails on any dependency declared with `path` or `git`.
fn assert_registry_only(value: &toml::Value, in_dependencies: bool) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        if in_dependencies {
            if let Some(dependency) = child.as_table() {
                for forbidden in ["path", "git"] {
                    assert!(
                        !dependency.contains_key(forbidden),
                        "dependency {key} must not use `{forbidden}`"
                    );
                }
            }
        } else {
            assert_registry_only(child, key.ends_with("dependencies"));
        }
    }
}

#[test]
fn toolchain_pins_rust_1_95_0_with_clippy_and_rustfmt() {
    let toolchain = read_toml(&root().join("rust-toolchain.toml"));
    let toolchain = toolchain["toolchain"]
        .as_table()
        .expect("[toolchain] table");
    assert_eq!(toolchain["channel"].as_str(), Some("1.95.0"));
    let components: Vec<&str> = toolchain["components"]
        .as_array()
        .expect("components array")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    assert!(components.contains(&"clippy"), "clippy component");
    assert!(components.contains(&"rustfmt"), "rustfmt component");
}

#[test]
fn license_readme_and_gitignore_are_present() {
    let license = fs::read_to_string(root().join("LICENSE-APACHE")).expect("LICENSE-APACHE");
    assert!(license.contains("Apache License"));
    assert!(license.contains("Version 2.0, January 2004"));

    let readme = fs::read_to_string(root().join("README.md")).expect("README.md");
    assert!(
        readme.contains("rayx-cli-installer.sh") && readme.contains("| sh"),
        "README carries the shell install one-liner"
    );
    assert!(
        readme.contains("rayx-cli-installer.ps1") && readme.contains("| iex"),
        "README carries the PowerShell install one-liner"
    );
    for command in ["setup", "doctor", "wsl", "self update", "app", "fmt"] {
        assert!(
            readme.contains(&format!("`rayx {command}`")),
            "README command overview lists `rayx {command}`"
        );
    }

    let gitignore = fs::read_to_string(root().join(".gitignore")).expect(".gitignore");
    let ignored: Vec<&str> = gitignore.lines().map(str::trim).collect();
    assert!(ignored.contains(&"/target/"), "target/ is ignored");
    assert!(
        ignored.contains(&"/artifacts-temp/"),
        "artifacts-temp/ is ignored"
    );
}

#[test]
fn integration_tests_are_one_file_per_feature_group() {
    let tests = root().join("tests");
    for entry in fs::read_dir(&tests).expect("read tests/") {
        let entry = entry.expect("tests/ entry");
        let path = entry.path();
        if path.is_file() {
            assert_eq!(
                path.extension().and_then(|e| e.to_str()),
                Some("rs"),
                "{} is not a test group",
                path.display()
            );
        } else {
            // `fixtures` holds the fixture projects, `common` the helpers the groups share.
            assert!(
                matches!(
                    path.file_name().and_then(|n| n.to_str()),
                    Some("fixtures" | "common")
                ),
                "only tests/fixtures/ and tests/common/ may be directories, found {}",
                path.display()
            );
        }
    }
}
