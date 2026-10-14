//! `rayx setup --wsl`: the Linux `rayx` is installed with the release's own versioned installer, so
//! it lives in `~/.rayx/bin` with a receipt and `rayx self update` works inside WSL.

use std::path::PathBuf;

use rayx_cli::cli::{Cli, Command};
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::wsl::{LINUX_BINARY, WslError, WslRequest, distribution, run_with};
use rayx_cli::wsl::Distribution;

use clap::Parser;

const INSTALLER: &str =
    "https://github.com/rayx-ui/RayX.CLI/releases/download/v1.0.0/rayx-cli-installer.sh";

fn windows(arch: Arch) -> HostFacts {
    HostFacts {
        os: Os::Windows,
        arch,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn machine() -> FakeMachine {
    FakeMachine::new().with_wsl_distributions(vec![Distribution {
        id: "{abc}".into(),
        name: distribution(),
        base_path: PathBuf::from(r"C:\Users\dev\AppData\Local\wsl\ubuntu"),
        version: Some(2),
        default_uid: Some(1000),
    }])
}

fn request() -> WslRequest {
    WslRequest {
        flags: vec!["--web".to_string()],
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        release: None,
        windows_gpu: None,
    }
}

/// A runner that answers `wsl --version` like WSL 2 (UTF-16 output arrives with NUL bytes).
fn wsl2_runner() -> Runner {
    Runner::record().with_os(Os::Windows).respond(
        |spec| spec.program == "wsl" && spec.args == ["--version"],
        Outcome::success().with_stdout(
            "WSL version: 2.6.1"
                .chars()
                .flat_map(|c| [c, '\0'])
                .collect::<String>(),
        ),
    )
}

/// The runner answers the Linux `rayx --version` probe with `output`.
fn with_installed(runner: Runner, output: &'static str) -> Runner {
    runner.respond(
        |spec| {
            spec.args
                .last()
                .is_some_and(|a| a.ends_with("rayx\" --version"))
        },
        Outcome::success().with_stdout(output),
    )
}

fn run(request: &WslRequest, host: &HostFacts, runner: &mut Runner) -> Result<String, WslError> {
    let mut out = Vec::new();
    run_with(request, host, runner, &machine(), None, &mut out)
        .map(|()| String::from_utf8(out).expect("UTF-8"))
}

/// The scripts each `wsl --exec bash -lc` received, in order.
fn scripts(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|spec| spec.program == "wsl" && spec.args.contains(&"bash".to_string()))
        .filter_map(|spec| spec.args.last().cloned())
        .collect()
}

#[test]
fn the_versioned_installer_is_piped_to_sh_through_exec_bash() {
    let mut runner = wsl2_runner();

    run(&request(), &windows(Arch::X64), &mut runner).expect("sets up");

    let install = runner
        .specs()
        .iter()
        .find(|spec| spec.args.last().is_some_and(|a| a.contains("curl")))
        .expect("an install command");
    assert_eq!(
        install.args[..5],
        ["-d", &distribution(), "--exec", "bash", "-lc"],
        "scripts run with --exec so the login shell does not parse them"
    );
    let script = install.args.last().expect("script");
    assert!(
        script.contains(&format!(
            "curl --proto '=https' --tlsv1.2 -LsSf '{INSTALLER}' | "
        )),
        "{script}"
    );
    assert!(script.contains("| env -u RAYX_HOME sh"), "{script}");
    assert!(
        script.starts_with("set -o pipefail;"),
        "a failed download must fail the step: {script}"
    );
    assert!(
        script.ends_with(&format!("test -x \"{LINUX_BINARY}\"")),
        "the step checks the binary landed: {script}"
    );
    for old in [".local/bin", "tar ", "tar.xz"] {
        assert!(
            !script.contains(old),
            "no hand extraction ({old}): {script}"
        );
    }
    assert!(install.interactive, "the installer talks to this console");
}

#[test]
fn the_binary_lives_in_the_installers_directory_for_the_probe_and_the_inner_setup() {
    assert_eq!(LINUX_BINARY, "$HOME/.rayx/bin/rayx");
    let mut runner = wsl2_runner();

    run(&request(), &windows(Arch::X64), &mut runner).expect("sets up");

    let scripts = scripts(&runner);
    let probe = scripts
        .iter()
        .find(|s| s.contains("--version"))
        .expect("a version probe");
    assert_eq!(probe, "\"$HOME/.rayx/bin/rayx\" --version");
    assert_eq!(
        scripts.last().expect("setup runs last"),
        "\"$HOME/.rayx/bin/rayx\" setup --web"
    );
}

#[test]
fn a_matching_version_is_not_reinstalled() {
    let mut runner = with_installed(wsl2_runner(), "rayx 1.0.0\n");

    run(&request(), &windows(Arch::X64), &mut runner).expect("sets up");

    assert!(
        !scripts(&runner).iter().any(|s| s.contains("curl")),
        "{:?}",
        scripts(&runner)
    );
    assert!(
        scripts(&runner)
            .last()
            .expect("setup")
            .contains("setup --web")
    );
}

#[test]
fn another_installed_version_is_replaced_by_the_running_versions_installer() {
    let mut runner = with_installed(wsl2_runner(), "rayx 0.9.0\n");

    run(&request(), &windows(Arch::X64), &mut runner).expect("sets up");

    assert!(
        scripts(&runner)
            .iter()
            .any(|s| s.contains(&format!("'{INSTALLER}'"))),
        "{:?}",
        scripts(&runner)
    );
}

#[test]
fn an_architecture_without_a_linux_release_fails_before_any_download() {
    let mut runner = wsl2_runner();

    let error = run(&request(), &windows(Arch::Other), &mut runner).expect_err("no release");

    assert_eq!(
        error,
        WslError::Failed("this CPU architecture has no Linux rayx release".to_string())
    );
    let scripts = scripts(&runner);
    assert!(
        scripts
            .iter()
            .all(|s| !s.contains("curl") && !s.contains("cp ") && !s.contains("setup")),
        "nothing is installed or run: {scripts:?}"
    );
}

#[test]
fn a_development_binary_is_copied_to_the_installers_directory_without_a_download() {
    let mut runner = wsl2_runner();
    let mut asked = request();
    asked.binary_override = Some(PathBuf::from(r"C:\Work\rayx build\rayx"));

    run(&asked, &windows(Arch::X64), &mut runner).expect("sets up");

    let scripts = scripts(&runner);
    let copy = scripts
        .iter()
        .find(|s| s.contains("cp "))
        .expect("a copy command");
    assert!(
        copy.contains("mkdir -p \"$HOME/.rayx/bin\" && cp \"/mnt/c/Work/rayx build/rayx\" \"$HOME/.rayx/bin/rayx\""),
        "{copy}"
    );
    assert!(
        scripts
            .iter()
            .all(|s| !s.contains("curl") && !s.contains(".local/bin")),
        "{scripts:?}"
    );
}

#[test]
fn everything_that_changes_the_distribution_is_interactive_so_print_mode_runs_none_of_it() {
    // In `--check` the runner prints instead of running `run`/`run_checked`; only `query` (which
    // refuses interactive commands) runs for real. The installer, the copy and the inner setup are
    // all interactive, so none of them can run in print mode.
    let mut runner = wsl2_runner();
    run(&request(), &windows(Arch::X64), &mut runner).expect("sets up");
    let mut copy = request();
    copy.binary_override = Some(PathBuf::from(r"C:\x\rayx"));
    let mut copy_runner = wsl2_runner();
    run(&copy, &windows(Arch::X64), &mut copy_runner).expect("sets up");

    for spec in runner.specs().iter().chain(copy_runner.specs()) {
        let script = spec.args.last().map_or("", String::as_str);
        let changes = script.contains("curl")
            || script.contains("cp ")
            || script.contains(" setup")
            || script.contains("--install");
        if changes {
            assert!(spec.interactive, "{script} must not be a read-only query");
        }
    }
}

#[test]
fn check_is_passed_through_to_the_inner_setup() {
    let cli = Cli::parse_from(["rayx", "setup", "--wsl", "--check"]);
    let Command::Setup(args) = cli.command else {
        panic!("setup command");
    };
    let flags = WslRequest::from_args(&args, None).flags;
    assert_eq!(flags, ["--check"]);
}
