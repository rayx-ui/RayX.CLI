//! The embedded Playwright package: its contents, and a real `rayx app <fixture> test wasm` on an
//! app without a Playwright package, which creates one and passes its smoke spec.
//!
//! The run needs the web and test sets (`rayx setup --web --test`), so it is `#[ignore]`d in a
//! plain `cargo test`; select it with `cargo test --test app_playwright_template -- --ignored`.

mod common;

use common::Workspace;
use rayx_cli::app::playwright::render_template;

fn file<'a>(files: &'a [(std::path::PathBuf, String)], name: &str) -> &'a str {
    files
        .iter()
        .find(|(path, _)| path.to_str() == Some(name))
        .map(|(_, text)| text.as_str())
        .unwrap_or_else(|| panic!("the template has {name}"))
}

#[test]
fn the_template_pins_playwright_and_pnpm() {
    let files = render_template("1.59.1");

    let package: serde_json::Value =
        serde_json::from_str(file(&files, "package.json")).expect("package.json is JSON");
    assert_eq!(package["devDependencies"]["@playwright/test"], "1.59.1");
    assert!(
        package["packageManager"]
            .as_str()
            .is_some_and(|manager| manager.starts_with("pnpm@")),
        "{package}"
    );
    assert!(
        !file(&files, "package.json").contains("{{"),
        "every placeholder is filled"
    );
}

#[test]
fn the_config_reads_the_served_address_and_runs_two_device_scale_factors() {
    let files = render_template("1.59.1");
    let config = file(&files, "playwright.config.ts");

    assert!(config.contains("RAYX_APP_URL"));
    assert!(config.contains("name: \"chromium\""));
    assert!(config.contains("name: \"chromium-dpr2\""));
    assert!(config.contains("deviceScaleFactor: 2"));
    assert!(
        config.contains("channel: \"chromium\""),
        "headed runs use the full Chromium"
    );
}

#[test]
fn the_config_carries_the_verified_webgpu_flags_per_os() {
    let files = render_template("1.59.1");
    let config = file(&files, "playwright.config.ts");

    // Linux without a GPU renders through SwiftShader.
    for flag in [
        "--enable-unsafe-webgpu",
        "--enable-features=Vulkan",
        "--use-angle=swiftshader",
        "--use-vulkan=swiftshader",
    ] {
        assert!(config.contains(flag), "{flag}");
    }
    // The Windows headless shell.
    assert!(config.contains("--use-angle=d3d11"));
    assert!(config.contains("--disable-dawn-features=use_dxc"));
    // A headed run on a real GPU needs no flags.
    assert!(config.contains("if (headed) return [];"));
}

#[test]
fn the_fixture_waits_for_the_canvas_and_can_require_browser_webgpu() {
    let files = render_template("1.59.1");
    let fixtures = file(&files, "fixtures.ts");

    assert!(fixtures.contains("waitForSelector(\"canvas\""));
    assert!(fixtures.contains("RAYX_WEBGPU_REQUIRED"));
    assert!(fixtures.contains("BrowserWebGpu"));

    let smoke = file(&files, "smoke.spec.ts");
    assert!(smoke.contains("from \"./fixtures\""));
    assert!(smoke.contains("canvas"));
}

#[test]
fn the_gitignore_keeps_generated_files_out_of_the_repository() {
    let files = render_template("1.59.1");
    let ignore = file(&files, ".gitignore");

    for entry in ["node_modules", "test-results", "playwright-report"] {
        assert!(ignore.lines().any(|line| line == entry), "{entry}");
    }
}

#[test]
#[ignore = "needs the web and test sets: rayx setup --web --test"]
fn a_fixture_app_without_a_package_gets_one_and_passes_its_smoke_spec() {
    let workspace = Workspace::fixture("threaded_wasm_app");
    let app = workspace.root.display().to_string();
    let port = free_port().to_string();

    rayx_cli::app::run_unchecked(vec![
        app.clone(),
        "test".into(),
        "wasm".into(),
        "--port".into(),
        port.clone(),
    ])
    .expect("the generated package passes its smoke spec");

    let package = workspace.path("tests/e2e");
    for file in [
        "package.json",
        "playwright.config.ts",
        "fixtures.ts",
        "smoke.spec.ts",
        ".gitignore",
        "pnpm-lock.yaml",
    ] {
        assert!(package.join(file).is_file(), "{file}");
    }

    // The package is the user's now: a later run reuses it, edits included.
    workspace.write(
        "tests/e2e/smoke.spec.ts",
        &workspace
            .read("tests/e2e/smoke.spec.ts")
            .replace("draws on a canvas", "still draws on a canvas"),
    );
    rayx_cli::app::run_unchecked(vec![
        app,
        "test".into(),
        "wasm".into(),
        "--port".into(),
        port,
    ])
    .expect("the edited spec passes");
    assert!(
        workspace
            .read("tests/e2e/smoke.spec.ts")
            .contains("still draws on a canvas")
    );
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind")
        .local_addr()
        .expect("address")
        .port()
}
