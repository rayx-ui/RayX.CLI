//! Rust steps shared by every operating system: rustup, the project's toolchain and the cargo
//! bin directory on the user PATH.

use std::path::PathBuf;

use crate::host::path_env::PathChange;
use crate::host::{CommandSpec, Os, Privilege};

use super::{Action, Cx, PlanEnv, Probed, Set, SetupError, Step, windows};

/// `~/.cargo/bin`.
pub fn cargo_bin_dir(cx: &Cx) -> Option<PathBuf> {
    cx.machine
        .home()
        .map(|home| home.join(".cargo").join("bin"))
}

fn exe(name: &str, os: Os) -> String {
    if os == Os::Windows {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The `rustup` to run: the one in `~/.cargo/bin` when it exists (a shell opened before the
/// install does not have that directory on PATH yet), else whatever PATH offers.
pub fn rustup_program(cx: &Cx) -> String {
    if let Some(bin) = cargo_bin_dir(cx) {
        let candidate = bin.join(exe("rustup", cx.env.host.os));
        if cx.machine.exists(&candidate) {
            return candidate.display().to_string();
        }
    }
    "rustup".to_string()
}

/// A tool installed by cargo or rustup: on PATH, else in `~/.cargo/bin` (a shell opened before
/// the install does not have that directory yet), else just its name.
pub fn tool_program(cx: &Cx, name: &str) -> String {
    if let Some(found) = cx.machine.which(name) {
        return found.display().to_string();
    }
    if let Some(bin) = cargo_bin_dir(cx) {
        let candidate = bin.join(exe(name, cx.env.host.os));
        if cx.machine.exists(&candidate) {
            return candidate.display().to_string();
        }
    }
    name.to_string()
}

fn has_rustup(cx: &mut Cx) -> bool {
    if cx.machine.which("rustup").is_some() {
        return true;
    }
    let program = rustup_program(cx);
    program != "rustup"
}

/// The rustup step for Linux and macOS: the official installer script, run in the user's console
/// without a default toolchain and without editing shell profiles (the PATH step does that).
pub fn rustup_unix_step() -> Step {
    Step::new(
        "rustup",
        Set::Base,
        "rustup (the Rust toolchain installer)",
        Privilege::None,
        |cx| {
            if has_rustup(cx) {
                Probed::ok()
            } else {
                Probed::missing("rustup not found")
            }
        },
        |_, _| {
            Ok(vec![Action::Run(
                CommandSpec::new("sh")
                    .arg("-c")
                    .arg(
                        "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
                         --default-toolchain none --no-modify-path",
                    )
                    .interactive(),
            )])
        },
    )
    .with_fix_hint("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh")
}

/// The project's pinned toolchain (from `rust-toolchain.toml`) when `setup` runs inside a project
/// that has one, else the stable toolchain as the default. On Windows the toolchain must be the
/// native one: an emulated x64 toolchain on an ARM64 machine does not count.
pub fn toolchain_step(env: &PlanEnv) -> Step {
    let pinned = env
        .pins
        .rust_toolchain
        .as_ref()
        .map(|pin| pin.value.clone());
    let root = env.project_root.clone();
    let triple = (env.host.os == Os::Windows).then(|| windows::native_triple(env.host.arch));
    let probe_pinned = pinned.clone();
    let title = match &pinned {
        Some(channel) => format!("Rust toolchain {channel} (the project's rust-toolchain.toml)"),
        None => "Rust stable toolchain".to_string(),
    };
    Step::new(
        "rust-toolchain",
        Set::Base,
        title,
        Privilege::None,
        move |cx| {
            let program = rustup_program(cx);
            let Some(list) = cx.query(&CommandSpec::new(program).args(["toolchain", "list"]))
            else {
                return Probed::missing("rustup is not installed");
            };
            // Each line is `<toolchain> [(default)]`.
            let entries: Vec<(&str, bool)> = list
                .stdout
                .lines()
                .filter_map(|line| {
                    let name = line.split_whitespace().next()?;
                    Some((name, line.contains("default)")))
                })
                .filter(|(name, _)| triple.is_none_or(|triple| name.ends_with(triple)))
                .collect();
            let found = match &probe_pinned {
                Some(channel) => entries
                    .iter()
                    .find(|(name, _)| toolchain_matches(name, channel)),
                None => entries.iter().find(|(_, is_default)| *is_default),
            }
            .map(|(name, _)| name.to_string());
            if let Some(found) = found {
                Probed::ok().with_found(found)
            } else {
                Probed::missing(match (&probe_pinned, triple) {
                    (Some(channel), Some(triple)) => format!("{channel}-{triple} is not installed"),
                    (Some(channel), None) => format!("{channel} is not installed"),
                    (None, _) => "no default toolchain is installed".to_string(),
                })
            }
        },
        move |cx, _| {
            let program = rustup_program(cx);
            Ok(match (&pinned, &root) {
                // Inside the project, `rustup toolchain install` reads rust-toolchain.toml: the
                // channel, its components and its targets.
                (Some(_), Some(root)) => vec![Action::Run(
                    CommandSpec::new(program)
                        .args(["toolchain", "install"])
                        .cwd(root)
                        .interactive(),
                )],
                (Some(channel), None) => vec![Action::Run(
                    CommandSpec::new(program)
                        .args(["toolchain", "install", channel.as_str()])
                        .interactive(),
                )],
                (None, _) => vec![
                    Action::Run(
                        CommandSpec::new(program.as_str())
                            .args(["toolchain", "install", "stable"])
                            .interactive(),
                    ),
                    // `rustup` is installed without a default toolchain.
                    Action::Run(
                        CommandSpec::new(program)
                            .args(["default", "stable"])
                            .interactive(),
                    ),
                ],
            })
        },
    )
}

/// `1.95.0` matches `1.95.0-x86_64-unknown-linux-gnu`; `stable` matches `stable-…`.
fn toolchain_matches(installed: &str, channel: &str) -> bool {
    installed == channel
        || installed
            .strip_prefix(channel)
            .is_some_and(|rest| rest.starts_with('-'))
}

/// `~/.cargo/bin` on the user PATH.
pub fn cargo_path_step() -> Step {
    Step::new(
        "cargo-path",
        Set::Base,
        "~/.cargo/bin on the user PATH",
        Privilege::None,
        |cx| {
            if cx.machine.which("cargo").is_some() {
                return Probed::ok();
            }
            let Some(dir) = cargo_bin_dir(cx) else {
                return Probed::missing("the home directory is unknown");
            };
            match cx.user_path.add(&dir, true) {
                Ok(PathChange::AlreadyPresent) => Probed::ok(),
                Ok(PathChange::Added) => Probed::missing("not on the user PATH"),
                Err(error) => Probed::missing(format!("cannot read the user PATH: {error}")),
            }
        },
        |cx, _| {
            let dir = cargo_bin_dir(cx).ok_or_else(|| {
                SetupError::Prerequisite("the home directory is unknown".to_string())
            })?;
            Ok(vec![Action::AddToPath(dir)])
        },
    )
}

/// Whether `rustup` has `name` installed (on Windows, for the native triple only).
pub fn toolchain_installed(cx: &mut Cx, name: &str) -> bool {
    let triple = (cx.env.host.os == Os::Windows).then(|| windows::native_triple(cx.env.host.arch));
    let program = rustup_program(cx);
    cx.query(&CommandSpec::new(program).args(["toolchain", "list"]))
        .is_some_and(|list| {
            list.stdout
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .filter(|installed| triple.is_none_or(|triple| installed.ends_with(triple)))
                .any(|installed| toolchain_matches(installed, name))
        })
}
