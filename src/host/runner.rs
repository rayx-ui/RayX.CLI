//! The one place external programs run.
//!
//! A [`Runner`] executes a [`CommandSpec`] in one of three [`Mode`]s: really (`Execute`), by
//! printing the exact command line instead (`Print`, used by `--check` and dry runs, with no side
//! effects), or by recording the spec for a test (`Record`).
//!
//! Privilege is per command. `rayx` never runs elevated itself and never sees a password:
//! [`Privilege::Root`] runs through `sudo` in the user's console, and [`Privilege::Admin`] on
//! Windows runs as one separate elevated child behind the system UAC prompt.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::Os;

/// The rights a command needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Privilege {
    /// The user's own rights.
    #[default]
    None,
    /// root on Linux and macOS: `sudo` when not already root.
    Root,
    /// Administrator on Windows: one elevated child process behind the UAC prompt.
    Admin,
}

/// An external program to run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub privilege: Privilege,
    /// Inherit the console so prompts (`sudo`, license questions) reach the user and output is
    /// shown as it happens. A non-interactive command has its output captured in [`Outcome`].
    pub interactive: bool,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn root(mut self) -> Self {
        self.privilege = Privilege::Root;
        self
    }

    pub fn admin(mut self) -> Self {
        self.privilege = Privilege::Admin;
        self
    }

    pub fn interactive(mut self) -> Self {
        self.interactive = true;
        self
    }
}

/// How a [`Runner`] treats the commands it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Execute,
    Print,
    Record,
}

/// What a command produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The exit code; `None` when the process was ended by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Outcome {
    pub fn success() -> Self {
        Self {
            code: Some(0),
            ..Self::default()
        }
    }

    pub fn failure(code: i32) -> Self {
        Self {
            code: Some(code),
            ..Self::default()
        }
    }

    pub fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
        self.stdout = stdout.into();
        self
    }

    pub fn is_success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Why a command did not run to completion.
#[derive(Debug)]
pub enum RunError {
    /// The program is not installed or not on `PATH`.
    NotFound { program: String },
    /// The program could not be started.
    Io { program: String, source: io::Error },
    /// The user declined the UAC prompt or interrupted `sudo`; the step did not run.
    Declined { command_line: String },
    /// The command's privilege level does not exist on this operating system, or the command
    /// cannot be elevated safely.
    Unsupported {
        command_line: String,
        reason: String,
    },
    /// The command ran and exited unsuccessfully ([`Runner::run_checked`]).
    Failed {
        command_line: String,
        code: Option<i32>,
        output: String,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::NotFound { program } => {
                write!(f, "`{program}` was not found; install it or add it to PATH")
            }
            RunError::Io { program, source } => write!(f, "could not start `{program}`: {source}"),
            RunError::Declined { command_line } => write!(
                f,
                "the step was stopped because the administrator or sudo prompt was declined: \
                 `{command_line}` did not run"
            ),
            RunError::Unsupported {
                command_line,
                reason,
            } => write!(f, "cannot run `{command_line}`: {reason}"),
            RunError::Failed {
                command_line,
                code,
                output,
            } => {
                match code {
                    Some(code) => write!(f, "`{command_line}` exited with code {code}")?,
                    None => write!(f, "`{command_line}` was ended by a signal")?,
                }
                let output = output.trim();
                if output.is_empty() {
                    Ok(())
                } else {
                    write!(f, ": {output}")
                }
            }
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Runs a command as a separate elevated process (Windows `Admin` privilege).
pub trait Elevator {
    fn run_elevated(&self, spec: &CommandSpec) -> Result<Outcome, RunError>;
}

/// Executes, prints or records commands.
pub struct Runner {
    mode: Mode,
    os: Os,
    is_root: bool,
    out: Box<dyn Write>,
    elevator: Box<dyn Elevator>,
    sudo: String,
    lines: Vec<String>,
    specs: Vec<CommandSpec>,
    responses: VecDeque<Response>,
}

struct Response {
    matches: Box<dyn Fn(&CommandSpec) -> bool>,
    outcome: Outcome,
}

impl Runner {
    /// Runs commands for real on this machine.
    pub fn execute() -> Self {
        Self::new(Mode::Execute, Box::new(io::stdout()))
    }

    /// Prints the exact command line of every command to standard output instead of running it.
    pub fn print() -> Self {
        Self::new(Mode::Print, Box::new(io::stdout()))
    }

    /// Prints command lines to `out` instead of running them.
    pub fn print_to(out: impl Write + 'static) -> Self {
        Self::new(Mode::Print, Box::new(out))
    }

    /// Records commands for a test; nothing runs and nothing is printed.
    pub fn record() -> Self {
        Self::new(Mode::Record, Box::new(io::sink()))
    }

    fn new(mode: Mode, out: Box<dyn Write>) -> Self {
        Self {
            mode,
            os: current_os(),
            is_root: current_is_root(),
            out,
            elevator: default_elevator(),
            sudo: "sudo".to_string(),
            lines: Vec::new(),
            specs: Vec::new(),
            responses: VecDeque::new(),
        }
    }

    /// Renders command lines for `os` instead of this machine's operating system.
    pub fn with_os(mut self, os: Os) -> Self {
        self.os = os;
        self
    }

    /// Overrides whether the process counts as root (no `sudo` is needed when it does).
    pub fn with_root(mut self, is_root: bool) -> Self {
        self.is_root = is_root;
        self
    }

    /// Runs root steps through `program` instead of `sudo`, which tests point at a fake.
    pub fn with_sudo_program(mut self, program: impl Into<String>) -> Self {
        self.sudo = program.into();
        self
    }

    /// Replaces the elevation mechanism, which tests use to script a declined prompt.
    pub fn with_elevator(mut self, elevator: impl Elevator + 'static) -> Self {
        self.elevator = Box::new(elevator);
        self
    }

    /// In `Record` mode, answers the next command that `matches` with `outcome` (first match
    /// wins); every other command succeeds with no output.
    pub fn respond(
        mut self,
        matches: impl Fn(&CommandSpec) -> bool + 'static,
        outcome: Outcome,
    ) -> Self {
        self.responses.push_back(Response {
            matches: Box::new(matches),
            outcome,
        });
        self
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether commands have no effect on the machine. Callers that change something other than
    /// through a command (the user PATH, a file) must skip it while this is true.
    pub fn is_dry_run(&self) -> bool {
        self.mode != Mode::Execute
    }

    /// The operating system command lines are rendered for.
    pub fn os(&self) -> Os {
        self.os
    }

    /// Whether the process counts as root.
    pub fn is_root(&self) -> bool {
        self.is_root
    }

    /// The command lines printed or recorded so far.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The specs recorded so far.
    pub fn specs(&self) -> &[CommandSpec] {
        &self.specs
    }

    /// The exact command line `spec` runs as on this runner's operating system.
    pub fn command_line(&self, spec: &CommandSpec) -> String {
        render(spec, self.os, self.is_root)
    }

    /// Runs `spec` according to the mode.
    pub fn run(&mut self, spec: &CommandSpec) -> Result<Outcome, RunError> {
        let line = self.command_line(spec);
        match self.mode {
            Mode::Print => {
                // A closed stdout leaves nothing more to report.
                let _ = writeln!(self.out, "{line}");
                self.lines.push(line);
                Ok(Outcome::success())
            }
            Mode::Record => {
                self.lines.push(line);
                self.specs.push(spec.clone());
                let index = self.responses.iter().position(|r| (r.matches)(spec));
                Ok(match index.and_then(|i| self.responses.remove(i)) {
                    Some(response) => response.outcome,
                    None => Outcome::success(),
                })
            }
            Mode::Execute => self.execute_spec(spec, line),
        }
    }

    /// Like [`Runner::run`], but a command that exits unsuccessfully is an error.
    pub fn run_checked(&mut self, spec: &CommandSpec) -> Result<Outcome, RunError> {
        let outcome = self.run(spec)?;
        if outcome.is_success() {
            Ok(outcome)
        } else {
            Err(RunError::Failed {
                command_line: self.command_line(spec),
                code: outcome.code,
                output: if outcome.stderr.trim().is_empty() {
                    outcome.stdout
                } else {
                    outcome.stderr
                },
            })
        }
    }

    fn execute_spec(&mut self, spec: &CommandSpec, line: String) -> Result<Outcome, RunError> {
        match spec.privilege {
            Privilege::None => run_process(spec, false),
            Privilege::Root if self.is_root => run_process(spec, false),
            Privilege::Root if self.os == Os::Windows => Err(RunError::Unsupported {
                command_line: line,
                reason: "root steps do not exist on Windows; use an administrator step".into(),
            }),
            Privilege::Root => {
                // Authenticate first, in the user's console: a refused or empty password makes
                // `sudo -v` fail, which is a declined prompt, while a failing wrapped command
                // stays an ordinary failure. `rayx` never sees the password.
                let auth = CommandSpec::new(self.sudo.as_str()).arg("-v").interactive();
                let authenticated = run_process(&auth, true)?;
                if !authenticated.is_success() {
                    return Err(RunError::Declined { command_line: line });
                }
                let outcome = run_process(&sudo_wrapped(spec, &self.sudo), true)?;
                if outcome.code.is_none() {
                    // `sudo` ended by a signal: the user interrupted the step.
                    Err(RunError::Declined { command_line: line })
                } else {
                    Ok(outcome)
                }
            }
            Privilege::Admin if self.os != Os::Windows => Err(RunError::Unsupported {
                command_line: line,
                reason: "administrator steps exist only on Windows; use a root step".into(),
            }),
            Privilege::Admin => self.elevator.run_elevated(spec),
        }
    }
}

fn current_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else {
        Os::Linux
    }
}

#[cfg(unix)]
fn current_is_root() -> bool {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn current_is_root() -> bool {
    false
}

#[cfg(windows)]
fn default_elevator() -> Box<dyn Elevator> {
    Box::new(WindowsElevator)
}

#[cfg(not(windows))]
fn default_elevator() -> Box<dyn Elevator> {
    Box::new(NoElevator)
}

/// The elevator on systems without UAC: every request is unsupported.
#[cfg(not(windows))]
struct NoElevator;

#[cfg(not(windows))]
impl Elevator for NoElevator {
    fn run_elevated(&self, spec: &CommandSpec) -> Result<Outcome, RunError> {
        Err(RunError::Unsupported {
            command_line: render(spec, current_os(), false),
            reason: "administrator steps exist only on Windows".into(),
        })
    }
}

/// `sudo [env K=V ...] program args...`: `sudo` resets the environment, so variables go through
/// `env`. The working directory survives `sudo`.
fn sudo_wrapped(spec: &CommandSpec, sudo: &str) -> CommandSpec {
    let mut args = Vec::new();
    if !spec.env.is_empty() {
        args.push("env".to_string());
        args.extend(spec.env.iter().map(|(key, value)| format!("{key}={value}")));
    }
    args.push(spec.program.clone());
    args.extend(spec.args.iter().cloned());
    CommandSpec {
        program: sudo.to_string(),
        args,
        env: Vec::new(),
        cwd: spec.cwd.clone(),
        privilege: Privilege::None,
        interactive: spec.interactive,
    }
}

/// Runs one process. `inherit_stdin` hands the console to a wrapper that may prompt (`sudo`).
fn run_process(spec: &CommandSpec, inherit_stdin: bool) -> Result<Outcome, RunError> {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    if inherit_stdin || spec.interactive {
        command.stdin(Stdio::inherit());
    } else {
        command.stdin(Stdio::null());
    }
    let map_error = |source: io::Error| {
        if source.kind() == io::ErrorKind::NotFound {
            RunError::NotFound {
                program: spec.program.clone(),
            }
        } else {
            RunError::Io {
                program: spec.program.clone(),
                source,
            }
        }
    };
    if spec.interactive {
        let status = command
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(map_error)?;
        Ok(Outcome {
            code: status.code(),
            ..Outcome::default()
        })
    } else {
        let output = command.output().map_err(map_error)?;
        Ok(Outcome {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn render(spec: &CommandSpec, os: Os, is_root: bool) -> String {
    let windows = os == Os::Windows;
    let quote = |arg: &str| {
        if windows {
            quote_windows(arg)
        } else {
            quote_posix(arg)
        }
    };
    let mut line = String::new();
    if spec.privilege == Privilege::Admin {
        line.push_str("[admin] ");
    }
    if let Some(cwd) = &spec.cwd {
        let cwd = cwd.display().to_string();
        if windows {
            line.push_str(&format!("cd /d {} && ", quote(&cwd)));
        } else {
            line.push_str(&format!("cd {} && ", quote(&cwd)));
        }
    }
    let sudo = spec.privilege == Privilege::Root && !is_root;
    if sudo {
        line.push_str("sudo ");
    }
    if !spec.env.is_empty() {
        if windows {
            for (key, value) in &spec.env {
                line.push_str(&format!("set \"{key}={value}\" && "));
            }
        } else if sudo {
            line.push_str("env ");
            for (key, value) in &spec.env {
                line.push_str(&format!("{} ", quote(&format!("{key}={value}"))));
            }
        } else {
            for (key, value) in &spec.env {
                line.push_str(&format!("{key}={} ", quote(value)));
            }
        }
    }
    line.push_str(&quote(&spec.program));
    for arg in &spec.args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    line
}

fn quote_posix(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

fn quote_windows(arg: &str) -> String {
    let plain = !arg.is_empty()
        && !arg
            .chars()
            .any(|c| c.is_whitespace() || "\"&|<>^()%!;,'".contains(c));
    if plain {
        return arg.to_string();
    }
    // CommandLineToArgvW rules: backslashes only matter before a quote, where they double, and
    // before the closing quote added here.
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            c => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(c);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(windows)]
pub use windows::WindowsElevator;

#[cfg(windows)]
mod windows {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, INFINITE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

    use super::{CommandSpec, Elevator, Outcome, RunError, io, quote_windows, render};
    use crate::host::Os;

    /// Elevates one command through `ShellExecuteExW` with the `runas` verb, so the user sees the
    /// system UAC prompt. The elevated process cannot share this console, so it runs under
    /// `cmd.exe` with its output redirected to a temporary file that is read back afterwards.
    pub struct WindowsElevator;

    impl Elevator for WindowsElevator {
        fn run_elevated(&self, spec: &CommandSpec) -> Result<Outcome, RunError> {
            let command_line = render(spec, Os::Windows, false);
            // `cmd.exe` expands `%` even inside quotes and the quoting below does not escape a
            // quote character; the commands `rayx` elevates contain neither.
            let unsafe_arg = std::iter::once(&spec.program)
                .chain(&spec.args)
                .chain(spec.env.iter().flat_map(|(k, v)| [k, v]))
                .any(|a| a.contains(['"', '%', '\r', '\n']));
            if unsafe_arg {
                return Err(RunError::Unsupported {
                    command_line,
                    reason: "an elevated command cannot contain quotes, % or line breaks".into(),
                });
            }

            let log = std::env::temp_dir().join(format!(
                "rayx-admin-{}-{}.log",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            let log_text = log.display().to_string();

            let mut inner = String::new();
            for (key, value) in &spec.env {
                inner.push_str(&format!("set \"{key}={value}\" && "));
            }
            inner.push_str(&quote_windows(&spec.program));
            for arg in &spec.args {
                inner.push(' ');
                inner.push_str(&quote_windows(arg));
            }
            // `/s` strips the outer quotes, so the inner command line keeps its own.
            let parameters = format!("/d /s /c \"{inner} > \"{log_text}\" 2>&1\"");

            let directory = spec
                .cwd
                .clone()
                .or_else(|| std::env::current_dir().ok())
                .map(|dir| wide(dir.as_os_str()));
            let verb = wide(OsStr::new("runas"));
            let file = wide(OsStr::new("cmd.exe"));
            let parameters = wide(OsStr::new(&parameters));

            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS,
                lpVerb: verb.as_ptr(),
                lpFile: file.as_ptr(),
                lpParameters: parameters.as_ptr(),
                lpDirectory: directory.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                nShow: if spec.interactive {
                    SW_SHOWNORMAL
                } else {
                    SW_HIDE
                },
                ..SHELLEXECUTEINFOW::default()
            };
            // SAFETY: `info` is fully initialized, and every string it points to outlives the call.
            let started = unsafe { ShellExecuteExW(&mut info) };
            if started == 0 {
                // SAFETY: `GetLastError` has no preconditions.
                let error = unsafe { GetLastError() };
                return Err(if error == ERROR_CANCELLED {
                    RunError::Declined { command_line }
                } else {
                    RunError::Io {
                        program: spec.program.clone(),
                        source: io::Error::from_raw_os_error(error as i32),
                    }
                });
            }

            let process = info.hProcess;
            let mut exit_code = 1u32;
            // SAFETY: `process` is the handle `SEE_MASK_NOCLOSEPROCESS` returned for the child;
            // it is waited on, queried and closed exactly once.
            unsafe {
                WaitForSingleObject(process, INFINITE);
                GetExitCodeProcess(process, &mut exit_code);
                CloseHandle(process);
            }

            let text = std::fs::read(&log)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default();
            // The log is a scratch file; failing to delete it only leaves a temp file behind.
            let _ = std::fs::remove_file(&log);
            if spec.interactive {
                print!("{text}");
            }
            Ok(Outcome {
                code: Some(exit_code as i32),
                stdout: text,
                stderr: String::new(),
            })
        }
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }
}
