//! Machine setup: requirement sets expand into an ordered plan of steps, each with a probe and an
//! install, and one engine plans, checks and executes them.
//!
//! One step table feeds `setup`, `setup --check` and `doctor`; there is no second requirement
//! list. A probe never installs anything, and a second run of `setup` executes nothing.

pub mod android;
pub mod gpu;
pub mod ios;
pub mod linux;
pub mod machine;
pub mod macos;
pub mod rust;
pub mod test_tools;
pub mod web;
pub mod windows;
pub mod wsl;

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use crate::cli::SetupArgs;
use crate::host::path_env::{PathChange, UserPath};
use crate::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Privilege, RunError, Runner};
use crate::project::{Pins, Project};

pub use machine::{FakeMachine, Machine, SystemMachine};

/// A group of requirements the user can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Set {
    /// Building gpux and RayX natively: system packages, Rust and its toolchain.
    Base,
    /// The browser build: the wasm target, `wasm-bindgen` and the pinned nightly.
    Web,
    /// Browser tests: Node, pnpm and Playwright with its browsers.
    Test,
    /// Android builds and the emulator.
    Android,
    /// iOS builds and the Simulator (macOS only).
    Ios,
    /// GPU drivers and Vulkan tooling.
    Gpu,
}

impl Set {
    pub const ALL: [Set; 6] = [
        Set::Base,
        Set::Web,
        Set::Test,
        Set::Android,
        Set::Ios,
        Set::Gpu,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Set::Base => "base",
            Set::Web => "web",
            Set::Test => "test",
            Set::Android => "android",
            Set::Ios => "ios",
            Set::Gpu => "gpu",
        }
    }

    /// Whether the set exists for this host: iOS only on macOS, Android not on Linux ARM64.
    pub fn applies_to(self, host: &HostFacts) -> bool {
        match self {
            Set::Ios => host.os == Os::MacOs,
            Set::Android => !(host.os == Os::Linux && host.arch == Arch::Arm64),
            _ => true,
        }
    }
}

impl fmt::Display for Set {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What a plan needs to know to build its steps.
#[derive(Clone, Debug)]
pub struct PlanEnv {
    pub host: HostFacts,
    pub pins: Pins,
    /// The project's workspace root when setup runs inside one.
    pub project_root: Option<PathBuf>,
    /// `[workspace.metadata.rayx] tools-node`: the project's Playwright package directory,
    /// relative to the workspace root (`tools-node` when unset).
    pub tools_node: Option<String>,
    /// `--yes`: accept licenses and agreements, and make installers non-interactive.
    pub yes: bool,
}

/// What probes and installs act on.
pub struct Cx<'a> {
    pub env: &'a PlanEnv,
    pub runner: &'a mut Runner,
    pub machine: &'a dyn Machine,
    pub user_path: &'a mut UserPath,
    /// Set when an action added a directory to the user PATH.
    pub path_changed: bool,
    /// How long an action waits between polls of something that finishes on its own.
    pub poll_interval: Duration,
    /// Where macOS marks the on-demand Command Line Tools install; `None` is the system path.
    /// A test sets it to a temporary file, which is then really written even by a recording runner.
    pub clt_marker: Option<PathBuf>,
}

impl<'a> Cx<'a> {
    pub fn new(
        env: &'a PlanEnv,
        runner: &'a mut Runner,
        machine: &'a dyn Machine,
        user_path: &'a mut UserPath,
    ) -> Self {
        Self {
            env,
            runner,
            machine,
            user_path,
            path_changed: false,
            poll_interval: Duration::from_secs(5),
            clt_marker: None,
        }
    }

    pub fn with_clt_marker(mut self, marker: impl Into<PathBuf>) -> Self {
        self.clt_marker = Some(marker.into());
        self
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }
}

impl Cx<'_> {
    /// Runs a side-effect-free command and returns its outcome, or `None` when it cannot run
    /// (the program is not installed).
    pub fn query(&mut self, spec: &CommandSpec) -> Option<Outcome> {
        self.runner.query(spec).ok()
    }
}

/// The result of probing one step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probed {
    pub satisfied: bool,
    /// What is missing, for steps that install a list (the apt packages).
    pub missing: Vec<String>,
    /// A short reason shown next to the status.
    pub detail: String,
    /// The version found, when the probe reads one: `doctor` reports a present requirement
    /// with it, and an unmet one that has a version as a wrong version.
    pub found: Option<String>,
}

impl Probed {
    pub fn ok() -> Self {
        Self {
            satisfied: true,
            ..Self::default()
        }
    }

    pub fn missing(detail: impl Into<String>) -> Self {
        Self {
            satisfied: false,
            missing: Vec::new(),
            detail: detail.into(),
            found: None,
        }
    }

    /// Records the version the probe found.
    pub fn with_found(mut self, version: impl Into<String>) -> Self {
        self.found = Some(version.into());
        self
    }

    pub fn missing_items(items: Vec<String>) -> Self {
        Self {
            satisfied: items.is_empty(),
            detail: items.join(", "),
            missing: items,
            found: None,
        }
    }
}

/// One thing an install does.
#[derive(Clone)]
pub enum Action {
    Run(CommandSpec),
    /// Adds a directory to the user PATH.
    AddToPath(PathBuf),
    WriteFile {
        path: PathBuf,
        contents: String,
    },
    /// Creates a directory and its parents.
    CreateDir(PathBuf),
    /// Persistently sets a user environment variable to a path.
    SetUserEnv {
        name: String,
        value: PathBuf,
    },
    /// Removes a file; runs even when an earlier action failed, so markers never linger.
    RemoveFile(PathBuf),
    /// Logic that cannot be a list of commands because a later command depends on an earlier
    /// one's output. `description` is what `--check` shows.
    Custom {
        description: String,
        run: CustomFn,
    },
}

impl fmt::Debug for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Run(spec) => f.debug_tuple("Run").field(spec).finish(),
            Action::AddToPath(path) => f.debug_tuple("AddToPath").field(path).finish(),
            Action::WriteFile { path, .. } => f.debug_tuple("WriteFile").field(path).finish(),
            Action::CreateDir(path) => f.debug_tuple("CreateDir").field(path).finish(),
            Action::SetUserEnv { name, value } => f
                .debug_tuple("SetUserEnv")
                .field(name)
                .field(value)
                .finish(),
            Action::RemoveFile(path) => f.debug_tuple("RemoveFile").field(path).finish(),
            Action::Custom { description, .. } => {
                f.debug_tuple("Custom").field(description).finish()
            }
        }
    }
}

/// The logic of an [`Action::Custom`].
pub type CustomFn = Rc<dyn Fn(&mut Cx) -> Result<(), SetupError>>;

pub type ProbeFn = Box<dyn Fn(&mut Cx) -> Probed>;
pub type ActionsFn = Box<dyn Fn(&mut Cx, &Probed) -> Result<Vec<Action>, SetupError>>;

/// How a step installs.
pub enum Install {
    /// apt packages; all missing packages of all such steps install in one batched command.
    Apt(Vec<String>),
    /// Anything else, built from the probe result at execution time.
    Actions(ActionsFn),
}

/// One requirement: how to tell it is met, how to meet it, and what to tell the owner.
pub struct Step {
    pub id: &'static str,
    pub set: Set,
    pub title: String,
    pub privilege: Privilege,
    /// The fix for the owner when the step cannot run unattended.
    pub fix_hint: String,
    pub probe: ProbeFn,
    pub install: Install,
    /// When set, the step cannot be met until the machine restarts: after its install ran, a probe
    /// that still fails is reported with this notice instead of as a failure.
    pub reboot_notice: Option<String>,
    /// The install asks the user a question (a license) even though it needs no privileges, so an
    /// app command does not run it on its own.
    pub prompts: bool,
    /// Building does not need this step (an IDE, say): `rayx setup` installs it, an app command
    /// does not wait for it.
    pub optional_for_builds: bool,
}

impl Step {
    /// A step that installs apt packages: root, and probed through `dpkg-query`.
    pub fn apt(id: &'static str, set: Set, title: impl Into<String>, packages: &[&str]) -> Step {
        let packages: Vec<String> = packages.iter().map(|p| p.to_string()).collect();
        let probe_packages = packages.clone();
        Step {
            id,
            set,
            title: title.into(),
            privilege: Privilege::Root,
            fix_hint: format!("sudo apt-get install {}", packages.join(" ")),
            probe: Box::new(move |cx| linux::probe_packages(cx, &probe_packages)),
            install: Install::Apt(packages),
            reboot_notice: None,
            prompts: false,
            optional_for_builds: false,
        }
    }

    /// A step with a probe and an action builder.
    pub fn new(
        id: &'static str,
        set: Set,
        title: impl Into<String>,
        privilege: Privilege,
        probe: impl Fn(&mut Cx) -> Probed + 'static,
        actions: impl Fn(&mut Cx, &Probed) -> Result<Vec<Action>, SetupError> + 'static,
    ) -> Step {
        Step {
            id,
            set,
            title: title.into(),
            privilege,
            fix_hint: String::new(),
            probe: Box::new(probe),
            install: Install::Actions(Box::new(actions)),
            reboot_notice: None,
            prompts: false,
            optional_for_builds: false,
        }
    }

    /// The step needs a restart before its probe passes.
    pub fn with_reboot_notice(mut self, notice: impl Into<String>) -> Self {
        self.reboot_notice = Some(notice.into());
        self
    }

    /// The install asks the user a question.
    pub fn with_prompt(mut self) -> Self {
        self.prompts = true;
        self
    }

    /// Building does not need this step.
    pub fn optional_for_builds(mut self) -> Self {
        self.optional_for_builds = true;
        self
    }

    pub fn with_fix_hint(mut self, hint: impl Into<String>) -> Self {
        self.fix_hint = hint.into();
        self
    }
}

/// The ordered steps of the selected sets.
pub struct Plan {
    pub steps: Vec<Step>,
}

/// Why setup cannot continue.
#[derive(Debug)]
pub enum SetupError {
    /// The host or distribution is not supported; the message names what is.
    Unsupported(String),
    /// The set is not available in this build.
    SetNotAvailable(Set),
    /// A prerequisite of a step is missing, with the instruction that fixes it.
    Prerequisite(String),
    Run(RunError),
    Io(io::Error),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::Unsupported(message) | SetupError::Prerequisite(message) => {
                f.write_str(message)
            }
            SetupError::SetNotAvailable(set) => {
                write!(f, "the `{set}` set is not implemented in this build")
            }
            SetupError::Run(error) => write!(f, "{error}"),
            SetupError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SetupError {}

impl From<RunError> for SetupError {
    fn from(error: RunError) -> Self {
        SetupError::Run(error)
    }
}

impl From<io::Error> for SetupError {
    fn from(error: io::Error) -> Self {
        SetupError::Io(error)
    }
}

/// Expands `sets` (plus the base set, always first) into the ordered steps for this host.
pub fn plan(env: &PlanEnv, sets: &[Set]) -> Result<Plan, SetupError> {
    let mut wanted: Vec<Set> = vec![Set::Base];
    for set in Set::ALL {
        if sets.contains(&set) && !wanted.contains(&set) {
            wanted.push(set);
        }
    }
    let mut steps = Vec::new();
    for set in wanted {
        match set {
            Set::Base => steps.extend(match env.host.os {
                Os::Linux => linux::steps(env)?,
                Os::Windows => windows::steps(env)?,
                Os::MacOs => macos::steps(env)?,
            }),
            Set::Web => steps.extend(web::steps(env)?),
            Set::Test => steps.extend(test_tools::steps(env)?),
            Set::Android => steps.extend(android::steps(env)?),
            Set::Ios => steps.extend(ios::steps(env)?),
            Set::Gpu => steps.extend(gpu::steps(env)?),
        }
    }
    Ok(Plan { steps })
}

/// The status of every step of a plan.
pub struct Checked {
    pub probed: Vec<Probed>,
}

impl Checked {
    pub fn missing(&self) -> usize {
        self.probed.iter().filter(|p| !p.satisfied).count()
    }
}

/// Probes every step. Nothing is installed.
pub fn check(plan: &Plan, cx: &mut Cx) -> Checked {
    Checked {
        probed: plan.steps.iter().map(|step| (step.probe)(cx)).collect(),
    }
}

/// The command lines a step would run, one per action, for `--check`.
pub fn describe_actions(
    step: &Step,
    probed: &Probed,
    batch: &[String],
    cx: &mut Cx,
) -> Vec<String> {
    match &step.install {
        Install::Apt(_) => {
            if batch.is_empty() {
                return Vec::new();
            }
            apt_commands(cx.env, batch)
                .iter()
                .map(|spec| cx.runner.command_line(spec))
                .collect()
        }
        Install::Actions(build) => match build(cx, probed) {
            Ok(actions) => actions
                .iter()
                .map(|action| match action {
                    Action::Run(spec) => cx.runner.command_line(spec),
                    Action::AddToPath(dir) => format!("add {} to the user PATH", dir.display()),
                    Action::WriteFile { path, .. } => format!("create {}", path.display()),
                    Action::CreateDir(path) => format!("create the directory {}", path.display()),
                    Action::SetUserEnv { name, value } => {
                        format!("set {name} to {} for the user", value.display())
                    }
                    Action::RemoveFile(path) => format!("remove {}", path.display()),
                    Action::Custom { description, .. } => description.clone(),
                })
                .collect(),
            Err(error) => vec![format!("cannot install: {error}")],
        },
    }
}

/// `apt-get update` and one `apt-get install` for every missing package.
pub fn apt_commands(env: &PlanEnv, packages: &[String]) -> Vec<CommandSpec> {
    let mut install = CommandSpec::new("apt-get").arg("install");
    if env.yes {
        install = install.arg("-y");
    }
    vec![
        CommandSpec::new("apt-get")
            .arg("update")
            .root()
            .interactive(),
        install.args(packages.iter().cloned()).root().interactive(),
    ]
}

/// The packages of all not-yet-satisfied apt steps, in plan order and without repeats.
pub fn apt_batch(plan: &Plan, checked: &Checked) -> Vec<String> {
    let mut batch: Vec<String> = Vec::new();
    for (step, probed) in plan.steps.iter().zip(&checked.probed) {
        if matches!(step.install, Install::Apt(_)) && !probed.satisfied {
            for package in &probed.missing {
                if !batch.contains(package) {
                    batch.push(package.clone());
                }
            }
        }
    }
    batch
}

/// Renders the plan with each step's status and the exact commands that would run.
pub fn render_plan(plan: &Plan, checked: &Checked, cx: &mut Cx) -> String {
    let batch = apt_batch(plan, checked);
    let mut out = String::new();
    let mut batch_shown = false;
    for (step, probed) in plan.steps.iter().zip(&checked.probed) {
        let status = if probed.satisfied {
            "ok     "
        } else {
            "missing"
        };
        let mut line = format!("[{status}] {}: {}", step.set, step.title);
        let mut notes: Vec<String> = Vec::new();
        if !probed.detail.is_empty() {
            notes.push(probed.detail.clone());
        }
        if let Some(found) = &probed.found {
            notes.push(format!("found {found}"));
        }
        if !notes.is_empty() {
            line.push_str(&format!(" ({})", notes.join("; ")));
        }
        out.push_str(&line);
        out.push('\n');
        if probed.satisfied {
            continue;
        }
        // The batched apt commands appear once, under the first missing apt step.
        let is_apt = matches!(step.install, Install::Apt(_));
        let lines = if is_apt && batch_shown {
            vec!["(installed by the apt command above)".to_string()]
        } else {
            if is_apt {
                batch_shown = true;
            }
            describe_actions(step, probed, &batch, cx)
        };
        for command in lines {
            out.push_str(&format!("            {command}\n"));
        }
    }
    out
}

/// How a step ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    /// Already met; nothing ran.
    Satisfied,
    Installed,
    Failed(String),
    /// Not attempted because an earlier step failed.
    NotRun,
}

/// What `execute` did.
#[derive(Debug)]
pub struct Report {
    pub steps: Vec<(&'static str, StepOutcome)>,
    /// Things the user still has to do, such as restarting the machine.
    pub notices: Vec<String>,
}

impl Report {
    /// How many steps installed something.
    pub fn executed(&self) -> usize {
        self.steps
            .iter()
            .filter(|(_, outcome)| *outcome == StepOutcome::Installed)
            .count()
    }

    pub fn failure(&self) -> Option<&str> {
        self.steps.iter().find_map(|(_, outcome)| match outcome {
            StepOutcome::Failed(message) => Some(message.as_str()),
            _ => None,
        })
    }

    pub fn succeeded(&self) -> bool {
        self.failure().is_none()
    }
}

/// Runs the steps that are missing: probe, skip the met ones, install the rest, re-probe.
/// Stops at the first failure; later steps depend on earlier ones.
pub fn execute(plan: &Plan, cx: &mut Cx, out: &mut dyn Write) -> Report {
    let checked = check(plan, cx);
    let mut outcomes: Vec<StepOutcome> = checked
        .probed
        .iter()
        .map(|probed| {
            if probed.satisfied {
                StepOutcome::Satisfied
            } else {
                StepOutcome::NotRun
            }
        })
        .collect();
    let mut batch_done = false;
    let mut notices: Vec<String> = Vec::new();

    for (index, step) in plan.steps.iter().enumerate() {
        if outcomes[index] != StepOutcome::NotRun {
            continue;
        }
        let _ = writeln!(out, "==> {}: {}", step.set, step.title);
        let result = match &step.install {
            Install::Apt(_) => {
                if batch_done {
                    Ok(())
                } else {
                    batch_done = true;
                    let batch = apt_batch(plan, &checked);
                    run_commands(cx, &apt_commands(cx.env, &batch))
                }
            }
            Install::Actions(build) => {
                build(cx, &checked.probed[index]).and_then(|actions| run_actions(cx, actions))
            }
        };
        let outcome = match result {
            Ok(()) if cx.runner.is_dry_run() => StepOutcome::Installed,
            Ok(()) => {
                let again = (step.probe)(cx);
                if again.satisfied {
                    StepOutcome::Installed
                } else if let Some(notice) = &step.reboot_notice {
                    notices.push(notice.clone());
                    StepOutcome::Installed
                } else {
                    StepOutcome::Failed(format!(
                        "{} is still missing after its install ran{}{}",
                        step.title,
                        if again.detail.is_empty() { "" } else { ": " },
                        again.detail
                    ))
                }
            }
            Err(error) => StepOutcome::Failed(format!("{}: {error}", step.title)),
        };
        let failed = matches!(outcome, StepOutcome::Failed(_));
        outcomes[index] = outcome;
        if failed {
            break;
        }
    }

    Report {
        steps: plan
            .steps
            .iter()
            .map(|step| step.id)
            .zip(outcomes)
            .collect(),
        notices,
    }
}

fn run_commands(cx: &mut Cx, specs: &[CommandSpec]) -> Result<(), SetupError> {
    for spec in specs {
        cx.runner.run_checked(spec)?;
    }
    Ok(())
}

/// Runs the actions in order; after a failure only the cleanup actions still run.
fn run_actions(cx: &mut Cx, actions: Vec<Action>) -> Result<(), SetupError> {
    let dry_run = cx.runner.is_dry_run();
    let mut failure: Option<SetupError> = None;
    for action in actions {
        if failure.is_some() && !matches!(action, Action::RemoveFile(_)) {
            continue;
        }
        let result: Result<(), SetupError> = match &action {
            Action::Run(spec) => cx.runner.run_checked(spec).map(|_| ()).map_err(Into::into),
            Action::AddToPath(dir) => cx
                .user_path
                .add(dir, dry_run)
                .map(|change| {
                    if change == PathChange::Added && !dry_run {
                        cx.path_changed = true;
                    }
                })
                .map_err(Into::into),
            Action::WriteFile { path, contents } => {
                if dry_run {
                    Ok(())
                } else {
                    write_file(path, contents).map_err(Into::into)
                }
            }
            Action::Custom { run, .. } => run(cx),
            Action::SetUserEnv { name, value } => cx
                .user_path
                .set_variable(name, value, dry_run)
                .map(|change| {
                    if change == PathChange::Added && !dry_run {
                        cx.path_changed = true;
                    }
                })
                .map_err(Into::into),
            Action::CreateDir(path) => {
                if dry_run {
                    Ok(())
                } else {
                    std::fs::create_dir_all(path).map_err(Into::into)
                }
            }
            Action::RemoveFile(path) => {
                if dry_run {
                    Ok(())
                } else {
                    match std::fs::remove_file(path) {
                        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
                        _ => Ok(()),
                    }
                }
            }
        };
        if let Err(error) = result {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn write_file(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

/// The plan environment of the current directory: its project (none is fine, since `setup` also
/// runs on a fresh machine before cargo exists), the resolved pins and the host.
pub fn environment(host: HostFacts, yes: bool) -> (PlanEnv, Option<Project>) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let project = Project::discover(&mut Runner::print(), &cwd, None).ok();
    (environment_for(host, project.as_ref(), yes), project)
}

/// The plan environment of a known project (or none).
pub fn environment_for(host: HostFacts, project: Option<&Project>, yes: bool) -> PlanEnv {
    PlanEnv {
        host,
        pins: Pins::resolve(project),
        project_root: project.map(|p| p.workspace_root.clone()),
        tools_node: project
            .and_then(|p| p.rayx_metadata.as_ref())
            .and_then(|metadata| metadata.get("tools-node"))
            .and_then(|value| value.as_str())
            .map(str::to_string),
        yes,
    }
}

/// Runs `rayx setup` against the real machine and returns the process exit code.
pub fn run(args: &SetupArgs) -> u8 {
    if args.wsl {
        return wsl::run(args);
    }
    let host = crate::host::facts();
    let mut runner = if args.check {
        Runner::print()
    } else {
        Runner::execute()
    };
    let (env, _) = environment(host.clone(), args.yes);
    let mut user_path = match UserPath::system(host.os) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("rayx: cannot edit the user PATH: {error}");
            return 1;
        }
    };
    let machine = SystemMachine;
    run_with(
        args,
        &env,
        &mut runner,
        &machine,
        &mut user_path,
        &mut io::stdout(),
    )
}

/// The body of `rayx setup`, over injectable collaborators.
pub fn run_with(
    args: &SetupArgs,
    env: &PlanEnv,
    runner: &mut Runner,
    machine: &dyn Machine,
    user_path: &mut UserPath,
    out: &mut dyn Write,
) -> u8 {
    let sets = selected_sets(args, &env.host);
    let plan = match plan(env, &sets) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("rayx: {error}");
            return 1;
        }
    };
    let mut cx = Cx::new(env, runner, machine, user_path);
    run_plan(args, &plan, &mut cx, out)
}

/// Checks or executes `plan` according to `args` and returns the exit code: `--check` is 1 when
/// anything is missing, a run is 1 when a step fails.
pub fn run_plan(args: &SetupArgs, plan: &Plan, cx: &mut Cx, out: &mut dyn Write) -> u8 {
    if args.check {
        let checked = check(plan, cx);
        let _ = write!(out, "{}", render_plan(plan, &checked, cx));
        let missing = checked.missing();
        let _ = if missing == 0 {
            writeln!(out, "Everything is installed.")
        } else {
            writeln!(
                out,
                "{missing} step(s) missing; `rayx setup` installs them."
            )
        };
        return u8::from(missing > 0);
    }
    let report = execute(plan, cx, out);
    if let Some(message) = report.failure() {
        eprintln!("rayx: setup stopped: {message}");
        return 1;
    }
    let _ = if report.executed() == 0 {
        writeln!(out, "Nothing to do: everything is installed.")
    } else {
        writeln!(out, "Installed {} step(s).", report.executed())
    };
    for notice in &report.notices {
        let _ = writeln!(out, "{notice}");
    }
    if cx.path_changed {
        // Running shells keep the PATH they started with.
        let _ = writeln!(out, "Open a new shell for the PATH change to take effect.");
    }
    0
}

/// The sets named by the flags: `--all` is every set that applies to the host.
pub fn selected_sets(args: &SetupArgs, host: &HostFacts) -> Vec<Set> {
    if args.all {
        return Set::ALL
            .into_iter()
            .filter(|set| set.applies_to(host))
            .collect();
    }
    let mut sets = vec![Set::Base];
    for (flag, set) in [
        (args.web, Set::Web),
        (args.test, Set::Test),
        (args.android, Set::Android),
        (args.ios, Set::Ios),
        (args.gpu, Set::Gpu),
    ] {
        if flag {
            sets.push(set);
        }
    }
    sets
}
