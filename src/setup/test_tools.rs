//! The test set: Node.js at the pinned major or newer, pnpm through corepack, the Linux libraries
//! headed Chromium needs, and the project's Playwright package with its browser.
//! It replaces RayX xtask's `setup-wasm`.
//!
//! Probes and installs find Node tools on PATH and in the places the installs put them
//! (`%ProgramFiles%\nodejs`, the user bin directory, Homebrew's keg-only `node@<major>`), and run
//! them by absolute path: the running process's PATH does not change when an install adds a
//! directory, so a plain name would fail the re-probe on the first run.

use std::path::{Path, PathBuf};

use crate::host::{Arch, CommandSpec, Os, Privilege};

use super::{Action, Cx, PlanEnv, Probed, Set, SetupError, Step, macos};

const NODESOURCE: &str = "https://deb.nodesource.com";

/// The pinned Node.js major version.
pub fn node_major(env: &PlanEnv) -> u32 {
    env.pins.node.value.trim().parse().unwrap_or(22)
}

/// The directory pnpm's corepack shims go in, which also has to come first on PATH.
pub fn user_bin_dir(cx: &Cx) -> Option<PathBuf> {
    let home = cx.machine.home()?;
    Some(home.join(".local").join("bin"))
}

/// Where Node tools live besides PATH: the user bin directory, the Windows installer's
/// directory and Homebrew's keg-only `node@<major>`.
fn known_dirs(cx: &Cx) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = user_bin_dir(cx).into_iter().collect();
    match cx.env.host.os {
        Os::Windows => {
            let program_files = cx
                .machine
                .env("ProgramFiles")
                .unwrap_or_else(|| r"C:\Program Files".to_string());
            dirs.push(PathBuf::from(program_files).join("nodejs"));
        }
        Os::MacOs => {
            let major = node_major(cx.env);
            for prefix in ["/opt/homebrew", "/usr/local"] {
                dirs.push(
                    Path::new(prefix)
                        .join("opt")
                        .join(format!("node@{major}"))
                        .join("bin"),
                );
            }
        }
        Os::Linux => {}
    }
    dirs
}

/// A Node tool (`node`, `corepack`, `pnpm`) by absolute path. PATH comes first, skipping the
/// Windows programs WSL appends under `/mnt` (a Windows `pnpm` is never the one to use from
/// Linux), then the places an install puts it.
pub fn find_tool(cx: &Cx, name: &str) -> Option<PathBuf> {
    let from_path = cx
        .machine
        .which_all(name)
        .into_iter()
        .find(|found| !(cx.env.host.wsl && found.starts_with("/mnt")));
    if from_path.is_some() {
        return from_path;
    }
    let files: Vec<String> = if cx.env.host.os == Os::Windows {
        vec![format!("{name}.cmd"), format!("{name}.exe")]
    } else {
        vec![name.to_string()]
    };
    known_dirs(cx).into_iter().find_map(|dir| {
        files
            .iter()
            .map(|file| dir.join(file))
            .find(|candidate| cx.machine.exists(candidate))
    })
}

/// A command for a Node tool, by absolute path when it can be found. A `.cmd` shim started by
/// its full path is handled by the standard library, so no `cmd /c` wrapper (which mishandles
/// quoted paths with spaces) is needed.
pub fn node_command(cx: &Cx, name: &str, args: &[&str]) -> CommandSpec {
    let program = find_tool(cx, name).map_or_else(|| name.to_string(), |p| p.display().to_string());
    with_node_path(cx, CommandSpec::new(program).args(args.iter().copied()))
}

/// Puts the directories of the Node found by [`find_tool`] and of the user bin directory at the
/// front of the command's `PATH`. The `pnpm` shim that corepack writes starts `node` by name
/// (`#!/usr/bin/env node`, or a bare `node` in `pnpm.cmd`), and right after an install the running
/// process's PATH still lacks the directory Node went to.
pub fn with_node_path(cx: &Cx, spec: CommandSpec) -> CommandSpec {
    let mut dirs: Vec<PathBuf> = find_tool(cx, "node")
        .and_then(|node| node.parent().map(Path::to_path_buf))
        .into_iter()
        .collect();
    dirs.extend(user_bin_dir(cx));
    if dirs.is_empty() {
        return spec;
    }
    let separator = if cx.env.host.os == Os::Windows {
        ';'
    } else {
        ':'
    };
    let mut path = dirs
        .iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(&separator.to_string());
    if let Some(current) = cx.machine.env("PATH").filter(|current| !current.is_empty()) {
        path.push(separator);
        path.push_str(&current);
    }
    spec.env("PATH", path)
}

fn node_step(env: &PlanEnv) -> Step {
    let major = node_major(env);
    let os = env.host.os;
    let arm = env.host.arch == Arch::Arm64;
    Step::new(
        "node",
        Set::Test,
        format!("Node.js {major} or newer"),
        Privilege::None,
        move |cx| {
            let Some(node) = find_tool(cx, "node") else {
                return Probed::missing("node is not installed");
            };
            let version = cx
                .query(&CommandSpec::new(node.display().to_string()).arg("--version"))
                .filter(|outcome| outcome.is_success())
                .map(|outcome| outcome.stdout);
            let found: Option<u32> = version.as_deref().and_then(|text| {
                text.trim()
                    .trim_start_matches('v')
                    .split('.')
                    .next()?
                    .parse()
                    .ok()
            });
            match found {
                Some(found) if found >= major => Probed::ok().with_found(found.to_string()),
                Some(found) => Probed::missing(format!("Node.js {found} is installed"))
                    .with_found(found.to_string()),
                None => Probed::missing("node does not run"),
            }
        },
        move |cx, _| {
            let yes = cx.env.yes;
            Ok(match os {
                Os::Linux => vec![
                    Action::Run(
                        CommandSpec::new("sh")
                            .arg("-c")
                            .arg(format!(
                                "curl -fsSL {NODESOURCE}/setup_{major}.x | sudo -E bash -"
                            ))
                            .interactive(),
                    ),
                    Action::Run(
                        CommandSpec::new("apt-get")
                            .arg("install")
                            .args(yes.then_some("-y"))
                            .arg("nodejs")
                            .root()
                            .interactive(),
                    ),
                ],
                Os::Windows => {
                    let mut install = CommandSpec::new("winget").args([
                        "install",
                        "--id",
                        "OpenJS.NodeJS.LTS",
                        "--exact",
                    ]);
                    if yes {
                        install = install
                            .args(["--accept-package-agreements", "--accept-source-agreements"]);
                    }
                    vec![Action::Run(install.interactive())]
                }
                Os::MacOs => {
                    let brew = macos::brew_program(cx)
                        .map_or_else(|| "brew".to_string(), |path| path.display().to_string());
                    // node@<major> is keg-only: its bin directory goes on PATH.
                    let prefix = if arm { "/opt/homebrew" } else { "/usr/local" };
                    vec![
                        Action::Run(
                            CommandSpec::new(brew)
                                .args(["install", &format!("node@{major}")])
                                .interactive(),
                        ),
                        Action::AddToPath(
                            Path::new(prefix)
                                .join("opt")
                                .join(format!("node@{major}"))
                                .join("bin"),
                        ),
                    ]
                }
            })
        },
    )
}

fn pnpm_step() -> Step {
    Step::new(
        "pnpm",
        Set::Test,
        "pnpm (through corepack)",
        Privilege::None,
        |cx| {
            let Some(pnpm) = find_tool(cx, "pnpm") else {
                return Probed::missing("pnpm is not installed");
            };
            let version = cx
                .query(&with_node_path(
                    cx,
                    CommandSpec::new(pnpm.display().to_string()).arg("--version"),
                ))
                .filter(|outcome| outcome.is_success())
                .map(|outcome| outcome.stdout.trim().to_string());
            match version {
                Some(version) => Probed::ok().with_found(version),
                None => Probed::missing("pnpm does not run"),
            }
        },
        |cx, _| {
            let bin = user_bin_dir(cx).ok_or_else(|| {
                SetupError::Prerequisite("the home directory is unknown".to_string())
            })?;
            Ok(vec![
                Action::CreateDir(bin.clone()),
                Action::Run(
                    node_command(
                        cx,
                        "corepack",
                        &[
                            "enable",
                            "--install-directory",
                            &bin.display().to_string(),
                            "pnpm",
                        ],
                    )
                    .interactive(),
                ),
                // Ahead of anything from /mnt/c, so a Windows pnpm is never picked in WSL.
                Action::AddToPath(bin),
            ])
        },
    )
}

/// The Chromium libraries Playwright needs on Ubuntu and Debian. Releases that renamed their
/// libraries for the 64-bit `time_t` transition (Ubuntu 24.04, Debian 13) use the `t64` names.
pub fn playwright_system_packages(t64: bool) -> Vec<&'static str> {
    let t = |plain: &'static str, renamed: &'static str| if t64 { renamed } else { plain };
    vec![
        t("libasound2", "libasound2t64"),
        t("libatk-bridge2.0-0", "libatk-bridge2.0-0t64"),
        t("libatk1.0-0", "libatk1.0-0t64"),
        t("libatspi2.0-0", "libatspi2.0-0t64"),
        "libcairo2",
        t("libcups2", "libcups2t64"),
        "libdbus-1-3",
        "libdrm2",
        "libgbm1",
        t("libglib2.0-0", "libglib2.0-0t64"),
        "libnspr4",
        "libnss3",
        "libpango-1.0-0",
        "libx11-6",
        "libxcb1",
        "libxcomposite1",
        "libxdamage1",
        "libxext6",
        "libxfixes3",
        "libxkbcommon0",
        "libxrandr2",
        "fonts-noto-color-emoji",
        "fonts-unifont",
        "libfontconfig1",
        "libfreetype6",
        "xfonts-cyrillic",
        "xfonts-scalable",
        "fonts-liberation",
        "fonts-ipafont-gothic",
        "fonts-wqy-zenhei",
        "fonts-tlwg-loma-otf",
        "fonts-freefont-ttf",
    ]
}

/// Whether the distribution renamed libraries with a `t64` suffix.
fn uses_t64(env: &PlanEnv) -> bool {
    let Some(distro) = &env.host.distro else {
        return true;
    };
    let version: f32 = distro.version.parse().unwrap_or(0.0);
    match distro.id.as_str() {
        "debian" => version >= 13.0,
        "ubuntu" => version >= 24.04,
        _ => distro.id_like.iter().any(|id| id == "ubuntu"),
    }
}

/// Where Playwright keeps its browsers: `PLAYWRIGHT_BROWSERS_PATH` unless it is `0` (RayX never
/// keeps a worktree-local browser cache), else the per-user cache.
pub fn browser_cache(cx: &Cx) -> Option<PathBuf> {
    if let Some(path) = cx
        .machine
        .env("PLAYWRIGHT_BROWSERS_PATH")
        .filter(|path| path != "0" && !path.is_empty())
    {
        return Some(PathBuf::from(path));
    }
    Some(match cx.env.host.os {
        Os::Windows => PathBuf::from(cx.machine.env("LOCALAPPDATA")?).join("ms-playwright"),
        Os::MacOs => cx
            .machine
            .home()?
            .join("Library")
            .join("Caches")
            .join("ms-playwright"),
        Os::Linux => cx.machine.home()?.join(".cache").join("ms-playwright"),
    })
}

/// The project's Playwright package directory: `[workspace.metadata.rayx] tools-node`, or
/// `tools-node` under the workspace root.
pub fn playwright_dir(env: &PlanEnv) -> Option<PathBuf> {
    let root = env.project_root.as_ref()?;
    Some(root.join(env.tools_node.as_deref().unwrap_or("tools-node")))
}

fn playwright_install_step(dir: PathBuf) -> Step {
    let probe_dir = dir.clone();
    Step::new(
        "playwright-packages",
        Set::Test,
        format!("Playwright packages in {}", dir.display()),
        Privilege::None,
        move |cx| {
            // A project without a Playwright package has nothing to install here.
            if !cx.machine.exists(&probe_dir.join("package.json")) {
                return Probed::ok();
            }
            let modules = probe_dir.join("node_modules");
            if !cx.machine.exists(&modules) {
                return Probed::missing("node_modules is missing");
            }
            // A lockfile edited after the last install (a new Playwright version, say) has not
            // been applied: pnpm rewrites `.modules.yaml` at the end of every install.
            let lock = cx.machine.modified(&probe_dir.join("pnpm-lock.yaml"));
            let installed = cx.machine.modified(&modules.join(".modules.yaml"));
            match (lock, installed) {
                (Some(lock), Some(installed)) if lock > installed => {
                    Probed::missing("pnpm-lock.yaml changed since the last install")
                }
                _ => Probed::ok(),
            }
        },
        move |cx, _| {
            Ok(vec![Action::Run(
                node_command(
                    cx,
                    "pnpm",
                    &["install", "--frozen-lockfile", "--prefer-offline"],
                )
                .cwd(&dir)
                .interactive(),
            )])
        },
    )
}

/// The text of `playwright-core`'s `browsers.json` in a package's `node_modules`: at the top level
/// for a flat install, else in pnpm's store (`.pnpm/playwright-core@<version>/node_modules`), where
/// a dependency of a dependency lives.
fn browsers_manifest(cx: &Cx, package_dir: &Path) -> Option<String> {
    let modules = package_dir.join("node_modules");
    let flat = modules.join("playwright-core").join("browsers.json");
    if let Some(text) = cx.machine.read_to_string(&flat) {
        return Some(text);
    }
    let store = modules.join(".pnpm");
    let mut versions: Vec<String> = cx
        .machine
        .list_dir(&store)
        .into_iter()
        .filter(|name| name.starts_with("playwright-core@"))
        .collect();
    versions.sort();
    versions.iter().rev().find_map(|name| {
        cx.machine.read_to_string(
            &store
                .join(name)
                .join("node_modules")
                .join("playwright-core")
                .join("browsers.json"),
        )
    })
}

fn playwright_browser_step(dir: PathBuf) -> Step {
    let probe_dir = dir.clone();
    Step::new(
        "playwright-chromium",
        Set::Test,
        "Playwright Chromium (the per-user browser cache)",
        Privilege::None,
        move |cx| {
            if !cx.machine.exists(&probe_dir.join("package.json")) {
                return Probed::ok();
            }
            let Some(text) = browsers_manifest(cx, &probe_dir) else {
                return Probed::missing("Playwright is not installed yet");
            };
            let revision = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|json| {
                    json["browsers"]
                        .as_array()?
                        .iter()
                        .find(|b| b["name"] == "chromium")
                        .and_then(|b| b["revision"].as_str().map(str::to_string))
                });
            let (Some(revision), Some(cache)) = (revision, browser_cache(cx)) else {
                return Probed::missing("the Chromium revision is unknown");
            };
            if cx
                .machine
                .exists(&cache.join(format!("chromium-{revision}")))
            {
                Probed::ok()
            } else {
                Probed::missing(format!("chromium-{revision} is not in {}", cache.display()))
            }
        },
        move |cx, _| {
            let mut install =
                node_command(cx, "pnpm", &["exec", "playwright", "install", "chromium"])
                    .cwd(&dir)
                    .interactive();
            // Install into the cache the probe checks, even if the environment asks for a
            // worktree-local one (`PLAYWRIGHT_BROWSERS_PATH=0`).
            if let Some(cache) = browser_cache(cx) {
                install = install.env("PLAYWRIGHT_BROWSERS_PATH", cache.display().to_string());
            }
            Ok(vec![Action::Run(install)])
        },
    )
}

pub fn steps(env: &PlanEnv) -> Result<Vec<Step>, SetupError> {
    let mut steps = vec![node_step(env), pnpm_step()];
    if env.host.os == Os::Linux {
        steps.push(Step::apt(
            "xvfb-xdotool",
            Set::Test,
            "xvfb and xdotool (the X server and input of the desktop lanes)",
            &["xvfb", "xdotool"],
        ));
        steps.push(Step::apt(
            "playwright-system-libraries",
            Set::Test,
            "Chromium system libraries and fonts for Playwright",
            &playwright_system_packages(uses_t64(env)),
        ));
    }
    if let Some(dir) = playwright_dir(env) {
        steps.push(playwright_install_step(dir.clone()));
        steps.push(playwright_browser_step(dir));
    }
    Ok(steps)
}
