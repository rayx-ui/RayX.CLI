//! `rayx app <dir> test <target>` for any app that uses RayX: which Playwright package, specs and
//! projects `test wasm` runs, and `test host` running the app package's own tests.

mod common;

use std::fs;

use common::Workspace;
use rayx_cli::app::playwright::{self, Placement};

const APP: &str = "apps/demo";

/// A path in one spelling, whatever form cargo and the standard library gave it.
fn same(path: &std::path::Path) -> std::path::PathBuf {
    fs::canonicalize(path).expect("canonical path")
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(ToString::to_string).collect()
}

fn run_app(workspace: &Workspace, parts: &[&str]) -> anyhow::Result<()> {
    let mut all = vec![workspace.path(APP).display().to_string()];
    all.extend(args(parts));
    rayx_cli::app::run_unchecked(all)
}

#[test]
fn the_package_is_found_through_workspace_metadata_first() {
    let workspace = Workspace::fixture("app-workspace");
    workspace.write("e2e/package.json", "{}");
    workspace.write("tools-node/package.json", "{}");
    workspace.append(
        "Cargo.toml",
        "\n[workspace.metadata.rayx]\ntools-node = \"e2e\"\n",
    );

    let package = playwright::resolve(&workspace.app(APP)).expect("resolves");

    assert_eq!(same(&package.dir), same(&workspace.path("e2e")));
    assert_eq!(package.placement, Placement::Workspace);
    assert!(!package.created);
}

#[test]
fn the_workspace_tools_node_directory_comes_next() {
    let workspace = Workspace::fixture("app-workspace");
    workspace.write("tools-node/package.json", "{}");
    workspace.write("apps/demo/tests/e2e/package.json", "{}");

    let package = playwright::resolve(&workspace.app(APP)).expect("resolves");

    assert_eq!(same(&package.dir), same(&workspace.path("tools-node")));
    assert_eq!(package.placement, Placement::Workspace);
}

#[test]
fn an_app_local_package_is_used_when_the_workspace_has_none() {
    let workspace = Workspace::fixture("app-workspace");
    workspace.write("apps/demo/tests/e2e/package.json", "{}");

    let package = playwright::resolve(&workspace.app(APP)).expect("resolves");

    assert_eq!(package.dir, workspace.path("apps/demo/tests/e2e"));
    assert_eq!(package.placement, Placement::AppLocal);
    assert!(!package.created);
}

#[test]
fn an_app_without_a_package_gets_one_from_the_template() {
    let workspace = Workspace::fixture("app-workspace");
    let app = workspace.app(APP);

    let package = playwright::resolve(&app).expect("creates");

    assert!(package.created);
    let dir = workspace.path("apps/demo/tests/e2e");
    assert_eq!(package.dir, dir);
    for file in [
        "package.json",
        "playwright.config.ts",
        "fixtures.ts",
        "smoke.spec.ts",
        ".gitignore",
    ] {
        assert!(dir.join(file).is_file(), "{file}");
    }
    assert!(
        workspace
            .read("apps/demo/tests/e2e/package.json")
            .contains("@playwright/test")
    );
    // Every spec of an app-local package runs.
    assert!(
        playwright::select_specs(&app, &package, &[])
            .expect("specs")
            .is_empty()
    );
    // A second run finds the package it created and leaves it alone.
    workspace.write("apps/demo/tests/e2e/smoke.spec.ts", "// edited\n");
    let again = playwright::resolve(&app).expect("resolves");
    assert!(!again.created);
    assert_eq!(
        workspace.read("apps/demo/tests/e2e/smoke.spec.ts"),
        "// edited\n"
    );
}

#[test]
fn a_shared_package_runs_the_directory_named_like_the_app() {
    // RayX's own layout: `apps/test_suite` with its specs in `tools-node/test_suite/`.
    let workspace = Workspace::fixture("app-workspace");
    workspace.rename_member("apps/demo", "apps/test_suite");
    workspace.write("tools-node/package.json", "{}");
    workspace.write("tools-node/test_suite/collapsible.spec.ts", "");
    workspace.write("tools-node/lab/other.spec.ts", "");
    let app = workspace.app("apps/test_suite");
    let package = playwright::resolve(&app).expect("resolves");

    assert_eq!(
        playwright::select_specs(&app, &package, &[]).expect("specs"),
        ["test_suite/"]
    );

    // The selectors given on the command line replace the default, whatever the app is called.
    assert_eq!(
        playwright::select_specs(
            &app,
            &package,
            &args(&["test_suite/collapsible.spec.ts", "lab/other.spec.ts"])
        )
        .expect("specs"),
        ["test_suite/collapsible.spec.ts", "lab/other.spec.ts"]
    );
}

#[test]
fn a_shared_package_without_a_directory_for_the_app_asks_for_a_spec() {
    let workspace = Workspace::fixture("app-workspace");
    workspace.write("tools-node/package.json", "{}");
    workspace.write("tools-node/lab/other.spec.ts", "");
    let app = workspace.app(APP);
    let package = playwright::resolve(&app).expect("resolves");

    let error = playwright::select_specs(&app, &package, &[]).expect_err("no default");

    let text = format!("{error:#}");
    assert!(text.contains("--playwright-test"), "{text}");
    assert!(text.contains("playwright_specs"), "{text}");
}

#[test]
fn package_metadata_overrides_the_specs_and_projects() {
    let workspace = Workspace::fixture("app-workspace");
    workspace.write("tools-node/package.json", "{}");
    let manifest = workspace.read("apps/demo/Cargo.toml").replace(
        "[package.metadata.rayx]\n",
        "[package.metadata.rayx]\nplaywright_specs = [\"suite/\", \"extra/a.spec.ts\"]\nplaywright_projects = [\"chromium\"]\n",
    );
    workspace.write("apps/demo/Cargo.toml", &manifest);
    let app = workspace.app(APP);
    let package = playwright::resolve(&app).expect("resolves");

    assert_eq!(
        playwright::select_specs(&app, &package, &[]).expect("specs"),
        ["suite/", "extra/a.spec.ts"]
    );
    assert_eq!(
        playwright::select_projects(&app).expect("projects"),
        ["chromium"]
    );
}

#[test]
fn without_metadata_every_project_of_the_config_runs() {
    let workspace = Workspace::fixture("app-workspace");
    let app = workspace.app(APP);

    assert!(
        playwright::select_projects(&app)
            .expect("projects")
            .is_empty()
    );
}

#[test]
fn the_served_address_and_the_webgpu_requirement_reach_the_fixtures() {
    let workspace = Workspace::fixture("app-workspace");
    let app = workspace.app(APP);
    let package = playwright::resolve(&app).expect("resolves");
    let run = |webgpu| {
        let command = playwright::test_command(
            &app,
            &package,
            &[],
            &[],
            &playwright::TestRun {
                url: "http://127.0.0.1:7878/",
                slug: &app.slug,
                features: "",
                headed: false,
                webgpu,
            },
        );
        let env = |key: &str| {
            command
                .get_envs()
                .find(|(name, _)| name.to_str() == Some(key))
                .and_then(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
        };
        (env("RAYX_APP_URL"), env("RAYX_WEBGPU_REQUIRED"))
    };

    assert_eq!(
        run(true),
        (
            Some("http://127.0.0.1:7878/".to_string()),
            Some("1".to_string())
        )
    );
    assert_eq!(run(false).1, Some("0".to_string()));
}

#[test]
fn test_host_runs_the_app_packages_own_tests() {
    let workspace = Workspace::fixture("app-workspace");
    workspace.write(
        "apps/demo/src/main.rs",
        "fn main() {}\n\n#[test]\nfn the_fixture_app_passes() {\n    std::fs::write(\"ran-by-test-host\", \"yes\").unwrap();\n}\n",
    );

    run_app(&workspace, &["test", "host"]).expect("the app test passes");

    // `cargo test` runs a package's tests with the package directory as the working directory.
    assert!(workspace.path("apps/demo/ran-by-test-host").is_file());
    // Output belongs to the selected workspace, not to the directory `rayx` ran in.
    assert!(workspace.path("target").is_dir());
}

#[test]
fn test_host_fails_when_an_app_test_fails() {
    let workspace = Workspace::fixture("app-workspace");
    workspace.write(
        "apps/demo/src/main.rs",
        "fn main() {}\n\n#[test]\nfn the_fixture_app_fails() {\n    panic!(\"broken on purpose\");\n}\n",
    );

    let error = run_app(&workspace, &["test", "host"]).expect_err("the test fails");

    assert!(format!("{error:#}").contains("failed"), "{error:#}");
}

#[test]
fn test_host_passes_arguments_after_the_separator_to_the_test_binary() {
    let workspace = Workspace::fixture("app-workspace");
    workspace.write(
        "apps/demo/src/main.rs",
        "fn main() {}\n\n#[test]\nfn first() {\n    std::fs::write(\"first-ran\", \"\").unwrap();\n}\n\n#[test]\nfn second() {\n    std::fs::write(\"second-ran\", \"\").unwrap();\n}\n",
    );

    run_app(&workspace, &["test", "host", "--", "first"]).expect("passes");

    assert!(workspace.path("apps/demo/first-ran").is_file());
    assert!(!workspace.path("apps/demo/second-ran").exists());
}

#[test]
fn android_and_ios_tests_say_they_are_not_supported_yet() {
    let workspace = Workspace::fixture("app-workspace");
    for target in ["android", "ios"] {
        let error = run_app(&workspace, &["test", target]).expect_err("unsupported");
        let text = format!("{error:#}");
        assert!(text.contains("not supported yet"), "{target}: {text}");
        assert!(text.contains(target), "{target}: {text}");
    }
    assert!(
        fs::read_dir(workspace.path("artifacts-temp")).is_err(),
        "nothing was built"
    );
}
