//! The base set on macOS: the Xcode Command Line Tools without the GUI dialog, Homebrew, rustup
//! and the project's toolchain.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::host::{Arch, CommandSpec, Privilege};

use super::{Action, Cx, PlanEnv, Probed, Set, SetupError, Step, rust};

/// While this file exists, `softwareupdate --list` offers the Command Line Tools.
pub const CLT_MARKER: &str = "/tmp/.com.apple.dt.CommandLineTools.installondemand.in-progress";

const HOMEBREW_INSTALLER: &str =
    "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh";

/// The Command Line Tools labels `softwareupdate --list` offers, oldest first. Both output styles
/// are understood: `* Label: Command Line Tools for Xcode-15.3` and `* Command Line Tools ...`.
pub fn parse_clt_labels(listing: &str) -> Vec<String> {
    let mut labels: Vec<String> = listing
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix('*')?.trim();
            let label = rest.strip_prefix("Label:").unwrap_or(rest).trim();
            label
                .starts_with("Command Line Tools")
                .then(|| label.to_string())
        })
        .collect();
    labels.sort_by_key(|label| version_key(label));
    labels.dedup();
    labels
}

/// The numbers in a label, so `Xcode-15.10` sorts after `Xcode-15.9`.
fn version_key(label: &str) -> Vec<u64> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    for c in label.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            current.push(c);
        } else if !current.is_empty() {
            numbers.push(current.parse().unwrap_or(0));
            current.clear();
        }
    }
    numbers
}

fn clt_probe(cx: &mut Cx) -> Probed {
    let selected = cx
        .query(&CommandSpec::new("xcode-select").arg("-p"))
        .is_some_and(|outcome| outcome.is_success());
    if selected {
        Probed::ok()
    } else {
        Probed::missing("the Command Line Tools are not installed")
    }
}

/// Installs the Command Line Tools without the GUI dialog: mark the on-demand install, install the
/// newest "Command Line Tools" label `softwareupdate` offers, and remove the marker. When no
/// label is offered, `xcode-select --install` opens the system installer and this waits for it.
fn install_clt(cx: &mut Cx) -> Result<(), SetupError> {
    let (marker, touch) = match &cx.clt_marker {
        Some(marker) => (marker.clone(), true),
        None => (PathBuf::from(CLT_MARKER), !cx.runner.is_dry_run()),
    };
    if touch {
        std::fs::write(&marker, "")?;
    }
    let result = install_clt_marked(cx);
    // The marker must never linger, whatever happened.
    if touch {
        let _ = std::fs::remove_file(&marker);
    }
    result
}

fn install_clt_marked(cx: &mut Cx) -> Result<(), SetupError> {
    let listing = cx
        .runner
        .query(&CommandSpec::new("softwareupdate").arg("--list"))?;
    let labels = parse_clt_labels(&format!("{}\n{}", listing.stdout, listing.stderr));
    if let Some(label) = labels.last() {
        cx.runner.run_checked(
            &CommandSpec::new("softwareupdate")
                .args(["--install", label.as_str()])
                .root()
                .interactive(),
        )?;
        return Ok(());
    }

    // Nothing offered: fall back to the system installer and wait for it to finish.
    cx.runner.run_checked(
        &CommandSpec::new("xcode-select")
            .arg("--install")
            .interactive(),
    )?;
    for _ in 0..720 {
        let done = cx
            .query(&CommandSpec::new("xcode-select").arg("-p"))
            .is_some_and(|outcome| outcome.is_success());
        if done {
            return Ok(());
        }
        std::thread::sleep(cx.poll_interval);
    }
    Err(SetupError::Prerequisite(
        "the Command Line Tools installer did not finish; complete it, then run `rayx setup` again"
            .to_string(),
    ))
}

fn clt_step() -> Step {
    Step::new(
        "command-line-tools",
        Set::Base,
        "Xcode Command Line Tools",
        Privilege::Root,
        clt_probe,
        |_, _| {
            Ok(vec![Action::Custom {
                description: format!(
                    "touch {CLT_MARKER}; sudo softwareupdate --install <newest \"Command Line \
                     Tools\" label from `softwareupdate --list`>; rm {CLT_MARKER} (when no label \
                     is offered: xcode-select --install, then wait for it)"
                ),
                run: Rc::new(install_clt),
            }])
        },
    )
}

/// The `brew` binary: on PATH, else in the Apple silicon or Intel default prefix.
pub fn brew_program(cx: &Cx) -> Option<PathBuf> {
    cx.machine.which("brew").or_else(|| {
        ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
            .into_iter()
            .map(Path::new)
            .find(|path| cx.machine.exists(path))
            .map(Path::to_path_buf)
    })
}

fn homebrew_step(env: &PlanEnv) -> Step {
    let arm = env.host.arch == Arch::Arm64;
    Step::new(
        "homebrew",
        Set::Base,
        "Homebrew",
        Privilege::None,
        |cx| {
            if brew_program(cx).is_some() {
                Probed::ok()
            } else {
                Probed::missing("brew not found")
            }
        },
        move |cx, _| {
            // The official installer runs in the user's console, so its `sudo` prompt reaches them.
            let mut installer = CommandSpec::new("/bin/bash")
                .arg("-c")
                .arg(format!(
                    "/bin/bash -c \"$(curl -fsSL {HOMEBREW_INSTALLER})\""
                ))
                .interactive();
            if cx.env.yes {
                installer = installer.env("NONINTERACTIVE", "1");
            }
            let mut actions = Vec::new();
            if cx.env.yes {
                // NONINTERACTIVE=1 makes the installer check for sudo without prompting, so ask
                // for the credentials first, in the user's console.
                actions.push(Action::Run(
                    CommandSpec::new("sudo").arg("-v").interactive(),
                ));
            }
            actions.push(Action::Run(installer));
            if arm {
                // Apple silicon installs under /opt/homebrew, which is not on PATH by default.
                actions.push(Action::AddToPath(PathBuf::from("/opt/homebrew/bin")));
            }
            Ok(actions)
        },
    )
}

pub fn steps(env: &PlanEnv) -> Result<Vec<Step>, SetupError> {
    Ok(vec![
        clt_step(),
        homebrew_step(env),
        rust::rustup_unix_step(),
        rust::toolchain_step(env),
        rust::cargo_path_step(),
    ])
}
