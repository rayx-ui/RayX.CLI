//! The nsh a project pins: reading the pin, the store under `$RAYX_HOME/nsh`, installing from a
//! release or from source, and `rayx nsh status|path|install|prune` (design §4).
//!
//! rayx never runs the `nsh` on `PATH` (D1): it runs the build of the project's pin from the
//! store, in the directory it read the pin in.

pub mod install;
pub mod pin;
pub mod store;

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use serde_json::json;

use crate::cli::NshCommand;
use crate::host::{HostFacts, Runner};
use crate::setup::records::read_setup;
use crate::setup::{Machine, SystemMachine, ask_on_terminal};

use install::{Install, RustTools};
use pin::{Origin, Pin, Source};
use store::{Kind, Store, human};

/// Where a pin stands in the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub pin: Pin,
    /// `None` for a path checkout, which is not stored.
    pub id: Option<String>,
    /// The entry's kind, once complete.
    pub kind: Option<Kind>,
    pub installed: bool,
    pub bytes: Option<u64>,
    /// The CLI: the entry's, or a path checkout's `target/release/nsh`.
    pub cli: Option<PathBuf>,
}

impl Status {
    /// The status of `pin` in `store`.
    pub fn of(pin: &Pin, store: &Store, host: &HostFacts) -> Status {
        if let Source::Path { checkout } = &pin.source {
            let cli = checkout.join("target").join("release").join(
                if host.os == crate::host::Os::Windows {
                    "nsh.exe"
                } else {
                    "nsh"
                },
            );
            return Status {
                pin: pin.clone(),
                id: None,
                kind: None,
                installed: cli.is_file(),
                bytes: None,
                cli: Some(cli),
            };
        }
        let id = pin.id();
        let receipt = id.as_deref().and_then(|id| store.receipt(id));
        let bytes = id
            .as_deref()
            .map(|id| store.dir(id))
            .filter(|dir| dir.is_dir())
            .map(|dir| store::size_of(&dir));
        Status {
            pin: pin.clone(),
            kind: receipt.as_ref().map(|r| r.kind),
            installed: receipt.is_some(),
            cli: receipt.as_ref().map(|r| store.cli(r)),
            bytes,
            id,
        }
    }

    /// `installed`, `unfinished` or `missing`.
    pub fn state(&self) -> &'static str {
        if self.installed {
            "installed"
        } else if self.bytes.is_some() {
            "unfinished"
        } else {
            "missing"
        }
    }

    /// One line, as doctor prints it: `nsh  fork-0.2.0-4 (327bb9e)  project  installed  3.1 GiB`.
    pub fn line(&self) -> String {
        let mut line = format!(
            "nsh  {}  {}  {}",
            self.pin.label(),
            self.pin.origin,
            self.state()
        );
        if let Some(bytes) = self.bytes {
            line.push_str(&format!("  {}", human(bytes)));
        }
        line
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "value": self.pin.label(),
            "source": self.pin.origin.as_str(),
            "id": self.id,
            "kind": self.kind.map(Kind::as_str),
            "state": self.state(),
            "installed": self.installed,
            "bytes": self.bytes,
            "cli": self.cli.as_ref().map(|p| p.display().to_string()),
            "commit": self.pin.commit(),
            "version": self.pin.version,
        })
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("nsh {}\n", self.pin.label()));
        out.push_str(&format!("  source     {}\n", self.pin.origin));
        if let Some(id) = &self.id {
            out.push_str(&format!("  id         {id}\n"));
        }
        out.push_str(&format!(
            "  state      {}{}\n",
            self.state(),
            self.kind
                .map_or(String::new(), |kind| format!(" ({})", kind.as_str()))
        ));
        if let Some(bytes) = self.bytes {
            out.push_str(&format!("  size       {}\n", human(bytes)));
        }
        if let Some(cli) = &self.cli {
            out.push_str(&format!("  cli        {}\n", cli.display()));
        }
        out
    }
}

/// The `rayx setup` command that installs the pin read in `dir`.
pub fn setup_command(dir: &Path) -> String {
    format!("cd {} && rayx setup", dir.display())
}

/// Runs `rayx nsh <command>` against the real machine and returns the exit code.
pub fn run(command: &NshCommand) -> u8 {
    let host = crate::host::facts();
    let machine = SystemMachine;
    let Some(store) = Store::locate(&machine) else {
        eprintln!("rayx: no home directory for the nsh store (set RAYX_HOME)");
        return 1;
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let dir = |given: &Option<PathBuf>| given.as_ref().map_or_else(|| cwd.clone(), |d| cwd.join(d));
    let mut runner = Runner::execute();
    let tools = RustTools::find(&machine, host.os);
    let mut out = io::stdout();
    let result = match command {
        // A report: it reads the pin and the entry and writes nothing, not even a missing lock.
        NshCommand::Status { dir: d, json } => status_in(
            &mut Runner::print(),
            &store,
            &host,
            &dir(d),
            *json,
            &mut out,
        ),
        NshCommand::Path { dir: d, no_install } => path_in(
            &mut runner,
            &store,
            &host,
            &tools,
            &dir(d),
            *no_install,
            &mut out,
            &mut io::stderr(),
        ),
        NshCommand::Install { dir: d } => {
            install_in(&mut runner, &store, &host, &tools, &dir(d), &mut out)
        }
        NshCommand::Prune { yes } => {
            let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
            let mut confirm = ask_on_terminal;
            // Reading the recorded projects' pins writes nothing into them, not even a lock.
            kept_ids(&mut Runner::print(), &machine, &host).map(|keep| {
                store::prune(
                    &store,
                    &keep,
                    *yes,
                    interactive.then_some(&mut confirm as &mut dyn FnMut(&str) -> bool),
                    &mut out,
                )
            })
        }
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("rayx: {error:#}");
            1
        }
    }
}

/// Resolves the pin and prints the warning an app without a pin of its own gets.
pub fn resolve(runner: &mut Runner, dir: &Path, err: &mut dyn Write) -> Result<Pin> {
    let pin = pin::resolve(runner, dir)?;
    if let Some(warning) = pin.default_warning() {
        let _ = writeln!(err, "{warning}");
    }
    Ok(pin)
}

/// `rayx nsh status [--json]` in `dir`.
pub fn status_in(
    runner: &mut Runner,
    store: &Store,
    host: &HostFacts,
    dir: &Path,
    json: bool,
    out: &mut dyn Write,
) -> Result<u8> {
    let pin = resolve(runner, dir, &mut io::stderr())?;
    let status = Status::of(&pin, store, host);
    let text = if json {
        serde_json::to_string_pretty(&status.to_json())? + "\n"
    } else {
        status.to_text()
    };
    let _ = out.write_all(text.as_bytes());
    Ok(0)
}

/// `rayx nsh path [--no-install]` in `dir`: the CLI's path on `out`, installing it first.
#[allow(clippy::too_many_arguments)]
pub fn path_in(
    runner: &mut Runner,
    store: &Store,
    host: &HostFacts,
    tools: &RustTools,
    dir: &Path,
    no_install: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<u8> {
    let pin = resolve(runner, dir, err)?;
    let status = Status::of(&pin, store, host);
    if status.installed {
        let _ = writeln!(
            out,
            "{}",
            status.cli.expect("an installed pin has a CLI").display()
        );
        return Ok(0);
    }
    if let Source::Path { checkout } = &pin.source {
        let _ = writeln!(
            err,
            "rayx: the NSH checkout {} has no built CLI yet; `rayx app` builds it",
            checkout.display()
        );
        return Ok(1);
    }
    if no_install {
        let _ = writeln!(
            err,
            "rayx: nsh {} is not installed; install it with `{}`",
            pin.label(),
            setup_command(&pin.dir)
        );
        return Ok(1);
    }
    let installed = Install {
        runner,
        store,
        host,
        tools,
        out: err,
    }
    .run(&pin)?;
    let _ = writeln!(out, "{}", installed.cli.display());
    Ok(0)
}

/// `rayx nsh install` in `dir`.
pub fn install_in(
    runner: &mut Runner,
    store: &Store,
    host: &HostFacts,
    tools: &RustTools,
    dir: &Path,
    out: &mut dyn Write,
) -> Result<u8> {
    let pin = resolve(runner, dir, &mut io::stderr())?;
    if matches!(pin.source, Source::Path { .. }) {
        return Err(anyhow!(
            "{} uses an NSH checkout by path; `rayx app` builds its CLI in place",
            dir.display()
        ));
    }
    let installed = Install {
        runner,
        store,
        host,
        tools,
        out,
    }
    .run(&pin)?;
    if installed.already {
        let _ = writeln!(
            out,
            "nsh {} is already installed in {}.",
            pin.label(),
            store.dir(&installed.receipt.id).display()
        );
    }
    Ok(0)
}

/// The store ids prune keeps: the default pin's and every recorded project's pin. A recorded
/// project whose pin cannot be read is an error: which entry it uses is unknown, so nothing may
/// be pruned. A recorded directory that no longer exists keeps nothing.
pub fn kept_ids(
    runner: &mut Runner,
    machine: &dyn Machine,
    host: &HostFacts,
) -> Result<Vec<String>> {
    let mut keep: Vec<String> = pin::default_pin(Path::new(".")).id().into_iter().collect();
    if let Ok(setup) = read_setup(machine, host.os) {
        for root in setup.roots {
            let root = PathBuf::from(root);
            if !root.is_dir() {
                continue;
            }
            let pin = pin::resolve(runner, &root).map_err(|error| {
                anyhow!(
                    "cannot tell which nsh {} uses ({error:#}), so nothing is pruned",
                    root.display()
                )
            })?;
            if let Some(id) = pin.id()
                && !keep.contains(&id)
            {
                keep.push(id);
            }
        }
    }
    Ok(keep)
}

/// Whether a pin is the built-in default.
pub fn is_default(pin: &Pin) -> bool {
    pin.origin == Origin::Default
}

/// What `rayx setup --update` does after its runs: lists the store entries no recorded project
/// and no default pin uses and removes them after `confirm` says yes (or with `yes`). It says
/// nothing when there is nothing to prune, says why when a recorded pin cannot be read, and
/// never changes the update's exit code.
pub fn offer_prune(
    runner: &mut Runner,
    machine: &dyn Machine,
    host: &HostFacts,
    yes: bool,
    confirm: Option<&mut dyn FnMut(&str) -> bool>,
    out: &mut dyn Write,
) {
    let Some(store) = Store::locate(machine) else {
        return;
    };
    let keep = match kept_ids(runner, machine, host) {
        Ok(keep) => keep,
        Err(error) => {
            let _ = writeln!(out, "rayx: {error:#}");
            return;
        }
    };
    if store::prune_candidates(&store, &keep).is_empty() {
        return;
    }
    let _ = store::prune(&store, &keep, yes, confirm, out);
}
