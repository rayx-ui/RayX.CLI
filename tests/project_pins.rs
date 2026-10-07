//! Project discovery and pin resolution over fixture workspaces.

use std::fs;
use std::path::{Path, PathBuf};

use rayx_cli::host::{Outcome, Runner};
use rayx_cli::project::pins::{DEFAULTS, GPUX_CHECKOUT};
use rayx_cli::project::{Discovery, PinSource, Pins, Project, ProjectError};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> PathBuf {
    repo().join("tests/fixtures").join(name)
}

fn discover(cwd: &Path, app: Option<&Path>) -> Result<Project, ProjectError> {
    Project::discover(&mut Runner::execute(), cwd, app)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    fs::canonicalize(a).expect("canonical a") == fs::canonicalize(b).expect("canonical b")
}

#[test]
fn discovery_starts_at_the_current_directory_inside_the_app() {
    let app = fixture("plain-app").join("apps/demo");
    let project = discover(&app, None).expect("discovers");
    assert!(same_dir(&project.workspace_root, &fixture("plain-app")));
    assert!(same_dir(&project.app_dir, &app));
    let package = project.package("demo").expect("demo package");
    assert!(same_dir(package.dir(), &app));
    assert_eq!(
        package
            .rayx_metadata
            .as_ref()
            .and_then(|m| m["name"].as_str()),
        Some("demo"),
        "the package's [package.metadata.rayx] is carried"
    );
    assert!(project.rayx_metadata.is_none());
}

#[test]
fn the_same_app_is_selected_from_another_directory_by_relative_or_absolute_path() {
    let inside = discover(&fixture("plain-app").join("apps/demo"), None).expect("inside");

    // From the repository root with a relative path.
    let relative = discover(
        &repo(),
        Some(Path::new("tests/fixtures/plain-app/apps/demo")),
    )
    .expect("relative");
    // From an unrelated directory with the absolute path.
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let absolute = discover(
        elsewhere.path(),
        Some(&fixture("plain-app").join("apps/demo")),
    )
    .expect("absolute");
    // From a sibling directory with `..` components.
    let dotted = discover(
        &fixture("pinned-app"),
        Some(Path::new("../plain-app/apps/./demo")),
    )
    .expect("dotted");

    for other in [&relative, &absolute, &dotted] {
        assert!(same_dir(&other.app_dir, &inside.app_dir));
        assert!(same_dir(&other.workspace_root, &inside.workspace_root));
        assert_eq!(other.packages.len(), inside.packages.len());
    }
}

#[test]
fn a_directory_outside_any_workspace_is_not_a_project() {
    let empty = tempfile::tempdir().expect("temp dir");
    let error = discover(empty.path(), None).expect_err("no workspace here");
    assert!(matches!(error, ProjectError::NotAProject { .. }), "{error}");

    let missing = discover(empty.path(), Some(Path::new("no/such/app"))).expect_err("missing");
    assert!(
        matches!(missing, ProjectError::MissingDirectory(_)),
        "{missing}"
    );
}

#[test]
fn no_source_file_reads_a_compile_time_path() {
    let mut offenders = Vec::new();
    let mut stack = vec![repo().join("src")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                // Test-only code may read the fixtures by their compile-time location: the unit
                // test module at the end of a file and the `test_support` module.
                if path.file_name().is_some_and(|name| name == "test_support.rs") {
                    continue;
                }
                let text = fs::read_to_string(&path).expect("read source");
                let production = text
                    .split("#[cfg(test)]\nmod tests")
                    .next()
                    .unwrap_or(&text);
                if production.contains("CARGO_MANIFEST_DIR") {
                    offenders.push(path);
                }
            }
        }
    }
    assert!(offenders.is_empty(), "compile-time paths in {offenders:?}");
}

#[test]
fn pins_come_from_the_lockfile_the_toolchain_file_and_workspace_metadata() {
    let project = discover(&fixture("pinned-app").join("apps/demo"), None).expect("discovers");
    let pins = Pins::resolve(Some(&project));

    let pin = |value: &str| (value.to_string(), PinSource::Project);
    let got = |p: &rayx_cli::project::Pin| (p.value.clone(), p.source);

    assert_eq!(
        got(pins.wasm_bindgen.as_ref().expect("locked")),
        pin("0.2.129")
    );
    assert_eq!(
        got(pins.rust_toolchain.as_ref().expect("toolchain file")),
        pin("1.95.0")
    );
    assert_eq!(got(&pins.web_toolchain), pin("nightly-2030-01-01"));
    assert_eq!(got(&pins.node), pin("24"), "an integer value is accepted");
    assert_eq!(got(&pins.jdk), pin("23"));
    assert_eq!(got(&pins.playwright), pin("1.60.0"));
    assert_eq!(got(&pins.android_ndk), pin("29.0.1"));
    assert_eq!(got(&pins.android_platform), pin("android-35"));
    assert_eq!(
        got(pins.android_build_tools.as_ref().expect("build tools")),
        pin("35.0.0")
    );
    assert_eq!(got(&pins.android_ndk_api), pin("30"));
    assert_eq!(
        got(&pins.android_system_image),
        pin("android-35;google_apis_playstore")
    );
    assert_eq!(got(pins.xcode.as_ref().expect("xcode")), pin("26.1"));
}

#[test]
fn a_project_that_declares_nothing_gets_defaults_with_their_source() {
    let project = discover(&fixture("plain-app").join("apps/demo"), None).expect("discovers");
    let pins = Pins::resolve(Some(&project));
    assert!(pins.wasm_bindgen.is_none() && pins.rust_toolchain.is_none());
    assert!(pins.android_build_tools.is_none() && pins.xcode.is_none());
    assert_eq!(pins.web_toolchain.value, "nightly-2026-06-23");
    assert_eq!(pins.jdk.value, "21");
    assert_eq!(pins.playwright.value, "1.59.1");
    for pin in [
        &pins.web_toolchain,
        &pins.node,
        &pins.jdk,
        &pins.playwright,
        &pins.android_ndk,
        &pins.android_platform,
        &pins.android_ndk_api,
        &pins.android_system_image,
    ] {
        assert_eq!(pin.source, PinSource::Default, "{}", pin.value);
    }
}

#[test]
fn defaults_apply_outside_any_project() {
    let pins = Pins::resolve(None);
    assert_eq!(pins.web_toolchain.value, DEFAULTS.web_toolchain);
    assert_eq!(pins.web_toolchain.value, "nightly-2026-06-23");
    assert_eq!(pins.node.value, "22");
    assert_eq!(pins.jdk.value, "21");
    assert_eq!(pins.android_ndk.value, "28.0.12674087");
    assert_eq!(pins.android_platform.value, "android-34");
    assert_eq!(pins.android_ndk_api.value, "31");
    assert!(pins.android_build_tools.is_none());
    assert_eq!(pins.android_platform.source.as_str(), "default");
}

#[test]
fn a_gpux_checkout_that_declares_nothing_uses_the_gpux_profile_for_android() {
    let project = discover(&fixture("gpux-checkout"), None).expect("discovers");
    assert!(project.is_gpux_checkout());
    let pins = Pins::resolve(Some(&project));

    assert_eq!(pins.android_platform.value, GPUX_CHECKOUT.android_platform);
    assert_eq!(pins.android_platform.value, "android-36");
    assert_eq!(pins.android_platform.source, PinSource::GpuxCheckout);
    let build_tools = pins.android_build_tools.expect("gpux pins build tools");
    assert_eq!(build_tools.value, "36.0.0");
    assert_eq!(build_tools.source.as_str(), "gpux-checkout");
    assert_eq!(pins.android_ndk.value, "28.0.12674087");
    assert_eq!(pins.android_ndk.source, PinSource::GpuxCheckout);
    // Pins the gpux profile does not cover still come from the defaults.
    assert_eq!(pins.node.source, PinSource::Default);
    assert_eq!(pins.web_toolchain.value, "nightly-2026-06-23");
}

#[test]
fn project_metadata_beats_the_gpux_profile() {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("tooling/testkit/src")).expect("dirs");
    fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"tooling/testkit\"]\n\n[workspace.metadata.rayx]\nandroid-platform = \"android-35\"\n",
    )
    .expect("manifest");
    fs::write(
        dir.path().join("tooling/testkit/Cargo.toml"),
        "[package]\nname = \"gpux-testkit\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("member manifest");
    fs::write(dir.path().join("tooling/testkit/src/lib.rs"), "").expect("lib");

    let project = discover(dir.path(), None).expect("discovers");
    let pins = Pins::resolve(Some(&project));
    assert_eq!(pins.android_platform.value, "android-35");
    assert_eq!(pins.android_platform.source, PinSource::Project);
    assert_eq!(pins.android_ndk.source, PinSource::GpuxCheckout);
}

#[test]
fn a_missing_pinned_toolchain_does_not_install_and_the_pins_still_resolve_from_files() {
    // The fixture pins toolchain 1.2.3, which does not exist. Rustup would try to install it when
    // cargo runs there; discovery must not, and must still read the project's pins.
    let app = fixture("missing-toolchain").join("apps/demo");
    // `cargo test` forces RUSTUP_TOOLCHAIN on child processes, which hides the fixture's own
    // toolchain file, so script what rustup says when it may not install.
    let mut runner = Runner::record().respond(
        |_| true,
        Outcome {
            code: Some(1),
            stdout: String::new(),
            stderr: "error: toolchain '1.2.3-x86_64-unknown-linux-gnu' is not installed".into(),
        },
    );
    let project = Project::discover(&mut runner, &app, None).expect("discovers from the manifests");
    assert_eq!(project.discovery, Discovery::Files);
    assert!(same_dir(
        &project.workspace_root,
        &fixture("missing-toolchain")
    ));
    assert!(
        project.package("demo").is_some(),
        "a `apps/*` member is found"
    );

    let pins = Pins::resolve(Some(&project));
    assert_eq!(
        pins.rust_toolchain.as_ref().map(|p| p.value.as_str()),
        Some("1.2.3")
    );
    assert_eq!(pins.web_toolchain.value, "nightly-2031-02-03");
    assert_eq!(pins.web_toolchain.source, PinSource::Project);
    assert_eq!(
        pins.wasm_bindgen.as_ref().map(|p| p.value.as_str()),
        Some("0.2.129")
    );
}

#[test]
fn discovery_asks_rustup_not_to_install_anything() {
    let mut runner = Runner::record().respond(|_| true, Outcome::failure(1));
    let _ = Project::discover(&mut runner, &fixture("plain-app"), None);
    assert!(!runner.specs().is_empty());
    for spec in runner.specs() {
        assert_eq!(spec.program, "cargo");
        assert!(
            spec.env
                .iter()
                .any(|(key, value)| key == "RUSTUP_AUTO_INSTALL" && value == "0"),
            "{spec:?} may let rustup install a toolchain"
        );
    }
}

#[test]
fn without_a_working_cargo_the_manifests_still_identify_the_project() {
    // A fresh machine: cargo does not run at all.
    for (name, gpux) in [("pinned-app", false), ("gpux-checkout", true)] {
        let mut runner = Runner::record().respond(|_| true, Outcome::failure(127));
        let project = Project::discover(&mut runner, &fixture(name), None).expect("from files");
        assert_eq!(project.discovery, Discovery::Files, "{name}");
        assert_eq!(project.is_gpux_checkout(), gpux, "{name}");
    }
    let mut runner = Runner::record().respond(|_| true, Outcome::failure(127));
    let project = Project::discover(&mut runner, &fixture("pinned-app"), None).expect("from files");
    let pins = Pins::resolve(Some(&project));
    assert_eq!(pins.rust_toolchain.expect("toolchain").value, "1.95.0");
    assert_eq!(pins.node.value, "24");
    assert_eq!(pins.wasm_bindgen.expect("locked").value, "0.2.129");
    assert!(
        project.rayx_metadata.is_some() && project.package("demo").is_some(),
        "metadata and members come from the manifests"
    );
}

#[test]
fn a_directory_with_no_manifest_anywhere_is_still_not_a_project() {
    let empty = tempfile::tempdir().expect("temp dir");
    let mut runner = Runner::record().respond(|_| true, Outcome::failure(127));
    let error = Project::discover(&mut runner, empty.path(), None).expect_err("no manifest");
    assert!(matches!(error, ProjectError::NotAProject { .. }), "{error}");
}
