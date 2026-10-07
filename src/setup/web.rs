//! The web set: the pinned nightly with `rust-src` and the wasm32 target, the wasm32 target on the
//! project's toolchain, `wasm-bindgen-cli` at the pinned version, and a clang that can target
//! wasm32. Clang and LLVM already come with the base set on Linux (apt) and Windows (winget);
//! macOS's Apple clang cannot target wasm32, so the set adds Homebrew's LLVM there.

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

fn nightly_step(env: &PlanEnv) -> Step {
    let nightly = env.pins.web_toolchain.value.clone();
    let probe_nightly = nightly.clone();
    Step::new(
        "web-nightly",
        Set::Web,
        format!("{nightly} with rust-src and the {WASM_TARGET} target"),
        Privilege::None,
        move |cx| {
            if !rust::toolchain_installed(cx, &probe_nightly) {
                return Probed::missing(format!("{probe_nightly} is not installed"));
            }
            let component = rustup_lists(cx, &["component", "list"], &probe_nightly, "rust-src");
            let target = rustup_lists(cx, &["target", "list"], &probe_nightly, WASM_TARGET);
            match (component, target) {
                (Some(true), Some(true)) => Probed::ok(),
                (Some(true), _) => Probed::missing(format!("the {WASM_TARGET} target is missing")),
                _ => Probed::missing("the rust-src component is missing"),
            }
        },
        move |cx, _| {
            let program = rust::rustup_program(cx);
            Ok(vec![Action::Run(
                // Also adds the component and target to a toolchain that is already installed.
                CommandSpec::new(program)
                    .args(["toolchain", "install", nightly.as_str()])
                    .args(["--profile", "minimal"])
                    .args(["--component", "rust-src", "--target", WASM_TARGET])
                    .interactive(),
            )])
        },
    )
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

fn wasm_bindgen_step(env: &PlanEnv) -> Step {
    let pinned = env.pins.wasm_bindgen.as_ref().map(|pin| pin.value.clone());
    let probe_pinned = pinned.clone();
    let title = match &pinned {
        Some(version) => format!("wasm-bindgen-cli {version} (the project's Cargo.lock)"),
        None => "wasm-bindgen-cli".to_string(),
    };
    Step::new(
        "wasm-bindgen",
        Set::Web,
        title,
        Privilege::None,
        move |cx| {
            let program = rust::tool_program(cx, "wasm-bindgen");
            let installed = cx
                .query(&CommandSpec::new(program).arg("--version"))
                .filter(|outcome| outcome.is_success())
                .and_then(|outcome| outcome.stdout.split_whitespace().nth(1).map(str::to_string));
            match (installed, &probe_pinned) {
                (Some(found), Some(want)) if &found == want => Probed::ok().with_found(found),
                (Some(found), Some(want)) => Probed::missing(format!(
                    "version {found} is installed, the project pins {want}"
                ))
                .with_found(found),
                (Some(found), None) => Probed::ok().with_found(found),
                (None, _) => Probed::missing("wasm-bindgen is not installed"),
            }
        },
        move |cx, _| {
            let cargo = rust::tool_program(cx, "cargo");
            let mut install = CommandSpec::new(cargo).args(["install", "wasm-bindgen-cli"]);
            if let Some(version) = &pinned {
                // `--force` replaces another installed version.
                install = install.args(["--version", version.as_str(), "--force"]);
            }
            Ok(vec![Action::Run(install.arg("--locked").interactive())])
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
    let mut steps = vec![
        nightly_step(env),
        wasm_target_step(env),
        wasm_bindgen_step(env),
    ];
    if env.host.os == Os::MacOs {
        steps.push(macos_llvm_step());
    }
    Ok(steps)
}
