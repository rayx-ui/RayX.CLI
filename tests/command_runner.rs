//! The command runner: print, record and execute modes, privilege handling and declined prompts.

use std::io::Write;
use std::sync::{Arc, Mutex};

use rayx_cli::host::{CommandSpec, Elevator, Mode, Os, Outcome, Privilege, RunError, Runner};

/// A writer whose contents the test can read back after the runner took ownership.
#[derive(Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl SharedBuffer {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("buffer").clone()).expect("UTF-8")
    }
}

impl Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("buffer").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Stands in for the UAC prompt: records what it was asked to run and either runs it or declines.
struct ScriptedElevator {
    decline: bool,
    seen: Arc<Mutex<Vec<CommandSpec>>>,
}

impl Elevator for ScriptedElevator {
    fn run_elevated(&self, spec: &CommandSpec) -> Result<Outcome, RunError> {
        self.seen.lock().expect("seen").push(spec.clone());
        if self.decline {
            Err(RunError::Declined {
                command_line: spec.program.clone(),
            })
        } else {
            Ok(Outcome::success().with_stdout("elevated output"))
        }
    }
}

fn apt_install() -> CommandSpec {
    CommandSpec::new("apt-get")
        .args(["install", "-y", "build-essential"])
        .root()
}

#[test]
fn print_mode_writes_sudo_for_root_steps_when_not_root() {
    let buffer = SharedBuffer::default();
    let mut runner = Runner::print_to(buffer.clone())
        .with_os(Os::Linux)
        .with_root(false);
    runner.run(&apt_install()).expect("print succeeds");
    assert_eq!(buffer.text(), "sudo apt-get install -y build-essential\n");
    assert_eq!(runner.lines(), ["sudo apt-get install -y build-essential"]);
}

#[test]
fn print_mode_omits_sudo_when_already_root() {
    let buffer = SharedBuffer::default();
    let mut runner = Runner::print_to(buffer.clone())
        .with_os(Os::Linux)
        .with_root(true);
    runner.run(&apt_install()).expect("print succeeds");
    assert_eq!(buffer.text(), "apt-get install -y build-essential\n");
}

#[test]
fn print_mode_marks_admin_steps_and_quotes_for_the_target_os() {
    let spec = CommandSpec::new("winget")
        .args(["install", "--id", "Microsoft.VisualStudio.2022.BuildTools"])
        .arg("--override")
        .arg("--add Microsoft.VisualStudio.Workload.VCTools --quiet")
        .env("ACCEPT", "yes")
        .cwd("C:\\Work Dir")
        .admin();
    let mut runner = Runner::print_to(SharedBuffer::default()).with_os(Os::Windows);
    runner.run(&spec).expect("print succeeds");
    assert_eq!(
        runner.lines(),
        [
            "[admin] cd /d \"C:\\Work Dir\" && set \"ACCEPT=yes\" && winget install --id \
             Microsoft.VisualStudio.2022.BuildTools --override \
             \"--add Microsoft.VisualStudio.Workload.VCTools --quiet\""
        ]
    );
}

#[test]
fn print_mode_renders_sudo_environment_and_posix_quoting() {
    let spec = CommandSpec::new("sh")
        .args(["-c", "echo it's fine"])
        .env("DEBIAN_FRONTEND", "noninteractive")
        .root();
    let runner = Runner::print_to(SharedBuffer::default())
        .with_os(Os::Linux)
        .with_root(false);
    assert_eq!(
        runner.command_line(&spec),
        r"sudo env DEBIAN_FRONTEND=noninteractive sh -c 'echo it'\''s fine'"
    );
}

#[test]
fn print_mode_has_no_side_effects() {
    let mut runner = Runner::print_to(SharedBuffer::default());
    assert!(runner.is_dry_run());
    assert_eq!(runner.mode(), Mode::Print);
    // Neither a missing program nor an elevated step is attempted: both print and succeed.
    let missing = CommandSpec::new("rayx-no-such-program-for-print-mode");
    assert!(runner.run(&missing).expect("printed, not run").is_success());
    assert!(
        runner
            .run(&CommandSpec::new("setup.exe").admin())
            .expect("printed, not elevated")
            .is_success()
    );
    assert_eq!(runner.lines().len(), 2);
}

#[test]
fn record_mode_captures_specs_and_answers_scripted_commands() {
    let mut runner = Runner::record()
        .respond(
            |spec| spec.program == "rustup",
            Outcome::success().with_stdout("stable-x86_64\n"),
        )
        .respond(|spec| spec.program == "cargo", Outcome::failure(101));
    assert!(runner.is_dry_run());

    let listed = runner
        .run(&CommandSpec::new("rustup").args(["toolchain", "list"]))
        .expect("recorded");
    assert_eq!(listed.stdout, "stable-x86_64\n");
    let built = runner
        .run(&CommandSpec::new("cargo").arg("build"))
        .expect("recorded");
    assert_eq!(built.code, Some(101));
    let other = runner
        .run(&CommandSpec::new("git").arg("status"))
        .expect("recorded");
    assert!(other.is_success(), "an unscripted command succeeds");

    let programs: Vec<&str> = runner.specs().iter().map(|s| s.program.as_str()).collect();
    assert_eq!(programs, ["rustup", "cargo", "git"]);
    assert_eq!(runner.specs()[0].args, ["toolchain", "list"]);
    assert_eq!(runner.specs()[2].privilege, Privilege::None);
}

#[test]
fn execute_captures_output_unless_the_command_is_interactive() {
    let mut runner = Runner::execute();
    let captured = runner
        .run(&CommandSpec::new("rustc").arg("--version"))
        .expect("rustc runs");
    assert!(captured.is_success());
    assert!(
        captured.stdout.starts_with("rustc "),
        "captured stdout: {:?}",
        captured.stdout
    );

    // An interactive command inherits the console, so its output is not captured.
    let inherited = runner
        .run(&CommandSpec::new("rustc").arg("--version").interactive())
        .expect("rustc runs");
    assert!(inherited.is_success());
    assert!(inherited.stdout.is_empty() && inherited.stderr.is_empty());
}

#[test]
fn execute_reports_a_missing_program_and_failed_commands() {
    let mut runner = Runner::execute();
    let missing = runner
        .run(&CommandSpec::new("rayx-no-such-program-for-execute"))
        .expect_err("not installed");
    assert!(matches!(missing, RunError::NotFound { .. }), "{missing}");
    assert!(
        missing
            .to_string()
            .contains("rayx-no-such-program-for-execute")
    );

    let failed = runner
        .run_checked(&CommandSpec::new("rustc").arg("--definitely-not-a-flag"))
        .expect_err("rustc rejects the flag");
    let RunError::Failed { code, .. } = &failed else {
        panic!("expected Failed, got {failed}");
    };
    assert_eq!(*code, Some(1));
}

#[test]
fn root_steps_run_directly_when_already_root() {
    // As root (a container), `sudo` is skipped: the program itself runs.
    let mut runner = Runner::execute().with_os(Os::Linux).with_root(true);
    let outcome = runner
        .run(&CommandSpec::new("rustc").arg("--version").root())
        .expect("runs without sudo");
    assert!(outcome.stdout.starts_with("rustc "));
}

#[test]
fn a_declined_elevation_stops_the_step_with_a_clear_message() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut runner = Runner::execute()
        .with_os(Os::Windows)
        .with_elevator(ScriptedElevator {
            decline: true,
            seen: seen.clone(),
        });
    let spec = CommandSpec::new("wsl")
        .args(["--install", "--no-distribution"])
        .admin();
    let error = runner.run(&spec).expect_err("the prompt was declined");
    assert!(matches!(error, RunError::Declined { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("declined") && message.contains("did not run"),
        "{message}"
    );
    assert_eq!(
        seen.lock().expect("seen").len(),
        1,
        "the elevator was asked once"
    );
}

#[test]
fn an_approved_elevation_returns_the_childs_outcome() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut runner = Runner::execute()
        .with_os(Os::Windows)
        .with_elevator(ScriptedElevator {
            decline: false,
            seen: seen.clone(),
        });
    let outcome = runner
        .run(&CommandSpec::new("dism").arg("/online").admin())
        .expect("elevated");
    assert_eq!(outcome.stdout, "elevated output");
    assert_eq!(seen.lock().expect("seen")[0].privilege, Privilege::Admin);
}

#[test]
fn privilege_levels_that_do_not_exist_on_the_host_are_rejected() {
    let mut windows = Runner::execute().with_os(Os::Windows);
    let error = windows.run(&apt_install()).expect_err("no root on Windows");
    assert!(matches!(error, RunError::Unsupported { .. }), "{error}");

    let mut linux = Runner::execute().with_os(Os::Linux);
    let error = linux
        .run(&CommandSpec::new("dism").admin())
        .expect_err("no administrator steps on Linux");
    assert!(matches!(error, RunError::Unsupported { .. }), "{error}");
}

#[cfg(windows)]
#[test]
fn the_windows_elevator_refuses_arguments_cmd_would_reinterpret() {
    use rayx_cli::host::runner::WindowsElevator;

    // Rejected before any UAC prompt is raised.
    for bad in ["50%", "say \"hi\"", "two\nlines"] {
        let spec = CommandSpec::new("tool").arg(bad).admin();
        let error = WindowsElevator
            .run_elevated(&spec)
            .expect_err("refused before elevation");
        assert!(
            matches!(error, RunError::Unsupported { .. }),
            "{bad:?}: {error}"
        );
    }
}

#[test]
fn windows_quoting_follows_the_command_line_parsing_rules() {
    let runner = Runner::print_to(SharedBuffer::default()).with_os(Os::Windows);
    let line = |arg: &str| runner.command_line(&CommandSpec::new("tool").arg(arg));
    // Backslashes before the closing quote double so the quote is not read as escaped.
    assert_eq!(
        line(r"C:\Program Files\Tool\"),
        r#"tool "C:\Program Files\Tool\\""#
    );
    // Embedded quotes escape, and the backslashes before them double.
    assert_eq!(line(r#"say "hi""#), r#"tool "say \"hi\"""#);
    assert_eq!(line(r#"a\"b c"#), r#"tool "a\\\"b c""#);
    // Backslashes elsewhere stay as written.
    assert_eq!(line(r"C:\Work Dir\x"), r#"tool "C:\Work Dir\x""#);
    assert_eq!(line(r"C:\plain\path"), r"tool C:\plain\path");
}

#[cfg(unix)]
mod fake_sudo {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use rayx_cli::host::{CommandSpec, Os, RunError, Runner};

    /// Writes an executable `sudo` stand-in that logs its arguments, answers `-v` with
    /// `auth_exit`, and otherwise runs the command it was given, like the real one does.
    fn fake_sudo(dir: &Path, auth_exit: i32) -> (PathBuf, PathBuf) {
        let log = dir.join("sudo.log");
        let script = dir.join("fake-sudo");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\nif [ \"$1\" = \"-v\" ]; then exit {auth_exit}; fi\nexec \"$@\"\n",
                log.display()
            ),
        )
        .expect("write fake sudo");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        (script, log)
    }

    fn runner(script: &Path) -> Runner {
        Runner::execute()
            .with_os(Os::Linux)
            .with_root(false)
            .with_sudo_program(script.display().to_string())
    }

    #[test]
    fn root_steps_authenticate_then_run_through_sudo_with_env_and_cwd() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (script, log) = fake_sudo(dir.path(), 0);
        let work = dir.path().canonicalize().expect("canonical dir");
        let spec = CommandSpec::new("sh")
            .args(["-c", "printf '%s|%s' \"$FOO\" \"$(pwd -P)\""])
            .env("FOO", "bar")
            .cwd(&work)
            .root();

        let outcome = runner(&script).run(&spec).expect("approved");
        assert_eq!(outcome.stdout, format!("bar|{}", work.display()));

        let log = fs::read_to_string(log).expect("sudo log");
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines[0], "-v", "credentials are asked for first");
        assert!(
            lines[1].starts_with("env FOO=bar sh -c "),
            "the command goes through `sudo env K=V program args`: {}",
            lines[1]
        );
    }

    #[test]
    fn a_refused_sudo_prompt_stops_the_step_before_the_command_runs() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (script, log) = fake_sudo(dir.path(), 1);
        let marker = dir.path().join("ran");
        let spec = CommandSpec::new("touch")
            .arg(marker.display().to_string())
            .root();

        let error = runner(&script).run(&spec).expect_err("declined");
        assert!(matches!(error, RunError::Declined { .. }), "{error}");
        assert!(error.to_string().contains("did not run"), "{error}");
        assert!(!marker.exists(), "the command must not have run");
        assert_eq!(fs::read_to_string(log).expect("sudo log").trim(), "-v");
    }

    #[test]
    fn a_failing_command_after_authentication_is_an_ordinary_failure() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (script, _log) = fake_sudo(dir.path(), 0);
        let spec = CommandSpec::new("sh").args(["-c", "exit 3"]).root();
        let outcome = runner(&script).run(&spec).expect("ran");
        assert_eq!(outcome.code, Some(3));
    }

    #[test]
    fn print_mode_never_calls_sudo() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (script, log) = fake_sudo(dir.path(), 0);
        let mut runner = Runner::print_to(std::io::sink())
            .with_os(Os::Linux)
            .with_root(false)
            .with_sudo_program(script.display().to_string());
        runner
            .run(&CommandSpec::new("true").root())
            .expect("printed");
        assert!(!log.exists(), "sudo was not invoked");
    }
}
