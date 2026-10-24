//! The base set's last step: the nsh the project pins, in rayx's store (design §4.5, NSHCMD-2).
//!
//! Its probe is the entry's receipt plus `nsh --version`; its install is `nsh::install`. It asks
//! nothing, so `--yes` does not change it, and `--check` prints what it would run.

use std::rc::Rc;

use crate::host::Privilege;
use crate::nsh::install::{Install, RustTools, archive_name, archive_url, check_cli, platform};
use crate::nsh::pin::{Pin, Source, Unreadable, resolve_with};
use crate::nsh::store::Store;

use super::{Action, Cx, Probed, Set, SetupError, Step, rust};

/// The step for `pin`.
pub fn step(pin: &Pin) -> Step {
    let probe_pin = pin.clone();
    let install_pin = pin.clone();
    Step::new(
        "nsh",
        Set::Base,
        format!("nsh {} ({})", pin.label(), pin.origin),
        Privilege::None,
        move |cx| probe(cx, &probe_pin),
        move |cx, _| actions(cx, &install_pin),
    )
}

/// The install of `pin`: one custom action, or none for a path checkout.
fn actions(cx: &Cx, pin: &Pin) -> Result<Vec<Action>, SetupError> {
    if let Source::Path { checkout } = &pin.source {
        // A path checkout is built in place by `rayx app` (RUN-3); nothing to install.
        return Err(SetupError::Prerequisite(format!(
            "the NSH checkout {} is built by `rayx app` when it runs",
            checkout.display()
        )));
    }
    let description = describe(cx, pin);
    let pin = pin.clone();
    Ok(vec![Action::Custom {
        description,
        run: Rc::new(move |cx| install(cx, &pin)),
    }])
}

/// The step for a pin that could not be read when the plan was made (two sources, a broken
/// `rayx.toml`, no `Cargo.lock` yet). Its probe and its install read the pin again in the same
/// directory: on a fresh machine the steps before it have installed Rust by then, so the
/// project's own pin is installed in the same run. While it still cannot be read, the step is
/// missing and its install fails with the reason. Nothing stands in for the pin, so a run never
/// installs an nsh the project does not name.
pub fn unreadable(unreadable: &Unreadable) -> Step {
    let probe_dir = unreadable.dir.clone();
    let install_dir = unreadable.dir.clone();
    Step::new(
        "nsh",
        Set::Base,
        "nsh (the project's pin cannot be read)",
        Privilege::None,
        move |cx| match read_again(cx, &probe_dir) {
            Ok(pin) => probe(cx, &pin),
            Err(error) => Probed::missing(format!("{error:#}")),
        },
        move |cx, _| match read_again(cx, &install_dir) {
            Ok(pin) => actions(cx, &pin),
            Err(error) => Err(SetupError::Prerequisite(format!(
                "the nsh pin cannot be read: {error:#}"
            ))),
        },
    )
}

/// The pin of `dir` read again, with the cargo the steps before installed (`~/.cargo/bin`, which
/// this process's `PATH` may not have yet).
fn read_again(cx: &mut Cx, dir: &std::path::Path) -> anyhow::Result<Pin> {
    let cargo = rust::tool_program(cx, "cargo");
    resolve_with(cx.runner, dir, &cargo)
}

/// Present when the store has the pin's entry and its CLI answers as the pin wants.
fn probe(cx: &mut Cx, pin: &Pin) -> Probed {
    if let Source::Path { checkout } = &pin.source {
        return Probed::ok().with_found(format!("checkout {}", checkout.display()));
    }
    let Some(store) = Store::locate(cx.machine) else {
        return Probed::missing("no home directory for the nsh store");
    };
    let Some(id) = pin.id() else {
        return Probed::missing("the pin has no store id");
    };
    let Some(receipt) = store.receipt(&id) else {
        return Probed::missing(if store.dir(&id).is_dir() {
            format!("the store entry {id} is unfinished")
        } else {
            format!("{id} is not in {}", store.root.display())
        });
    };
    let cli = store.cli(&receipt);
    match check_cli(cx.runner, &cli, pin) {
        Ok(version) => Probed::ok().with_found(format!(
            "{} {}{}",
            receipt.kind.as_str(),
            version.version,
            version
                .commit
                .map_or(String::new(), |commit| format!(" ({commit})"))
        )),
        Err(error) => Probed::missing(format!("{error:#}")),
    }
}

/// What `--check` shows for the install: the archive it downloads, else the source build.
fn describe(cx: &Cx, pin: &Pin) -> String {
    let store = Store::locate(cx.machine).map_or_else(
        || "the nsh store".to_string(),
        |s| s.root.display().to_string(),
    );
    let id = pin.id().unwrap_or_default();
    let source = format!(
        "git fetch --depth 1 {} {}, rustup toolchain install, cargo build --release --locked -p nsh-cli --target-dir target (without CARGO and RUSTUP_TOOLCHAIN) in {store}/{id}/source",
        pin.git_url().unwrap_or_default(),
        pin.git_ref().unwrap_or_default()
    );
    match (pin.release(), platform(&cx.env.host)) {
        (Some(release), Some(platform)) => format!(
            "download {} (and its .sha256) into {store}/{id}, else {source}",
            archive_url(&release, &archive_name(platform))
        ),
        _ => source,
    }
}

/// Installs the pin into the store, progress on standard output.
fn install(cx: &mut Cx, pin: &Pin) -> Result<(), SetupError> {
    let store = Store::locate(cx.machine)
        .ok_or_else(|| SetupError::Prerequisite("no home directory for the nsh store".into()))?;
    let host = cx.env.host.clone();
    let tools = RustTools::find(cx.machine, host.os);
    let mut out = std::io::stdout();
    Install {
        runner: cx.runner,
        store: &store,
        host: &host,
        tools: &tools,
        out: &mut out,
    }
    .run(pin)
    .map(|_| ())
    .map_err(|error| SetupError::Prerequisite(format!("installing nsh {}: {error:#}", pin.label())))
}
