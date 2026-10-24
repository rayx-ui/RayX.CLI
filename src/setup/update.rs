//! `rayx setup --update`: re-run setup for what this machine was set up with (`setup.json`), once
//! per recorded project root so each project's pins apply, and then refresh the guests it drives.
//! This module is the host part; it never runs `apt-get upgrade` on the host.

use std::io::{self, IsTerminal, Write};

use crate::cli::SetupArgs;
use crate::host::path_env::UserPath;
use crate::host::{HostFacts, Os, Runner};
use crate::project::Project;

use super::guest;
use super::lima::sizing::{Prompter, TerminalPrompter};
use super::records::{self, SetupFile};
use super::refresh::{RefreshOptions, check_targets, refresh_probed};
use super::wsl;
use super::{Machine, Set, SystemMachine, ask_on_terminal, environment_for, run_sets};

/// What the host part of an update found and did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostUpdate {
    /// 0, or 1 when a project run failed or an update was declined.
    pub code: u8,
    /// Whether `setup.json` held anything to re-run.
    pub recorded: bool,
    /// `setup.json` exists but could not be used: something is recorded, `rayx setup` cannot fix it.
    pub unreadable: bool,
}

/// The recorded sets that apply to `host`, base first.
pub fn recorded_sets(setup: &SetupFile, host: &HostFacts) -> Vec<Set> {
    let mut sets = vec![Set::Base];
    for name in &setup.sets {
        if let Some(set) = Set::from_name(name)
            && set.applies_to(host)
            && !sets.contains(&set)
        {
            sets.push(set);
        }
    }
    sets
}

/// Re-runs the recorded setup, once per project root that still exists (in that root), or once
/// without a project when no root is recorded. A vanished root is reported and dropped.
pub fn update_host(
    args: &SetupArgs,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    user_path: &mut UserPath,
    out: &mut dyn Write,
    mut confirm: Option<&mut dyn FnMut(&str) -> bool>,
) -> HostUpdate {
    let mut setup = match records::read_setup(machine, host.os) {
        Ok(setup) => setup,
        Err(error) => {
            eprintln!("rayx: {error}");
            return HostUpdate {
                code: 1,
                recorded: false,
                unreadable: true,
            };
        }
    };
    if setup.is_empty() {
        return HostUpdate {
            code: 0,
            recorded: false,
            unreadable: false,
        };
    }
    let sets = recorded_sets(&setup, host);
    let run_args = SetupArgs {
        check: args.check,
        yes: args.yes,
        ..SetupArgs::default()
    };
    let mut roots = Vec::new();
    for root in setup.roots.clone() {
        if machine.is_dir(std::path::Path::new(&root)) {
            roots.push(Some(root));
            continue;
        }
        eprintln!(
            "rayx: the recorded project {root} no longer exists; it is dropped from the record"
        );
        setup.forget_root(&root);
        if !args.check {
            let _ = records::write_setup(machine, host.os, &setup);
        }
    }
    if roots.is_empty() {
        roots.push(None);
    }
    let mut code = 0;
    for root in roots {
        let project = root
            .as_ref()
            .and_then(|root| Project::discover(runner, std::path::Path::new(root), None).ok());
        let _ = writeln!(
            out,
            "==> setup --update: {}",
            root.as_deref().unwrap_or("no project")
        );
        // The pin is read in the recorded directory (an app without a Cargo workspace is recorded
        // by its own), through the run's runner, so `--check` writes no lock.
        let dir = root.as_ref().map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            std::path::PathBuf::from,
        );
        let env = environment_for(host.clone(), project.as_ref(), &dir, args.yes, runner);
        let mut ask = |question: &str| confirm.as_mut().is_none_or(|ask| ask(question));
        let result = run_sets(
            &run_args,
            &sets,
            &env,
            runner,
            machine,
            user_path,
            out,
            Some(&mut ask),
        );
        code = code.max(result);
    }
    HostUpdate {
        code,
        recorded: true,
        unreadable: false,
    }
}

/// `RAYX_WSL_BINARY` as the distribution reads it. One that is not on a Windows drive cannot be
/// reached from WSL: it is reported (through `warn`) instead of silently replaced by the release
/// installer.
pub fn wsl_binary_for_refresh(
    path: Option<std::path::PathBuf>,
    warn: &mut dyn FnMut(String),
) -> Option<String> {
    let path = path?;
    match wsl::to_wsl_path(&path) {
        Ok(linux) => Some(linux),
        Err(error) => {
            warn(format!(
                "RAYX_WSL_BINARY is ignored and the release installer is used: {error}"
            ));
            None
        }
    }
}

/// Runs `rayx setup --update` against the real machine and returns the process exit code.
pub fn run(args: &SetupArgs) -> u8 {
    let host = crate::host::facts();
    let release = match guest::chosen_release(args) {
        Ok(release) => release,
        Err(error) => {
            eprintln!("rayx: {error}");
            return 1;
        }
    };
    let mut runner = if args.check {
        Runner::print()
    } else {
        Runner::execute()
    };
    let mut user_path = match UserPath::system(host.os) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("rayx: cannot edit the user PATH: {error}");
            return 1;
        }
    };
    let machine = SystemMachine;
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut confirm = move |question: &str| interactive && ask_on_terminal(question);
    let options = RefreshOptions {
        version: guest::guest_version(release.as_ref()),
        release,
        targets: args.vm.clone(),
        yes: args.yes,
        check: args.check,
        lima_binary: std::env::var("RAYX_LIMA_BINARY")
            .ok()
            .filter(|path| !path.trim().is_empty()),
        wsl_binary: wsl_binary_for_refresh(wsl::binary_override_from_env(), &mut |warning| {
            eprintln!("rayx: {warning}");
        }),
    };
    let mut prompter = TerminalPrompter {
        terminal: interactive,
    };
    let code = run_with(
        args,
        &options,
        &host,
        &mut runner,
        &machine,
        &mut user_path,
        Some(&mut confirm),
        Some(&mut prompter),
        &mut io::stdout(),
    );
    let mut ask = ask_on_terminal;
    after_update(
        args,
        &mut Runner::print(),
        &machine,
        &host,
        interactive.then_some(&mut ask as &mut dyn FnMut(&str) -> bool),
        &mut io::stdout(),
    );
    code
}

/// After `rayx setup --update`: the recorded projects' pins are installed now, so it offers to
/// remove the store entries none of them uses (STORE-2). `--check`, and a run that only targets
/// guests (`--vm`), leave the store alone.
pub fn after_update(
    args: &SetupArgs,
    runner: &mut Runner,
    machine: &dyn Machine,
    host: &HostFacts,
    confirm: Option<&mut dyn FnMut(&str) -> bool>,
    out: &mut dyn Write,
) {
    if args.check || !args.vm.is_empty() {
        return;
    }
    crate::nsh::offer_prune(runner, machine, host, args.yes, confirm, out);
}

/// The body of `rayx setup --update` over injectable collaborators: a chosen release is looked
/// for first (nothing has changed if it is missing), then the host part runs, then the guests
/// are refreshed. Returns the exit code.
#[allow(clippy::too_many_arguments)]
pub fn run_with(
    args: &SetupArgs,
    options: &RefreshOptions,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    user_path: &mut UserPath,
    confirm: Option<&mut dyn FnMut(&str) -> bool>,
    prompter: Option<&mut dyn Prompter>,
    out: &mut dyn Write,
) -> u8 {
    // Names are checked before the first command, the release probe included.
    if let Err(error) = check_targets(options, host, machine) {
        eprintln!("rayx: {error}");
        return 1;
    }
    // Only a host that drives guests has anything to give the release to; asking about it
    // elsewhere would make an offline Linux box fail an update it can do.
    if host.os != Os::Linux && !host.wsl {
        let override_name = match host.os {
            Os::MacOs => options.lima_binary.is_some().then_some("RAYX_LIMA_BINARY"),
            _ => options.wsl_binary.is_some().then_some("RAYX_WSL_BINARY"),
        };
        if let Err(error) = guest::plan_release(options.release.as_ref(), override_name, runner) {
            eprintln!("rayx: {error}");
            return 1;
        }
    }
    // Naming guests is an explicit target, not a host run: the host part is skipped.
    let update = if options.targets.is_empty() {
        update_host(args, host, runner, machine, user_path, out, confirm)
    } else {
        let _ = writeln!(out, "the host part is skipped: --vm names guests only");
        HostUpdate {
            code: 0,
            recorded: true,
            unreadable: false,
        }
    };
    let refreshed = refresh_probed(options, host, runner, machine, prompter, out);
    if !update.recorded
        && !update.unreadable
        && !refreshed.unreadable
        && refreshed.refused.is_none()
        && refreshed.guests.is_empty()
    {
        let _ = writeln!(out, "nothing recorded: run `rayx setup` first");
    }
    update.code.max(refreshed.code())
}
