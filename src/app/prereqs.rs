//! What an app command needs on the machine before it builds. The target decides which `setup`
//! sets must be complete; what is missing is installed here when that needs no privileges and no
//! license answer, and otherwise the user is asked (in a terminal) or told the exact
//! `rayx setup` command.

use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::{Result, anyhow, bail};

use crate::app::ProjectContext;
use crate::host::Privilege;
use crate::host::Runner;
use crate::host::path_env::UserPath;
use crate::setup::{self, Cx, Machine, Plan, PlanEnv, Set, Step, SystemMachine};

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

/// The `setup` sets an app action on a target needs besides the base set (which every command
/// needs). `assets` and unknown targets need nothing more.
pub fn required_sets(action: &str, target: &str) -> Vec<Set> {
    match (action, target) {
        ("assets", _) => Vec::new(),
        ("test", "wasm") => vec![Set::Web, Set::Test],
        (_, "wasm") => vec![Set::Web],
        (_, "android") => vec![Set::Android],
        (_, "ios") => vec![Set::Ios],
        _ => Vec::new(),
    }
}

/// The `rayx setup` command line that installs `sets`.
pub fn setup_command(sets: &[Set]) -> String {
    let mut command = String::from("rayx setup");
    for set in sets.iter().filter(|set| **set != Set::Base) {
        command.push_str(" --");
        command.push_str(set.name());
    }
    command
}

/// Whether the owner has to be involved to install a step: it needs administrator rights, or it
/// asks a question.
fn needs_owner(step: &Step) -> bool {
    step.privilege != Privilege::None || step.prompts
}

/// Plans the sets (and the base set) and installs what is missing, as [`ensure_plan`] does.
#[allow(clippy::too_many_arguments)]
pub fn ensure(
    env: &PlanEnv,
    sets: &[Set],
    options: &Options,
    runner: &mut Runner,
    machine: &dyn Machine,
    user_path: &mut UserPath,
    out: &mut dyn Write,
    confirm: &mut dyn FnMut(&str) -> bool,
) -> Result<usize> {
    let plan = setup::plan(env, sets).map_err(|error| anyhow!("{error}"))?;
    let mut cx = Cx::new(env, runner, machine, user_path);
    ensure_plan(&plan, &setup_command(sets), options, &mut cx, out, confirm)
}

/// Checks a plan and installs what is missing; `command` is the `rayx setup` command line that
/// installs the same plan by hand. Returns how many steps installed something.
///
/// - Everything present: nothing happens.
/// - `no_install`: the missing items are listed and the command stops.
/// - Only unprivileged, silent items missing: a one-line notice, then they are installed.
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
    let missing: Vec<&Step> = plan
        .steps
        .iter()
        .zip(&checked.probed)
        .filter(|(_, probed)| !probed.satisfied)
        .map(|(step, _)| step)
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }
    let titles = missing
        .iter()
        .map(|step| step.title.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    if options.no_install {
        bail!("missing: {titles}. Run `{command}` to install it (--no-install is set)");
    }
    let owner = missing
        .iter()
        .filter(|step| needs_owner(step))
        .map(|step| step.title.as_str())
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

/// [`ensure`] on this machine for an app command, answering questions on the terminal.
pub fn ensure_for_command(
    context: &ProjectContext,
    action: &str,
    target: &str,
    no_install: bool,
) -> Result<()> {
    let sets = required_sets(action, target);
    let env = setup::environment_for(context.host.clone(), Some(&context.project), false);
    let mut runner = Runner::execute();
    let mut user_path = UserPath::system(env.host.os)
        .map_err(|error| anyhow!("cannot edit the user PATH: {error}"))?;
    let machine = SystemMachine;
    let options = Options::from_terminal(no_install);
    ensure(
        &env,
        &sets,
        &options,
        &mut runner,
        &machine,
        &mut user_path,
        &mut io::stdout(),
        &mut ask_on_terminal,
    )
    .map(|_| ())
}

fn ask_on_terminal(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    if io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
