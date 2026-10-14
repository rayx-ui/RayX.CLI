//! `rayx lima remove <name>` and `rayx wsl remove <name>`: delete a guest `rayx` created, always
//! confirmed. Only guests in `guests.json` can be removed; anything else is the owner's own and
//! is never touched.

use std::io::{IsTerminal, Write};

use crate::cli::RemoveArgs;
use crate::host::{CommandSpec, HostFacts, Os, Outcome, Runner};

use super::guest::{self, Guest, GuestKind};
use super::lima::sizing::{Prompter, TerminalPrompter};
use super::lima::{lima_path_dir, parse_instances};
use super::records;
use super::{Machine, SystemMachine};

/// What stopped a removal.
#[derive(Debug, PartialEq, Eq)]
pub enum RemoveError {
    /// The command belongs to another OS.
    WrongOs(String),
    /// The name is not in `guests.json`.
    NotRecorded(String),
    /// Not confirmed, or nobody could be asked.
    Declined(String),
    Failed(String),
}

impl std::fmt::Display for RemoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoveError::WrongOs(message)
            | RemoveError::NotRecorded(message)
            | RemoveError::Declined(message)
            | RemoveError::Failed(message) => f.write_str(message),
        }
    }
}

/// Runs `rayx lima remove` or `rayx wsl remove` against the real machine; the exit code.
pub fn run(kind: GuestKind, args: &RemoveArgs) -> u8 {
    let host = crate::host::facts();
    let mut runner = Runner::execute();
    let machine = SystemMachine;
    let mut prompter = TerminalPrompter {
        terminal: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
    };
    match remove_guest(
        kind,
        &args.name,
        args.yes,
        &host,
        &mut runner,
        &machine,
        Some(&mut prompter),
        &mut std::io::stdout(),
    ) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("rayx: {error}");
            1
        }
    }
}

fn kind_name(kind: GuestKind) -> &'static str {
    match kind {
        GuestKind::Lima => records::LIMA,
        GuestKind::Wsl => records::WSL,
    }
}

/// Removes a recorded guest after listing the work in it that exists nowhere else and asking.
#[allow(clippy::too_many_arguments)]
pub fn remove_guest(
    kind: GuestKind,
    name: &str,
    yes: bool,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    prompter: Option<&mut dyn Prompter>,
    out: &mut dyn Write,
) -> Result<(), RemoveError> {
    let (label, os, here) = match kind {
        GuestKind::Lima => ("lima", Os::MacOs, "macOS"),
        GuestKind::Wsl => ("wsl", Os::Windows, "Windows"),
    };
    if host.os != os || host.wsl {
        return Err(RemoveError::WrongOs(format!(
            "`rayx {label} remove` removes a {} guest created from {here}; there is none here",
            kind.label()
        )));
    }
    let file = records::read_guests(machine, host.os)
        .map_err(|error| RemoveError::Failed(error.to_string()))?;
    let Some(record) = file.find(kind_name(kind), name).cloned() else {
        return Err(RemoveError::NotRecorded(format!(
            "{name} was not created by rayx; remove it with {} yourself",
            match kind {
                GuestKind::Lima => "limactl delete",
                GuestKind::Wsl => "wsl --unregister",
            }
        )));
    };
    let path_dir = lima_path_dir(machine);
    let guest = match kind {
        GuestKind::Lima => Guest::lima(name).with_path_dir(path_dir.clone()),
        GuestKind::Wsl => Guest::wsl(name),
    };

    // What the host has: a Lima VM is inspected only while it runs (nothing is started for a
    // removal); a registered WSL distribution starts on demand.
    let present = match kind {
        GuestKind::Lima => {
            let spec = CommandSpec::new("limactl").args(["list", "--json"]);
            let spec = match &path_dir {
                Some(dir) => spec.env("PATH", guest::path_with(dir)),
                None => spec,
            };
            let instances = runner
                .query(&spec)
                .ok()
                .filter(Outcome::is_success)
                .map(|outcome| parse_instances(&outcome.stdout))
                .unwrap_or_default();
            instances
                .iter()
                .find(|i| i.name == name)
                .map(|i| i.is_running())
        }
        GuestKind::Wsl => machine
            .wsl_distributions()
            .iter()
            .any(|d| d.name == name)
            .then_some(true),
    };
    match present {
        Some(true) => {
            let dirty = guest::dirty_clones(&guest, runner);
            if !dirty.is_empty() {
                let _ = writeln!(
                    out,
                    "These clones in {name} hold changes that exist nowhere else (uncommitted work \
                     or commits on no remote):"
                );
                for dir in &dirty {
                    let _ = writeln!(out, "  {dir}");
                }
            }
        }
        Some(false) => {
            let _ = writeln!(
                out,
                "{name} is stopped, so its clones are not inspected for unpushed work"
            );
        }
        None => {
            let _ = writeln!(
                out,
                "{label} does not have {name} any more: only its record is dropped"
            );
        }
    }

    if !yes {
        match prompter {
            None => {}
            Some(prompter) => {
                if !prompter.interactive() {
                    return Err(RemoveError::Declined(format!(
                        "{name} was not removed: nobody can be asked, run `rayx {label} remove \
                         {name} --yes`"
                    )));
                }
                if !prompter.confirm(&format!("Remove {name} and everything in it?")) {
                    return Err(RemoveError::Declined(format!("{name} was not removed")));
                }
            }
        }
    }

    if present.is_some() {
        let command = match kind {
            GuestKind::Lima => {
                let spec = CommandSpec::new("limactl")
                    .args(["delete", "--force", name])
                    .interactive();
                match &path_dir {
                    Some(dir) => spec.env("PATH", guest::path_with(dir)),
                    None => spec,
                }
            }
            GuestKind::Wsl => CommandSpec::new("wsl")
                .args(["--unregister", name])
                .interactive(),
        };
        runner
            .run_checked(&command)
            .map_err(|error| RemoveError::Failed(format!("removing {name}: {error}")))?;
    }
    let owned = name.to_string();
    records::update_guests(machine, host.os, |file| {
        file.remove(kind_name(kind), &owned);
    })
    .map_err(|error| RemoveError::Failed(error.to_string()))?;
    let _ = writeln!(out, "Removed {} guest {name}.", kind.label());
    if !record.retired {
        let _ = writeln!(
            out,
            "It was the current guest: `rayx setup --{label}` creates it again."
        );
    }
    Ok(())
}
