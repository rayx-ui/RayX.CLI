//! The web set: the wasm32 target on the project's toolchain, `wasm-bindgen-cli` at the version
//! the project's `Cargo.lock` pins (only when it locks one), and a clang that can target wasm32.
//! Clang and LLVM already come with the base set on Linux (apt) and Windows (winget); macOS's Apple
//! clang cannot target wasm32, so the set adds Homebrew's LLVM there.
//!
//! NSH's web runtime is single-threaded, so the set installs no nightly and no `rust-src` (D3).

use crate::host::{CommandSpec, Os, Privilege};

use super::{Action, Cx, PlanEnv, Probed, Set, SetupError, Step, macos, rust};

const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// Whether `rustup <subcommand...>` for `toolchain` lists `needle` among its output lines.
fn rustup_lists(cx: &mut Cx, args: &[&str], toolchain: &str, needle: &str) -> Option<bool> {
    let program = rust::rustup_program(cx);
    let spec = CommandSpec::new(program)
        .args(args.iter().copied())
        .args(["--toolchain", toolchain, "--installed"])
        .env("RUSTUP_AUTO_INSTALL", "0");
    let outcome = cx.query(&spec)?;
    outcome.is_success().then(|| {
        outcome
            .stdout
            .lines()
            .any(|line| line.split_whitespace().next() == Some(needle))
    })
}

fn wasm_target_step(env: &PlanEnv) -> Step {
    let toolchain = env
        .pins
        .rust_toolchain
        .as_ref()
        .map_or_else(|| "stable".to_string(), |pin| pin.value.clone());
    let probe_toolchain = toolchain.clone();
    Step::new(
        "web-wasm-target",
        Set::Web,
        format!("the {WASM_TARGET} target on {toolchain}"),
        Privilege::None,
        move |cx| match rustup_lists(cx, &["target", "list"], &probe_toolchain, WASM_TARGET) {
            Some(true) => Probed::ok(),
            Some(false) => Probed::missing(format!("the {WASM_TARGET} target is missing")),
            None => Probed::missing(format!("{probe_toolchain} is not installed")),
        },
        move |cx, _| {
            let program = rust::rustup_program(cx);
            Ok(vec![Action::Run(
                CommandSpec::new(program)
                    .args([
                        "target",
                        "add",
                        WASM_TARGET,
                        "--toolchain",
                        toolchain.as_str(),
                    ])
                    .interactive(),
            )])
        },
    )
}

/// `wasm-bindgen-cli` at `version`: nsh runs it on an app's module when the app's crates use
/// `wasm-bindgen`, and its version must equal the crate's.
fn wasm_bindgen_step(version: String) -> Step {
    let probe_version = version.clone();
    Step::new(
        "wasm-bindgen",
        Set::Web,
        format!("wasm-bindgen-cli {version} (the project's Cargo.lock)"),
        Privilege::None,
        move |cx| {
            let program = rust::tool_program(cx, "wasm-bindgen");
            let installed = cx
                .query(&CommandSpec::new(program).arg("--version"))
                .filter(|outcome| outcome.is_success())
                .and_then(|outcome| outcome.stdout.split_whitespace().nth(1).map(str::to_string));
            match installed {
                Some(found) if found == probe_version => Probed::ok().with_found(found),
                Some(found) => Probed::missing(format!(
                    "version {found} is installed, the project pins {probe_version}"
                ))
                .with_found(found),
                None => Probed::missing("wasm-bindgen is not installed"),
            }
        },
        move |cx, _| {
            let cargo = rust::tool_program(cx, "cargo");
            Ok(vec![Action::Run(
                // `--force` replaces another installed version.
                CommandSpec::new(cargo)
                    .args(["install", "wasm-bindgen-cli", "--version", version.as_str()])
                    .args(["--force", "--locked"])
                    .interactive(),
            )])
        },
    )
}

/// Homebrew's LLVM: Apple's clang has no wasm32 backend.
fn macos_llvm_step() -> Step {
    Step::new(
        "web-llvm",
        Set::Web,
        "LLVM with the wasm32 backend (Homebrew)",
        Privilege::None,
        |cx| {
            let Some(brew) = macos::brew_program(cx) else {
                return Probed::missing("Homebrew is not installed");
            };
            let listed = cx
                .query(&CommandSpec::new(brew.display().to_string()).args([
                    "list",
                    "--versions",
                    "llvm",
                ]))
                .is_some_and(|outcome| outcome.is_success() && !outcome.stdout.trim().is_empty());
            if listed {
                Probed::ok()
            } else {
                Probed::missing("llvm is not installed")
            }
        },
        |cx, _| {
            let brew = macos::brew_program(cx)
                .map_or_else(|| "brew".to_string(), |path| path.display().to_string());
            Ok(vec![Action::Run(
                CommandSpec::new(brew)
                    .args(["install", "llvm"])
                    .interactive(),
            )])
        },
    )
}

pub fn steps(env: &PlanEnv) -> Result<Vec<Step>, SetupError> {
    let mut steps = vec![wasm_target_step(env)];
    // A project whose lock has no `wasm-bindgen` needs no CLI for it.
    if let Some(pin) = &env.pins.wasm_bindgen {
        steps.push(wasm_bindgen_step(pin.value.clone()));
    }
    if env.host.os == Os::MacOs {
        steps.push(macos_llvm_step());
    }
    Ok(steps)
}
