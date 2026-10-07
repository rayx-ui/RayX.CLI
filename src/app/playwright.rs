//! Browser tests for any RayX app: which Playwright package, which specs and which projects
//! `rayx app <dir> test wasm` runs, and the embedded package it creates when an app has none.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::app::AppDescriptor;
use crate::app::process::run as run_process;
use crate::host::{HostFacts, Os};

const TEMPLATE_PACKAGE_JSON: &str = include_str!("templates/playwright/package.json");
const TEMPLATE_CONFIG: &str = include_str!("templates/playwright/playwright.config.ts");
const TEMPLATE_FIXTURES: &str = include_str!("templates/playwright/fixtures.ts");
const TEMPLATE_SMOKE: &str = include_str!("templates/playwright/smoke.spec.ts");
const TEMPLATE_GITIGNORE: &str = include_str!("templates/playwright/gitignore");

/// Where an app's Playwright package lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Shared by the workspace's apps: `[workspace.metadata.rayx] tools-node` or `tools-node/`.
    Workspace,
    /// The app's own `tests/e2e/`.
    AppLocal,
}

/// A resolved Playwright package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaywrightPackage {
    pub dir: PathBuf,
    pub placement: Placement,
    /// The package was written from the embedded template by this run.
    pub created: bool,
}

/// The package's files rendered for a Playwright version: relative path and contents.
pub fn render_template(playwright_version: &str) -> Vec<(PathBuf, String)> {
    vec![
        (
            PathBuf::from("package.json"),
            TEMPLATE_PACKAGE_JSON.replace("{{PLAYWRIGHT_VERSION}}", playwright_version),
        ),
        (
            PathBuf::from("playwright.config.ts"),
            TEMPLATE_CONFIG.to_string(),
        ),
        (PathBuf::from("fixtures.ts"), TEMPLATE_FIXTURES.to_string()),
        (PathBuf::from("smoke.spec.ts"), TEMPLATE_SMOKE.to_string()),
        (PathBuf::from(".gitignore"), TEMPLATE_GITIGNORE.to_string()),
    ]
}

/// Finds the app's Playwright package, in this order: `[workspace.metadata.rayx] tools-node`, the
/// workspace's `tools-node/`, the app's `tests/e2e/`. When none exists the package is created in
/// `<app>/tests/e2e/` from the embedded template; existing files are never overwritten.
pub fn resolve(app: &AppDescriptor) -> Result<PlaywrightPackage> {
    let root = app.context.workspace_root();
    if let Some(configured) = app
        .context
        .project
        .rayx_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("tools-node"))
        .and_then(|value| value.as_str())
    {
        let dir = root.join(configured);
        if !dir.join("package.json").is_file() {
            bail!(
                "[workspace.metadata.rayx] tools-node points at {}, which has no package.json",
                dir.display()
            );
        }
        return Ok(PlaywrightPackage {
            dir,
            placement: Placement::Workspace,
            created: false,
        });
    }
    let shared = root.join("tools-node");
    if shared.join("package.json").is_file() {
        return Ok(PlaywrightPackage {
            dir: shared,
            placement: Placement::Workspace,
            created: false,
        });
    }
    let local = app.root.join("tests").join("e2e");
    if local.join("package.json").is_file() {
        return Ok(PlaywrightPackage {
            dir: local,
            placement: Placement::AppLocal,
            created: false,
        });
    }
    write_template(&local, &app.context.pins.playwright.value)?;
    Ok(PlaywrightPackage {
        dir: local,
        placement: Placement::AppLocal,
        created: true,
    })
}

/// Writes the template into `dir`, skipping files that already exist.
pub fn write_template(dir: &Path, playwright_version: &str) -> Result<()> {
    for (relative, contents) in render_template(playwright_version) {
        let path = dir.join(&relative);
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// The app's directory name in snake case, `test_suite` for `apps/test_suite`.
fn app_directory_snake(app: &AppDescriptor) -> String {
    app.root
        .file_name()
        .map(|name| {
            name.to_string_lossy()
                .replace('-', "_")
                .to_ascii_lowercase()
        })
        .unwrap_or_default()
}

/// The specs to run, as Playwright file filters relative to the package:
/// `--playwright-test` selectors; else `[package.metadata.rayx] playwright_specs`; else, in a
/// shared package, the app's own subdirectory (named like the app directory, or like its package
/// without a `rayx_` prefix), or, in an app-local package, every spec.
pub fn select_specs(
    app: &AppDescriptor,
    package: &PlaywrightPackage,
    selectors: &[String],
) -> Result<Vec<String>> {
    if !selectors.is_empty() {
        return selectors
            .iter()
            .map(|selector| normalize_selector(package, selector))
            .collect();
    }
    if let Some(specs) = app.playwright_specs()? {
        return Ok(specs);
    }
    if package.placement == Placement::AppLocal {
        return Ok(Vec::new());
    }
    let package_name = crate::app::package_name_from_manifest(&app.manifest())?;
    let snake = crate::app::rust_crate_file_stem(&package_name);
    let candidates = [
        app_directory_snake(app),
        snake.strip_prefix("rayx_").unwrap_or(&snake).to_string(),
        snake.clone(),
    ];
    for candidate in candidates {
        if !candidate.is_empty() && package.dir.join(&candidate).is_dir() {
            return Ok(vec![format!("{candidate}/")]);
        }
    }
    bail!(
        "{} has no directory for app `{}`: pass --playwright-test <spec> or set \
         [package.metadata.rayx] playwright_specs",
        package.dir.display(),
        app_directory_snake(app)
    )
}

/// The Playwright projects to run: `[package.metadata.rayx] playwright_projects`, else none, which
/// runs every project of the config.
pub fn select_projects(app: &AppDescriptor) -> Result<Vec<String>> {
    Ok(app.playwright_projects()?.unwrap_or_default())
}

/// `pnpm` by absolute path, so a Node installed during this session is found before the process
/// PATH knows about it: PATH first (skipping the Windows programs WSL appends under `/mnt`), then
/// the user bin directory corepack writes shims to, then the Windows installer's directory.
fn find_pnpm(host: &HostFacts) -> Option<PathBuf> {
    let names: &[&str] = if host.os == Os::Windows {
        &["pnpm.cmd", "pnpm.exe"]
    } else {
        &["pnpm"]
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if host.wsl {
        dirs.retain(|dir| !dir.starts_with("/mnt"));
    }
    if let Some(home) = home_dir() {
        dirs.push(home.join(".local").join("bin"));
    }
    if host.os == Os::Windows {
        let program_files =
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
        dirs.push(PathBuf::from(program_files).join("nodejs"));
    }
    dirs.into_iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// A `pnpm` command running in `dir`. Its `PATH` starts with the directory of the `pnpm` found and
/// the user bin directory: the shim corepack writes starts `node` by name.
pub fn pnpm_command(host: &HostFacts, dir: &Path) -> Command {
    let found = find_pnpm(host);
    let mut command = Command::new(found.as_deref().unwrap_or_else(|| Path::new("pnpm")));
    command.current_dir(dir);
    let mut front: Vec<PathBuf> = found
        .iter()
        .filter_map(|pnpm| pnpm.parent().map(Path::to_path_buf))
        .collect();
    front.extend(home_dir().map(|home| home.join(".local").join("bin")));
    if !front.is_empty() {
        let rest = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default();
        if let Ok(joined) = std::env::join_paths(front.into_iter().chain(rest)) {
            command.env("PATH", joined);
        }
    }
    command
}

/// The arguments of `pnpm install` for the package: reproducible when the package has a lockfile,
/// otherwise a plain install that writes one (a package just created from the template).
pub fn install_arguments(package: &PlaywrightPackage) -> Vec<&'static str> {
    if package.dir.join("pnpm-lock.yaml").is_file() {
        vec!["install", "--frozen-lockfile", "--prefer-offline"]
    } else {
        vec!["install", "--prefer-offline"]
    }
}

/// Installs the package's dependencies and the Chromium it drives.
pub fn prepare(app: &AppDescriptor, package: &PlaywrightPackage) -> Result<()> {
    let host = &app.context.host;
    let mut install = pnpm_command(host, &package.dir);
    install.args(install_arguments(package));
    run_process(&mut install).with_context(|| {
        format!(
            "installing the Playwright package in {} failed",
            package.dir.display()
        )
    })?;
    let mut browser = pnpm_command(host, &package.dir);
    browser.args(["exec", "playwright", "install", "chromium"]);
    run_process(&mut browser).with_context(|| {
        format!(
            "installing Chromium for the Playwright package in {} failed",
            package.dir.display()
        )
    })
}

/// What one Playwright invocation needs to know about the served app.
pub struct TestRun<'a> {
    pub url: &'a str,
    pub slug: &'a str,
    pub features: &'a str,
    pub headed: bool,
    pub webgpu: bool,
}

/// The `pnpm exec playwright test` command: spec filters, `--project` flags, and the environment
/// the template's fixtures read.
pub fn test_command(
    app: &AppDescriptor,
    package: &PlaywrightPackage,
    specs: &[String],
    projects: &[String],
    run: &TestRun<'_>,
) -> Command {
    let mut command = pnpm_command(&app.context.host, &package.dir);
    command.args(["exec", "playwright", "test"]);
    command.args(specs);
    for project in projects {
        command.arg(format!("--project={project}"));
    }
    if run.headed {
        command.arg("--headed");
    }
    command
        .env("RAYX_APP_URL", run.url)
        .env("RAYX_WASM_URL", run.url)
        .env("RAYX_APP_SLUG", run.slug)
        .env("RAYX_APP_FEATURES", run.features)
        .env("RAYX_WEBGPU_REQUIRED", if run.webgpu { "1" } else { "0" })
        .env("RAYX_WASM_THREADS_REQUIRED", "1")
        .env_remove("FORCE_COLOR")
        .env_remove("NO_COLOR");
    command
}

/// A `spec.ts:line` selector becomes the line of the `test(` that contains it, so a line inside a
/// test body runs that test.
fn normalize_selector(package: &PlaywrightPackage, selector: &str) -> Result<String> {
    let Some((path, line)) = selector.rsplit_once(':') else {
        return Ok(selector.to_owned());
    };
    let Ok(line) = line.parse::<usize>() else {
        return Ok(selector.to_owned());
    };
    let source = fs::read_to_string(package.dir.join(path))
        .with_context(|| format!("failed to read Playwright selector source {path}"))?;
    let Some(test_line) = nearest_test_line(&source, line) else {
        return Ok(selector.to_owned());
    };
    Ok(format!("{path}:{test_line}"))
}

fn nearest_test_line(source: &str, requested_line: usize) -> Option<usize> {
    source
        .lines()
        .take(requested_line)
        .enumerate()
        .filter_map(|(index, line)| line.trim_start().starts_with("test(").then_some(index + 1))
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::FixtureWorkspace;
    use std::ffi::OsStr;

    fn shared_package(workspace: &FixtureWorkspace, name: &str) -> PathBuf {
        let dir = workspace.root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("package.json"), "{}").unwrap();
        dir
    }

    fn env_of(command: &Command, key: &str) -> Option<String> {
        command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new(key))
            .and_then(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
    }

    fn args_of(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_template_renders_the_pinned_playwright_version() {
        let files = render_template("9.8.7");
        let names: Vec<_> = files
            .iter()
            .map(|(path, _)| path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "package.json",
                "playwright.config.ts",
                "fixtures.ts",
                "smoke.spec.ts",
                ".gitignore"
            ]
        );
        let package = &files[0].1;
        assert!(package.contains("\"@playwright/test\": \"9.8.7\""));
        assert!(!package.contains("{{"));
        let json: serde_json::Value = serde_json::from_str(package).unwrap();
        assert!(
            json["packageManager"]
                .as_str()
                .unwrap()
                .starts_with("pnpm@")
        );
    }

    #[test]
    fn the_template_reads_the_environment_rayx_sets() {
        let files = render_template("1.0.0");
        let fixtures = &files[2].1;
        assert!(fixtures.contains("RAYX_APP_URL"));
        assert!(fixtures.contains("RAYX_WEBGPU_REQUIRED"));
        let config = &files[1].1;
        assert!(config.contains("chromium-dpr2"));
        assert!(config.contains("RAYX_PLAYWRIGHT_OUTPUT_DIR"));
    }

    #[test]
    fn writing_the_template_never_overwrites_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("playwright.config.ts"), "mine").unwrap();

        write_template(dir.path(), "1.2.3").unwrap();

        assert_eq!(
            fs::read_to_string(dir.path().join("playwright.config.ts")).unwrap(),
            "mine"
        );
        assert!(dir.path().join("package.json").is_file());
        assert!(dir.path().join(".gitignore").is_file());
    }

    #[test]
    fn an_app_without_a_package_gets_the_template_in_its_tests_e2e_directory() {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();

        let package = resolve(&app).unwrap();

        assert!(package.created);
        assert_eq!(package.placement, Placement::AppLocal);
        assert_eq!(package.dir, app.root.join("tests").join("e2e"));
        let manifest = fs::read_to_string(package.dir.join("package.json")).unwrap();
        assert!(manifest.contains(&app.context.pins.playwright.value));

        let again = resolve(&app).unwrap();
        assert!(!again.created);
        assert_eq!(again.dir, package.dir);
    }

    #[test]
    fn the_workspace_tools_node_directory_is_shared_by_its_apps() {
        let workspace = FixtureWorkspace::new();
        let dir = shared_package(&workspace, "tools-node");
        let app = workspace.app();

        let package = resolve(&app).unwrap();

        assert_eq!(package.dir, dir);
        assert_eq!(package.placement, Placement::Workspace);
        assert!(!package.created);
        assert!(!app.root.join("tests").exists());
    }

    #[test]
    fn workspace_metadata_names_the_shared_package() {
        let workspace = FixtureWorkspace::new();
        shared_package(&workspace, "tools-node");
        let dir = shared_package(&workspace, "e2e");
        let mut app = workspace.app();
        app.context.project.rayx_metadata = Some(serde_json::json!({ "tools-node": "e2e" }));

        assert_eq!(resolve(&app).unwrap().dir, dir);

        app.context.project.rayx_metadata = Some(serde_json::json!({ "tools-node": "missing" }));
        let error = resolve(&app).unwrap_err();
        assert!(
            error.to_string().contains("has no package.json"),
            "{error:#}"
        );
    }

    #[test]
    fn an_app_local_package_beats_creating_one() {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let local = app.root.join("tests").join("e2e");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("package.json"), "{}").unwrap();

        let package = resolve(&app).unwrap();

        assert_eq!(package.dir, local);
        assert!(!package.created);
    }

    #[test]
    fn a_shared_package_runs_the_apps_own_subdirectory_by_default() {
        let workspace = FixtureWorkspace::new();
        let dir = shared_package(&workspace, "tools-node");
        fs::create_dir_all(dir.join("other")).unwrap();
        let app = workspace.app();
        let package = resolve(&app).unwrap();

        let error = select_specs(&app, &package, &[]).unwrap_err();
        assert!(error.to_string().contains("--playwright-test"), "{error:#}");

        // The app directory name is the first candidate.
        fs::create_dir_all(dir.join("demo")).unwrap();
        assert_eq!(select_specs(&app, &package, &[]).unwrap(), ["demo/"]);
    }

    #[test]
    fn the_package_name_without_its_rayx_prefix_is_the_second_candidate() {
        let workspace = FixtureWorkspace::new();
        let dir = shared_package(&workspace, "tools-node");
        fs::create_dir_all(dir.join("demo")).unwrap();
        // Rename the app directory so only the package name still matches.
        let renamed = workspace.root.join("apps").join("renamed");
        fs::rename(workspace.root.join("apps").join("demo"), &renamed).unwrap();
        let workspace_manifest = workspace.root.join("Cargo.toml");
        let text = fs::read_to_string(&workspace_manifest).unwrap();
        fs::write(
            &workspace_manifest,
            text.replace("apps/demo", "apps/renamed"),
        )
        .unwrap();
        let app = crate::app::test_support::resolve(&renamed).unwrap();
        let package = resolve(&app).unwrap();

        assert_eq!(select_specs(&app, &package, &[]).unwrap(), ["demo/"]);
    }

    #[test]
    fn an_app_local_package_runs_every_spec_by_default() {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let package = resolve(&app).unwrap();

        assert!(select_specs(&app, &package, &[]).unwrap().is_empty());
    }

    #[test]
    fn manifest_specs_and_command_line_selectors_override_the_default() {
        let workspace = FixtureWorkspace::new();
        let dir = shared_package(&workspace, "tools-node");
        fs::create_dir_all(dir.join("demo")).unwrap();
        fs::write(
            workspace.root.join("apps/demo/Cargo.toml"),
            "[package]\nname = \"rayx_demo\"\nversion = \"0.1.0\"\n[package.metadata.rayx]\nplaywright_specs = [\"custom/\"]\nplaywright_projects = [\"chromium\"]\n[dependencies]\nrayx = { path = \"../../crates/rayx\" }\n",
        )
        .unwrap();
        let app = workspace.app();
        let package = resolve(&app).unwrap();

        assert_eq!(select_specs(&app, &package, &[]).unwrap(), ["custom/"]);
        assert_eq!(
            select_specs(&app, &package, &["demo/a.spec.ts".to_string()]).unwrap(),
            ["demo/a.spec.ts"]
        );
        assert_eq!(select_projects(&app).unwrap(), ["chromium"]);
    }

    #[test]
    fn a_line_selector_resolves_to_the_test_that_contains_it() {
        let workspace = FixtureWorkspace::new();
        let dir = shared_package(&workspace, "tools-node");
        fs::write(
            dir.join("a.spec.ts"),
            "test(\"first\", () => {\n  expect(true);\n});\n\ntest(\"second\", () => {\n  expect(true);\n});\n",
        )
        .unwrap();
        let app = workspace.app();
        let package = resolve(&app).unwrap();

        let resolve_line = |line: &str| {
            select_specs(&app, &package, &[format!("a.spec.ts:{line}")])
                .unwrap()
                .remove(0)
        };
        assert_eq!(resolve_line("2"), "a.spec.ts:1");
        assert_eq!(resolve_line("6"), "a.spec.ts:5");
        assert_eq!(
            select_specs(&app, &package, &["a.spec.ts".to_string()]).unwrap(),
            ["a.spec.ts"]
        );
    }

    #[test]
    fn nearest_test_line_ignores_lines_before_the_first_test() {
        let source = "import x;\n\ntest(\"a\", () => {});\n";
        assert_eq!(nearest_test_line(source, 2), None);
        assert_eq!(nearest_test_line(source, 3), Some(3));
    }

    #[test]
    fn a_lockfile_makes_the_install_reproducible() {
        let dir = tempfile::tempdir().unwrap();
        let package = PlaywrightPackage {
            dir: dir.path().to_path_buf(),
            placement: Placement::AppLocal,
            created: false,
        };
        assert_eq!(install_arguments(&package), ["install", "--prefer-offline"]);

        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(
            install_arguments(&package),
            ["install", "--frozen-lockfile", "--prefer-offline"]
        );
    }

    #[test]
    fn the_test_command_passes_the_served_app_to_the_fixtures() {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let package = resolve(&app).unwrap();

        let command = test_command(
            &app,
            &package,
            &["demo/".to_string()],
            &["chromium".to_string(), "chromium-dpr2".to_string()],
            &TestRun {
                url: "http://127.0.0.1:7878/",
                slug: "demo",
                features: "profile-a",
                headed: true,
                webgpu: true,
            },
        );

        assert_eq!(
            args_of(&command),
            [
                "exec",
                "playwright",
                "test",
                "demo/",
                "--project=chromium",
                "--project=chromium-dpr2",
                "--headed"
            ]
        );
        assert_eq!(command.get_current_dir(), Some(package.dir.as_path()));
        assert_eq!(
            env_of(&command, "RAYX_APP_URL").unwrap(),
            "http://127.0.0.1:7878/"
        );
        assert_eq!(
            env_of(&command, "RAYX_WASM_URL").unwrap(),
            "http://127.0.0.1:7878/"
        );
        assert_eq!(env_of(&command, "RAYX_APP_SLUG").unwrap(), "demo");
        assert_eq!(env_of(&command, "RAYX_APP_FEATURES").unwrap(), "profile-a");
        assert_eq!(env_of(&command, "RAYX_WEBGPU_REQUIRED").unwrap(), "1");
        assert_eq!(env_of(&command, "RAYX_WASM_THREADS_REQUIRED").unwrap(), "1");
    }

    #[test]
    fn without_projects_or_headed_the_command_runs_every_configured_project() {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let package = resolve(&app).unwrap();

        let command = test_command(
            &app,
            &package,
            &[],
            &[],
            &TestRun {
                url: "http://127.0.0.1:1/",
                slug: "demo",
                features: "",
                headed: false,
                webgpu: false,
            },
        );

        assert_eq!(args_of(&command), ["exec", "playwright", "test"]);
        assert_eq!(env_of(&command, "RAYX_WEBGPU_REQUIRED").unwrap(), "0");
    }
}
