//! What an nsh command needs on the machine before it runs (design §6.1, RUN-2). The command and
//! its `--target` decide which `setup` sets must be complete besides the base set, whose last step
//! is the pinned nsh. What is missing is installed here when that needs no administrator rights
//! and no license answer, with a line saying so; otherwise the user is asked (in a terminal) or
//! told the exact `rayx setup` command. `--no-install` reports instead.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

use crate::host::path_env::UserPath;
use crate::host::{CommandSpec, Os, Privilege, Runner};
use crate::setup::{self, Action, Cx, Install, Machine, Plan, PlanEnv, Probed, Set, Step};

/// How the prerequisite step behaves.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// `--no-install`: report what is missing and stop instead of installing.
    pub no_install: bool,
    /// Whether the user can answer a question: standard input and output are a terminal.
    pub interactive: bool,
}

impl Options {
    /// The options of the current process: interactive when both standard streams are terminals.
    pub fn from_terminal(no_install: bool) -> Self {
        Self {
            no_install,
            interactive: io::stdin().is_terminal() && io::stdout().is_terminal(),
        }
    }
}

/// The value of `--target` in nsh's arguments (`--target web` or `--target=web`), before any `--`.
pub fn target(args: &[OsString]) -> Option<String> {
    let mut words = args.iter().map(|arg| arg.to_string_lossy().into_owned());
    while let Some(word) = words.next() {
        if word == "--" {
            return None;
        }
        if word == "--target" {
            return words.next();
        }
        if let Some(value) = word.strip_prefix("--target=") {
            return Some(value.to_string());
        }
    }
    None
}

/// Whether nsh's arguments name `--browser`, before any `--`.
pub fn has_browser(args: &[OsString]) -> bool {
    args.iter()
        .map(|arg| arg.to_string_lossy())
        .take_while(|word| word != "--")
        .any(|word| word == "--browser" || word.starts_with("--browser="))
}

/// The sets an nsh command needs besides the base set (design §6.1): web for any command with
/// `--target web`, and test with it for `test`; iOS for `ios`, `ios-simulator` and `catalyst`;
/// Android for `android`. Desktop targets, no target, and `build`, `check`, `init`, `keygen`,
/// `certify`, `publish`, `verify`, `revoke`, `symbolicate` need nothing more.
pub fn required_sets(command: &str, target: Option<&str>) -> Vec<Set> {
    if matches!(
        command,
        "build"
            | "check"
            | "init"
            | "keygen"
            | "certify"
            | "publish"
            | "verify"
            | "revoke"
            | "symbolicate"
    ) {
        return Vec::new();
    }
    match (command, target) {
        ("test", Some("web")) => vec![Set::Web, Set::Test],
        (_, Some("web")) => vec![Set::Web],
        (_, Some("ios" | "ios-simulator" | "catalyst")) => vec![Set::Ios],
        (_, Some("android")) => vec![Set::Android],
        _ => Vec::new(),
    }
}

/// The `rayx setup` command line that installs `sets` in `dir`.
pub fn setup_command(dir: &std::path::Path, sets: &[Set]) -> String {
    let mut command = format!("cd {} && rayx setup", dir.display());
    for set in sets.iter().filter(|set| **set != Set::Base) {
        command.push_str(" --");
        command.push_str(set.name());
    }
    command
}

/// Whether the owner has to be involved to install a step: it needs administrator rights, or it
/// asks a question (a license, as `winget` does without `--accept-package-agreements`).
fn needs_owner(step: &Step, probed: &Probed, cx: &mut Cx) -> bool {
    if step.privilege != Privilege::None || step.prompts {
        return true;
    }
    let Install::Actions(build) = &step.install else {
        return false;
    };
    build(cx, probed).is_ok_and(|actions| {
        actions
            .iter()
            .any(|action| matches!(action, Action::Run(spec) if asks_for_a_license(spec)))
    })
}

fn asks_for_a_license(spec: &CommandSpec) -> bool {
    let program = spec
        .program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(program.as_str(), "winget" | "winget.exe")
        && !spec
            .args
            .iter()
            .any(|arg| arg == "--accept-package-agreements")
}

/// Plans the base set and `sets` and installs what is missing, as [`ensure_plan`] does; `dir` is
/// where `rayx setup` would run. Returns how many steps installed something.
#[allow(clippy::too_many_arguments)]
pub fn ensure(
    env: &PlanEnv,
    sets: &[Set],
    dir: &std::path::Path,
    options: &Options,
    runner: &mut Runner,
    machine: &dyn Machine,
    user_path: &mut UserPath,
    out: &mut dyn Write,
    confirm: &mut dyn FnMut(&str) -> bool,
) -> Result<usize> {
    let (plan, command) = match setup::plan(env, sets) {
        Ok(plan) => (plan, setup_command(dir, sets)),
        // A host `rayx setup` cannot install for (another Linux distribution, say) is not a reason
        // to refuse to run: say so and let nsh find what is missing. The pinned nsh needs nothing
        // such a host lacks (it installs into rayx's store as anywhere), so it is still ensured,
        // with the command that installs it alone.
        Err(setup::SetupError::Unsupported(message)) => {
            let _ = writeln!(out, "rayx: prerequisites not checked: {message}");
            let Some(step) = setup::nsh_step(env) else {
                return Ok(0);
            };
            (
                Plan { steps: vec![step] },
                format!("rayx nsh install -C {}", dir.display()),
            )
        }
        Err(error) => return Err(anyhow!("{error}")),
    };
    let mut cx = Cx::new(env, runner, machine, user_path);
    ensure_plan(&plan, &command, options, &mut cx, out, confirm)
}

/// Checks a plan and installs what is missing; `command` is the `rayx setup` command line that
/// installs the same plan by hand. Returns how many steps installed something.
///
/// - Everything present: nothing happens.
/// - `no_install`: the missing items are listed and the command stops.
/// - Only unprivileged, silent items missing (the pinned nsh among them): a one-line notice,
///   then they are installed.
/// - Any item that needs `sudo`, administrator rights or a license answer: in a terminal the user
///   is asked whether to run the setup now (`confirm`); otherwise the command stops with the
///   `rayx setup` command line.
pub fn ensure_plan(
    plan: &Plan,
    command: &str,
    options: &Options,
    cx: &mut Cx,
    out: &mut dyn Write,
    confirm: &mut dyn FnMut(&str) -> bool,
) -> Result<usize> {
    let checked = setup::check(plan, cx);
    let missing: Vec<(&Step, &Probed)> = plan
        .steps
        .iter()
        .zip(&checked.probed)
        .filter(|(step, probed)| !probed.satisfied && !step.optional_for_builds)
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }
    let titles = missing
        .iter()
        .map(|(step, _)| step.title.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    if options.no_install {
        bail!("missing: {titles}. Run `{command}` to install it (--no-install is set)");
    }
    let owner = missing
        .iter()
        .filter(|(step, probed)| needs_owner(step, probed, cx))
        .map(|(step, _)| step.title.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if !owner.is_empty() {
        if !options.interactive {
            bail!(
                "missing: {titles}. {owner} needs administrator rights or a license answer: \
                 run `{command}` in a terminal"
            );
        }
        let question = format!(
            "Missing: {titles}.\n{owner} needs administrator rights or a license answer. \
             Run `{command}` now?"
        );
        if !confirm(&question) {
            bail!("missing: {titles}. Run `{command}` to install it");
        }
    }
    let _ = writeln!(out, "rayx: installing missing prerequisites: {titles}");
    let report = setup::execute(plan, cx, out);
    if let Some(message) = report.failure() {
        bail!("setup stopped: {message}. Run `{command}` to see the details");
    }
    for notice in &report.notices {
        let _ = writeln!(out, "{notice}");
    }
    Ok(report.executed())
}

/// The names Chrome, Edge and Chromium have on Linux, as nsh looks for them.
const LINUX_BROWSERS: [&str; 6] = [
    "google-chrome",
    "google-chrome-stable",
    "microsoft-edge",
    "microsoft-edge-stable",
    "chromium",
    "chromium-browser",
];

/// The Chrome, Edge or Chromium nsh finds on its own, in the places it looks (`nsh`'s `cdp.rs`).
pub fn system_browser(machine: &dyn Machine, os: Os) -> Option<PathBuf> {
    match os {
        Os::MacOs => {
            let apps = [
                "Google Chrome.app/Contents/MacOS/Google Chrome",
                "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
                "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                "Chromium.app/Contents/MacOS/Chromium",
            ];
            let mut roots = vec![PathBuf::from("/Applications")];
            roots.extend(machine.home().map(|home| home.join("Applications")));
            apps.iter()
                .flat_map(|app| {
                    roots.iter().map(move |root| {
                        app.split('/')
                            .fold(root.clone(), |path, part| path.join(part))
                    })
                })
                .find(|path| machine.exists(path))
        }
        Os::Windows => {
            let apps = [
                ["Google", "Chrome", "Application", "chrome.exe"],
                ["Microsoft", "Edge", "Application", "msedge.exe"],
                ["Chromium", "Application", "chrome.exe", ""],
            ];
            let roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
                .into_iter()
                .filter_map(|name| machine.env(name))
                .map(PathBuf::from)
                .collect();
            apps.iter()
                .flat_map(|app| {
                    roots.iter().map(move |root| {
                        app.iter()
                            .filter(|part| !part.is_empty())
                            .fold(root.clone(), |path, part| path.join(part))
                    })
                })
                .find(|path| machine.exists(path))
        }
        Os::Linux => LINUX_BROWSERS.iter().find_map(|name| machine.which(name)),
    }
}

/// `--browser <the test set's Chromium>` for `test --target web` without `--browser` on a host
/// where nsh would find no browser of its own (RUN-2); empty otherwise.
pub fn browser_args(command: &str, args: &[OsString], cx: &Cx) -> Vec<OsString> {
    if command != "test" || target(args).as_deref() != Some("web") || has_browser(args) {
        return Vec::new();
    }
    if system_browser(cx.machine, cx.env.host.os).is_some() {
        return Vec::new();
    }
    match setup::test_tools::playwright_chromium(cx) {
        Some(chromium) => vec![OsString::from("--browser"), chromium.into_os_string()],
        None => Vec::new(),
    }
}
