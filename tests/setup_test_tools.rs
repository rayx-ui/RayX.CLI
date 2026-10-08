//! The test set: Node and pnpm per OS, Linux Chromium libraries, the project's Playwright package
//! and the WSL rule that a Windows `node` or `pnpm` never counts.

use std::path::PathBuf;

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::UserPath;
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::test_tools::playwright_system_packages;
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, plan, run_plan};

const UBUNTU_2404: &str = "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";
const UBUNTU_2204: &str = "ID=ubuntu\nVERSION_ID=\"22.04\"\nPRETTY_NAME=\"Ubuntu 22.04 LTS\"\n";

fn env(os: Os, arch: Arch, os_release: &str, wsl: bool, project: bool) -> PlanEnv {
    PlanEnv {
        host: HostFacts {
            os,
            arch,
            emulated: false,
            wsl,
            distro: (os == Os::Linux).then(|| Distro::parse(os_release)),
        },
        pins: Pins::resolve(None),
        project_root: project.then(|| PathBuf::from("/work/rayx")),
        tools_node: None,
        yes: false,
    }
}

fn linux() -> PlanEnv {
    env(Os::Linux, Arch::X64, UBUNTU_2404, false, false)
}

/// A path as a Linux or macOS shell prints it, whatever the host that runs this test.
fn show(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn path(parts: &[&str]) -> PathBuf {
    parts.iter().fold(PathBuf::new(), |acc, p| acc.join(p))
}

fn base_machine() -> FakeMachine {
    FakeMachine::new()
        .with_home("/home/dev")
        .with_env("LOCALAPPDATA", r"C:\Users\dev\AppData\Local")
}

fn check(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> (u8, String) {
    let plan = plan(env, &[Set::Test]).expect("plan");
    let mut path = UserPath::Profiles {
        files: vec![PathBuf::from("/home/dev/.profile")],
        home: PathBuf::from("/home/dev"),
    };
    let mut out = Vec::new();
    let mut cx = Cx::new(env, runner, machine, &mut path);
    let args = SetupArgs {
        check: true,
        test: true,
        ..SetupArgs::default()
    };
    let code = run_plan(&args, &plan, &mut cx, &mut out);
    let text = String::from_utf8(out).expect("UTF-8");
    // Unix output is compared with forward slashes and without the quotes a backslash would add.
    let text = if env.host.os == Os::Windows {
        text
    } else {
        text.replace('\\', "/").replace('\'', "")
    };
    (code, text)
}

/// The line of the report for the step whose title contains `needle`.
fn status<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines()
        .find(|l| l.starts_with('[') && l.contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "no step {needle:?}:
{text}"
            )
        })
}

fn bare(os: Os) -> Runner {
    Runner::record().with_os(os).with_root(false)
}

/// A runner whose `node --version` and `pnpm --version` answer as given.
fn with_versions(os: Os, node: Option<&'static str>, pnpm: bool) -> Runner {
    bare(os).responder(move |spec| {
        let program = spec.program.rsplit(['/', '\\']).next().unwrap_or("");
        match program {
            p if p.starts_with("node") => Some(match node {
                Some(version) => Outcome::success().with_stdout(format!("{version}\n")),
                None => Outcome::failure(127),
            }),
            p if p.starts_with("pnpm") => Some(if pnpm {
                Outcome::success().with_stdout("10.15.0\n")
            } else {
                Outcome::failure(127)
            }),
            _ => None,
        }
    })
}

#[test]
fn linux_gets_node_from_nodesource_pnpm_through_corepack_and_the_desktop_libraries() {
    let (code, text) = check(&linux(), &base_machine(), &mut bare(Os::Linux));
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("deb.nodesource.com/setup_22.x | sudo -E bash -"),
        "{text}"
    );
    assert!(text.contains("sudo apt-get install nodejs"), "{text}");
    let bin = PathBuf::from("/home/dev").join(".local").join("bin");
    assert!(
        text.contains(&format!(
            "corepack enable --install-directory {} pnpm",
            show(&bin)
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("add {} to the user PATH", show(&bin))),
        "{text}"
    );
    // xvfb, xdotool and the Chromium libraries install in the same batched apt command.
    assert_eq!(text.matches("apt-get install").count(), 2, "{text}");
    for package in [
        "xvfb",
        "xdotool",
        "libasound2t64",
        "libnss3",
        "fonts-liberation",
    ] {
        assert!(text.contains(package), "{package}:\n{text}");
    }
}

#[test]
fn chromium_library_names_follow_the_distribution() {
    let new = playwright_system_packages(true);
    let old = playwright_system_packages(false);
    assert!(new.contains(&"libasound2t64") && !new.contains(&"libasound2"));
    assert!(old.contains(&"libasound2") && !old.contains(&"libasound2t64"));
    assert_eq!(new.len(), old.len());

    let env_2204 = env(Os::Linux, Arch::X64, UBUNTU_2204, false, false);
    let (_, text) = check(&env_2204, &base_machine(), &mut bare(Os::Linux));
    assert!(text.contains("libasound2 "), "{text}");
    assert!(!text.contains("libasound2t64"), "{text}");
}

#[test]
fn windows_and_macos_install_node_their_own_way() {
    let windows = env(Os::Windows, Arch::Arm64, "", false, false);
    let (_, text) = check(&windows, &base_machine(), &mut bare(Os::Windows));
    assert!(
        text.contains("winget install --id OpenJS.NodeJS.LTS --exact"),
        "{text}"
    );
    assert!(
        text.contains("corepack enable --install-directory") && !text.contains("cmd /c"),
        "Windows shims start by their path, with no cmd /c wrapper:\n{text}"
    );

    let arm = env(Os::MacOs, Arch::Arm64, "", false, false);
    let machine = base_machine().with_file("/opt/homebrew/bin/brew");
    let (_, text) = check(&arm, &machine, &mut bare(Os::MacOs));
    assert!(
        text.contains("/opt/homebrew/bin/brew install node@22"),
        "{text}"
    );
    assert!(
        text.contains("add /opt/homebrew/opt/node@22/bin to the user PATH"),
        "{text}"
    );
    assert!(!text.contains("apt-get"), "{text}");

    let intel = env(Os::MacOs, Arch::X64, "", false, false);
    let (_, text) = check(&intel, &machine, &mut bare(Os::MacOs));
    assert!(
        text.contains("add /usr/local/opt/node@22/bin to the user PATH"),
        "{text}"
    );
}

#[test]
fn node_must_be_at_the_pinned_major_or_newer() {
    let machine = base_machine()
        .with_program("node", "/usr/bin/node")
        .with_program("pnpm", "/home/dev/.local/bin/pnpm");
    let status = |version: &'static str| -> String {
        let (_, text) = check(
            &linux(),
            &machine,
            &mut with_versions(Os::Linux, Some(version), true),
        );
        text.lines()
            .find(|l| l.contains("Node.js 22 or newer"))
            .unwrap_or_else(|| panic!("node line:\n{text}"))
            .to_string()
    };
    assert!(status("v22.3.1").starts_with("[ok     ]"));
    assert!(status("v24.0.0").starts_with("[ok     ]"));
    let old = status("v20.11.0");
    assert!(
        old.starts_with("[missing]") && old.contains("Node.js 20 is installed"),
        "{old}"
    );
}

#[test]
fn a_windows_node_and_pnpm_reached_through_mnt_c_do_not_count_in_wsl() {
    let wsl = env(Os::Linux, Arch::X64, UBUNTU_2404, true, false);
    let windows_tools = base_machine()
        .with_program("node", "/mnt/c/Program Files/nodejs/node")
        .with_program("pnpm", "/mnt/c/Users/dev/AppData/Roaming/npm/pnpm");
    let (_, text) = check(
        &wsl,
        &windows_tools,
        &mut with_versions(Os::Linux, Some("v24.0.0"), true),
    );
    assert!(
        text.contains("[missing] test: Node.js 22 or newer"),
        "{text}"
    );
    assert!(text.contains("[missing] test: pnpm"), "{text}");

    let linux_tools = base_machine()
        .with_program("node", "/usr/bin/node")
        .with_program("pnpm", "/home/dev/.local/bin/pnpm");
    let (_, text) = check(
        &wsl,
        &linux_tools,
        &mut with_versions(Os::Linux, Some("v24.0.0"), true),
    );
    assert!(
        text.contains("[ok     ] test: Node.js 22 or newer"),
        "{text}"
    );
    assert!(text.contains("[ok     ] test: pnpm"), "{text}");
}

#[test]
fn a_project_with_a_playwright_package_installs_packages_and_chromium() {
    let env = env(Os::Linux, Arch::X64, UBUNTU_2404, false, true);
    let tools = path(&["/work/rayx", "tools-node"]);
    let machine = base_machine().with_text(tools.join("package.json"), "{}");
    let (_, text) = check(&env, &machine, &mut bare(Os::Linux));
    let at = format!("cd {} && ", show(&tools));
    // The commands start with the package directory; Node's directory is put on their PATH.
    let line = |needle: &str| {
        text.lines()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("no line with {needle:?}:\n{text}"))
            .trim()
            .to_string()
    };
    let install = line("pnpm install --frozen-lockfile --prefer-offline");
    assert!(
        install.starts_with(&at) && install.contains("PATH="),
        "{install}"
    );
    let browser = line("pnpm exec playwright install chromium");
    assert!(
        browser.starts_with(&at) && browser.contains("PLAYWRIGHT_BROWSERS_PATH="),
        "the browser cache is pinned explicitly:\n{browser}"
    );
}

#[test]
fn the_playwright_package_location_follows_workspace_metadata() {
    let mut env = env(Os::Linux, Arch::X64, UBUNTU_2404, false, true);
    env.tools_node = Some("tools/e2e".to_string());
    let tools = path(&["/work/rayx", "tools/e2e"]);
    let machine = base_machine().with_text(tools.join("package.json"), "{}");
    let (_, text) = check(&env, &machine, &mut bare(Os::Linux));
    let install = text
        .lines()
        .find(|l| l.contains("pnpm install"))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        install
            .trim()
            .starts_with(&format!("cd {} && ", show(&tools))),
        "{install}"
    );
}

#[test]
fn a_project_without_a_playwright_package_has_nothing_to_install_there() {
    let env = env(Os::Linux, Arch::X64, UBUNTU_2404, false, true);
    let (_, text) = check(&env, &base_machine(), &mut bare(Os::Linux));
    assert!(
        text.contains("[ok     ] test: Playwright packages"),
        "{text}"
    );
    assert!(
        text.contains("[ok     ] test: Playwright Chromium"),
        "{text}"
    );
}

#[test]
fn chromium_counts_once_the_revision_is_in_the_per_user_cache() {
    let env = env(Os::Linux, Arch::X64, UBUNTU_2404, false, true);
    let tools = path(&["/work/rayx", "tools-node"]);
    let manifest = tools
        .join("node_modules")
        .join("playwright-core")
        .join("browsers.json");
    let browsers = r#"{"browsers":[{"name":"chromium","revision":"1181"},{"name":"firefox","revision":"1490"}]}"#;
    let cache = path(&["/home/dev", ".cache", "ms-playwright"]);
    let installed = base_machine()
        .with_text(tools.join("package.json"), "{}")
        .with_dir(tools.join("node_modules"))
        .with_text(&manifest, browsers)
        .with_dir(cache.join("chromium-1181"));
    let (_, text) = check(&env, &installed, &mut bare(Os::Linux));
    assert!(
        text.contains("[ok     ] test: Playwright packages"),
        "{text}"
    );
    assert!(
        text.contains("[ok     ] test: Playwright Chromium"),
        "{text}"
    );

    // An older revision in the cache does not satisfy the installed Playwright.
    let stale = base_machine()
        .with_text(tools.join("package.json"), "{}")
        .with_dir(tools.join("node_modules"))
        .with_text(&manifest, browsers)
        .with_dir(cache.join("chromium-1100"));
    let (_, text) = check(&env, &stale, &mut bare(Os::Linux));
    assert!(
        text.contains("[missing] test: Playwright Chromium"),
        "{text}"
    );
    assert!(text.contains("chromium-1181 is not in"), "{text}");

    // PLAYWRIGHT_BROWSERS_PATH=0 (a worktree-local cache) is never honoured.
    let local = stale.clone().with_env("PLAYWRIGHT_BROWSERS_PATH", "0");
    let (_, text) = check(&env, &local, &mut bare(Os::Linux));
    assert!(text.contains(&show(&cache).to_string()), "{text}");
}

#[test]
fn tools_an_install_just_added_are_found_without_a_new_shell() {
    // Right after the install the process PATH is the old one: node is only in Program Files,
    // pnpm only in the user bin directory. The re-probe must still see both.
    let windows = env(Os::Windows, Arch::X64, "", false, false);
    let node = path(&[r"C:\Program Files", "nodejs", "node.exe"]);
    let pnpm = path(&[r"C:\Users\dev", ".local", "bin", "pnpm.cmd"]);
    let machine = base_machine()
        .with_home(r"C:\Users\dev")
        .with_env("ProgramFiles", r"C:\Program Files")
        .with_file(&node)
        .with_file(&pnpm);
    let (_, text) = check(
        &windows,
        &machine,
        &mut with_versions(Os::Windows, Some("v24.0.0"), true),
    );
    assert!(
        status(&text, "Node.js 22 or newer").starts_with("[ok     ]"),
        "{text}"
    );
    assert!(status(&text, "pnpm").starts_with("[ok     ]"), "{text}");

    // macOS: Homebrew's keg-only node@22 is not on PATH either.
    let mac = env(Os::MacOs, Arch::Arm64, "", false, false);
    let keg = machine_with_home("/Users/dev").with_file("/opt/homebrew/opt/node@22/bin/node");
    let (_, text) = check(
        &mac,
        &keg,
        &mut with_versions(Os::MacOs, Some("v22.9.0"), false),
    );
    assert!(
        status(&text, "Node.js 22 or newer").starts_with("[ok     ]"),
        "{text}"
    );

    // Linux: pnpm in ~/.local/bin although that directory is not on PATH yet.
    let (_, text) = check(
        &linux(),
        &base_machine().with_file("/home/dev/.local/bin/pnpm"),
        &mut with_versions(Os::Linux, None, true),
    );
    assert!(status(&text, "pnpm").starts_with("[ok     ]"), "{text}");
}

fn machine_with_home(home: &str) -> FakeMachine {
    FakeMachine::new().with_home(home)
}

#[test]
fn the_later_commands_run_the_tools_by_absolute_path() {
    let env = env(Os::Linux, Arch::X64, UBUNTU_2404, false, true);
    let tools = path(&["/work/rayx", "tools-node"]);
    let machine = base_machine()
        .with_text(tools.join("package.json"), "{}")
        .with_file("/home/dev/.local/bin/pnpm");
    let (_, text) = check(&env, &machine, &mut bare(Os::Linux));
    assert!(
        text.contains("/home/dev/.local/bin/pnpm install --frozen-lockfile --prefer-offline"),
        "{text}"
    );
    assert!(
        text.contains("/home/dev/.local/bin/pnpm exec playwright install chromium"),
        "{text}"
    );
}

#[test]
fn wsl_skips_windows_entries_on_path_and_uses_the_linux_tool_behind_them() {
    let wsl = env(Os::Linux, Arch::X64, UBUNTU_2404, true, false);
    // The Windows pnpm comes first on PATH, the Linux one second.
    let machine = base_machine()
        .with_program("pnpm", "/mnt/c/Users/dev/AppData/Roaming/npm/pnpm")
        .with_program("pnpm", "/home/dev/.local/bin/pnpm")
        .with_program("node", "/mnt/c/Program Files/nodejs/node")
        .with_program("node", "/usr/bin/node");
    let (_, text) = check(
        &wsl,
        &machine,
        &mut with_versions(Os::Linux, Some("v22.1.0"), true),
    );
    assert!(status(&text, "pnpm").starts_with("[ok     ]"), "{text}");
    assert!(
        status(&text, "Node.js 22 or newer").starts_with("[ok     ]"),
        "{text}"
    );
}

#[test]
fn versions_found_are_recorded_for_doctor() {
    use rayx_cli::setup::check as check_plan;

    let env = linux();
    let machine = base_machine()
        .with_program("node", "/usr/bin/node")
        .with_program("pnpm", "/home/dev/.local/bin/pnpm");
    let plan = plan(&env, &[Set::Test]).expect("plan");
    let mut path = UserPath::Profiles {
        files: vec![PathBuf::from("/home/dev/.profile")],
        home: PathBuf::from("/home/dev"),
    };
    let mut runner = with_versions(Os::Linux, Some("v20.1.0"), true);
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    let checked = check_plan(&plan, &mut cx);
    let probe = |id: &str| {
        let index = plan.steps.iter().position(|s| s.id == id).expect("step");
        checked.probed[index].clone()
    };
    let node = probe("node");
    assert!(
        !node.satisfied && node.found.as_deref() == Some("20"),
        "{node:?}"
    );
    let pnpm = probe("pnpm");
    assert!(
        pnpm.satisfied && pnpm.found.as_deref() == Some("10.15.0"),
        "{pnpm:?}"
    );
}

#[test]
fn a_changed_lockfile_is_applied_and_playwright_installs_into_the_probed_cache() {
    let env = env(Os::Linux, Arch::X64, UBUNTU_2404, false, true);
    let tools = path(&["/work/rayx", "tools-node"]);
    let modules = tools.join("node_modules");
    let base = base_machine()
        .with_program("pnpm", "/home/dev/.local/bin/pnpm")
        .with_text(tools.join("package.json"), "{}")
        .with_dir(&modules)
        .with_file(modules.join(".modules.yaml"))
        .with_file(tools.join("pnpm-lock.yaml"));

    // Lockfile older than the install: nothing to do. Newer: install again.
    let fresh = base
        .clone()
        .with_modified(tools.join("pnpm-lock.yaml"), 100)
        .with_modified(modules.join(".modules.yaml"), 200);
    let (_, text) = check(&env, &fresh, &mut bare(Os::Linux));
    assert!(
        status(&text, "Playwright packages").starts_with("[ok     ]"),
        "{text}"
    );
    let drifted = base
        .clone()
        .with_modified(tools.join("pnpm-lock.yaml"), 300)
        .with_modified(modules.join(".modules.yaml"), 200);
    let (_, text) = check(&env, &drifted, &mut bare(Os::Linux));
    assert!(
        status(&text, "Playwright packages").starts_with("[missing]"),
        "{text}"
    );
    assert!(
        status(&text, "Playwright packages").contains("pnpm-lock.yaml changed"),
        "{text}"
    );

    // With PLAYWRIGHT_BROWSERS_PATH=0 in the environment, the install still goes to the
    // per-user cache that the probe reads.
    let zero = drifted.with_env("PLAYWRIGHT_BROWSERS_PATH", "0");
    let plan = plan(&env, &[Set::Test]).expect("plan");
    let mut recorder = bare(Os::Linux);
    let mut path = UserPath::Profiles {
        files: vec![PathBuf::from("/home/dev/.profile")],
        home: PathBuf::from("/home/dev"),
    };
    let mut cx = Cx::new(&env, &mut recorder, &zero, &mut path);
    let browser = plan
        .steps
        .iter()
        .find(|s| s.id == "playwright-chromium")
        .expect("step");
    let probed = (browser.probe)(&mut cx);
    let rayx_cli::setup::Install::Actions(build) = &browser.install else {
        panic!("actions install");
    };
    let actions = build(&mut cx, &probed).expect("actions");
    let rayx_cli::setup::Action::Run(spec) = &actions[0] else {
        panic!("a command");
    };
    let cache = path_of(&["/home/dev", ".cache", "ms-playwright"]);
    assert!(
        spec.env
            .iter()
            .any(|(k, v)| k == "PLAYWRIGHT_BROWSERS_PATH" && v == &cache),
        "{:?}",
        spec.env
    );
}

fn path_of(parts: &[&str]) -> String {
    path(parts).display().to_string()
}

#[test]
fn pnpm_commands_carry_the_node_directory_because_the_shim_starts_node_by_name() {
    use rayx_cli::setup::{Action, Install};

    // Node was just installed to Program Files and the process PATH does not have it yet; the
    // corepack pnpm shim would start `node` from PATH and fail without this.
    let env = env(Os::Windows, Arch::X64, "", false, true);
    let node_dir = path(&[r"C:\Program Files", "nodejs"]);
    let bin = path(&[r"C:\Users\dev", ".local", "bin"]);
    let tools = path(&[r"C:\work\rayx", "tools-node"]);
    let machine = base_machine()
        .with_home(r"C:\Users\dev")
        .with_env("ProgramFiles", r"C:\Program Files")
        .with_env("PATH", r"C:\Windows\System32")
        .with_file(node_dir.join("node.exe"))
        .with_file(bin.join("pnpm.cmd"))
        .with_text(tools.join("package.json"), "{}");
    let plan = plan(&env, &[Set::Test]).expect("plan");
    let mut recorder = bare(Os::Windows);
    let mut path_store = UserPath::Windows(Box::new(
        rayx_cli::host::path_env::MemoryPathStore::new(None),
    ));
    let mut cx = Cx::new(&env, &mut recorder, &machine, &mut path_store);

    for id in ["playwright-packages", "playwright-chromium"] {
        let step = plan.steps.iter().find(|s| s.id == id).expect("step");
        let probed = (step.probe)(&mut cx);
        let Install::Actions(build) = &step.install else {
            panic!("actions install");
        };
        let actions = build(&mut cx, &probed).expect("actions");
        let Action::Run(spec) = &actions[0] else {
            panic!("a command");
        };
        let path_value = spec
            .env
            .iter()
            .find(|(key, _)| key == "PATH")
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("{id}: no PATH in {:?}", spec.env));
        let entries: Vec<&str> = path_value.split(';').collect();
        assert_eq!(
            entries[0],
            node_dir.display().to_string(),
            "{id}: {path_value}"
        );
        assert_eq!(entries[1], bin.display().to_string(), "{id}: {path_value}");
        assert_eq!(
            entries.last().copied(),
            Some(r"C:\Windows\System32"),
            "{id}: {path_value}"
        );
        assert!(spec.program.ends_with("pnpm.cmd"), "{id}: {}", spec.program);
    }
}

#[test]
fn chromium_is_found_through_pnpms_store_when_playwright_core_is_not_hoisted() {
    let env = env(Os::Linux, Arch::X64, UBUNTU_2404, false, true);
    let tools = path(&["/work/rayx", "tools-node"]);
    // pnpm links only direct dependencies at the top of `node_modules`; `playwright-core` is a
    // dependency of `playwright` and lives in the store.
    let manifest = tools
        .join("node_modules")
        .join(".pnpm")
        .join("playwright-core@1.59.1")
        .join("node_modules")
        .join("playwright-core")
        .join("browsers.json");
    let browsers = r#"{"browsers":[{"name":"chromium","revision":"1181"}]}"#;
    let cache = path(&["/home/dev", ".cache", "ms-playwright"]);
    let installed = base_machine()
        .with_text(tools.join("package.json"), "{}")
        .with_dir(tools.join("node_modules").join("playwright"))
        .with_text(&manifest, browsers)
        .with_dir(cache.join("chromium-1181"));

    let (_, text) = check(&env, &installed, &mut bare(Os::Linux));

    assert!(
        text.contains("[ok     ] test: Playwright Chromium"),
        "{text}"
    );

    let without_browser = base_machine()
        .with_text(tools.join("package.json"), "{}")
        .with_dir(tools.join("node_modules").join("playwright"))
        .with_text(&manifest, browsers);
    let (_, text) = check(&env, &without_browser, &mut bare(Os::Linux));
    assert!(
        text.contains("[missing] test: Playwright Chromium"),
        "{text}"
    );
    assert!(text.contains("chromium-1181"), "{text}");
}
