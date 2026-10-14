//! `rayx setup --lima`: the command line, the host checks, Lima through Homebrew, the VM `rayx`
//! created once and started when stopped, and the guest steps through `limactl shell`.
//!
//! GitHub's macOS runners cannot run VMs: every `limactl`, `brew` and `sw_vers` answer here is
//! scripted through a recording runner.

use std::path::PathBuf;

use clap::Parser;
use rayx_cli::cli::{Cli, Command, SetupArgs};
use rayx_cli::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::guest::Checkout;
use rayx_cli::setup::lima::sizing::{Prompter, ScriptedPrompter};
use rayx_cli::setup::lima::{
    INSTANCE, LimaError, LimaRequest, VmFlags, detect_checkout, parse_instances,
    parse_lima_version, run_with,
};

const HOME: &str = "/Users/dev";

fn parse(args: &[&str]) -> Result<SetupArgs, clap::Error> {
    let mut argv = vec!["rayx", "setup"];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv).map(|cli| match cli.command {
        Command::Setup(args) => args,
        other => panic!("not setup: {other:?}"),
    })
}

fn macos() -> HostFacts {
    HostFacts {
        os: Os::MacOs,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn machine() -> FakeMachine {
    FakeMachine::new()
        .with_home(HOME)
        .with_program("brew", "/opt/homebrew/bin/brew")
        .with_program("limactl", "/opt/homebrew/bin/limactl")
}

fn request() -> LimaRequest {
    LimaRequest {
        flags: vec!["--web".to_string()],
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        release: None,
        vm: VmFlags::default(),
        check: false,
        yes: false,
    }
}

/// What the scripted Mac answers.
#[derive(Clone)]
struct Mac {
    macos: &'static str,
    /// `limactl --version` output; `None` is a Mac without Lima.
    lima: Option<&'static str>,
    /// `limactl list --json` output.
    instances: String,
}

impl Mac {
    fn new() -> Self {
        Self {
            macos: "14.5",
            lima: Some("limactl version 2.2.1"),
            instances: String::new(),
        }
    }

    fn with_vm(mut self, status: &str) -> Self {
        self.instances = instance_line(INSTANCE, status);
        self
    }

    fn runner(&self) -> Runner {
        let mac = self.clone();
        Runner::record().with_os(Os::MacOs).responder(move |spec| {
            match (spec.program.as_str(), spec.args.as_slice()) {
                ("sw_vers", _) => Some(Outcome::success().with_stdout(format!("{}\n", mac.macos))),
                ("sysctl", _) => Some(Outcome::success().with_stdout("10\n34359738368\n")),
                ("df", _) => Some(Outcome::success().with_stdout(
                    "Filesystem 1024-blocks Used Available Capacity iused ifree %iused Mounted on\n\
                     /dev/disk3s5 971350180 400000000 500000000 45% 1 2 0% /System/Volumes/Data\n",
                )),
                ("limactl", [flag]) if flag == "--version" => Some(match mac.lima {
                    Some(text) => Outcome::success().with_stdout(text),
                    None => Outcome::failure(127),
                }),
                ("limactl", [list, json]) if list == "list" && json == "--json" => {
                    Some(Outcome::success().with_stdout(mac.instances.clone()))
                }
                _ => None,
            }
        })
    }
}

fn instance_line(name: &str, status: &str) -> String {
    format!(
        "{{\"name\":\"{name}\",\"status\":\"{status}\",\"cpus\":6,\"memory\":12884901888,\
         \"disk\":107374182400,\"dir\":\"{HOME}/.lima/{name}\",\"vmType\":\"vz\"}}\n"
    )
}

fn run(
    request: &LimaRequest,
    runner: &mut Runner,
    checkout: Option<&Checkout>,
    answer: Option<bool>,
) -> Result<String, LimaError> {
    let mut out = Vec::new();
    let mut scripted = ScriptedPrompter::always(answer.unwrap_or(false));
    let prompter: Option<&mut dyn Prompter> = answer.map(|_| &mut scripted as _);
    run_with(
        request,
        &macos(),
        runner,
        &machine(),
        checkout,
        prompter,
        &mut out,
    )
    .map(|_| String::from_utf8(out).expect("UTF-8"))
}

fn lines(runner: &Runner) -> Vec<String> {
    runner.lines().to_vec()
}

/// The commands that change a VM or install software: everything but `list`, `--version` and the
/// probes run inside the guest.
fn changes(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|spec| {
            let first = spec.args.first().map(String::as_str);
            match spec.program.as_str() {
                "limactl" => !matches!(first, Some("list" | "--version" | "shell")),
                "sw_vers" | "sysctl" | "df" => false,
                _ => true,
            }
        })
        .map(|spec| format!("{} {}", spec.program, spec.args.join(" ")))
        .collect()
}

fn guest_scripts(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|spec| spec.program == "limactl" && spec.args.first().is_some_and(|a| a == "shell"))
        .filter_map(|spec| spec.args.last().cloned())
        .collect()
}

#[test]
fn the_command_line_accepts_lima_and_its_vm_flags() {
    let plain = parse(&["--lima"]).expect("--lima");
    assert!(plain.lima && !plain.wsl && !plain.all);
    assert_eq!(plain.vm_cpus, None);

    let full = parse(&[
        "--lima",
        "--web",
        "--clone",
        "--vm-cpus",
        "4",
        "--vm-memory",
        "8",
        "--vm-disk",
        "60",
        "--vm-home-writable",
    ])
    .expect("full");
    assert_eq!(full.clone.as_deref(), Some(""));
    assert_eq!(
        (
            full.vm_cpus,
            full.vm_memory,
            full.vm_disk,
            full.vm_home_writable
        ),
        (Some(4), Some(8), Some(60), true)
    );
    let named = parse(&["--lima", "--clone", "~/work/app"]).expect("named clone");
    assert_eq!(named.clone.as_deref(), Some("~/work/app"));
    assert!(parse(&["--wsl", "--clone"]).is_ok(), "--wsl keeps --clone");
}

#[test]
fn the_command_line_rejects_conflicting_and_misplaced_flags() {
    for (args, why) in [
        (vec!["--lima", "--wsl"], "--lima conflicts with --wsl"),
        (vec!["--clone"], "--clone needs --wsl or --lima"),
        (vec!["--vm-cpus", "4"], "--vm-cpus needs --lima"),
        (vec!["--vm-memory", "8"], "--vm-memory needs --lima"),
        (vec!["--vm-disk", "50"], "--vm-disk needs --lima"),
        (
            vec!["--vm-home-writable"],
            "--vm-home-writable needs --lima",
        ),
        (vec!["--wsl", "--vm-cpus", "4"], "--vm-cpus is not for WSL"),
        (
            vec!["--lima", "--vm-cpus", "0"],
            "a positive number of CPUs",
        ),
        (vec!["--lima", "--vm-memory", "x"], "a number"),
        (vec!["--lima", "--vm-disk", "-5"], "positive"),
    ] {
        assert!(parse(&args).is_err(), "{why}: {args:?}");
    }
}

#[test]
fn all_never_implies_lima_and_the_inner_flags_leave_the_vm_flags_out() {
    let all = parse(&["--all", "--yes"]).expect("--all");
    assert!(!all.lima);
    let lima = parse(&["--lima", "--web", "--vm-cpus", "4", "--yes"]).expect("lima");
    let request = LimaRequest::from_args(&lima, None);
    assert_eq!(request.flags, ["--web", "--yes"]);
    assert_eq!(request.vm.cpus, Some(4));
}

#[test]
fn off_macos_it_exits_before_running_anything() {
    for os in [Os::Windows, Os::Linux] {
        let host = HostFacts { os, ..macos() };
        let mut runner = Mac::new().runner();
        let error = run_with(
            &request(),
            &host,
            &mut runner,
            &machine(),
            None,
            None,
            &mut Vec::new(),
        )
        .expect_err("macOS only");
        assert_eq!(error, LimaError::NotMacOs);
        assert_eq!(
            error.to_string(),
            "`rayx setup --lima` creates a Lima VM on macOS"
        );
        assert!(runner.specs().is_empty(), "nothing ran on {os:?}");
    }
}

#[test]
fn an_old_macos_stops_before_homebrew_or_lima() {
    let mut mac = Mac::new();
    mac.macos = "12.7.4";
    let mut runner = mac.runner();

    let error = run(&request(), &mut runner, None, None).expect_err("too old");

    assert!(error.to_string().contains("macOS 13 or later"), "{error}");
    assert!(changes(&runner).is_empty());
    assert_eq!(runner.specs().len(), 1, "only sw_vers ran");
}

#[test]
fn missing_homebrew_asks_for_rayx_setup_first() {
    let mut runner = Mac::new().runner();
    let mut out = Vec::new();
    let error = run_with(
        &request(),
        &macos(),
        &mut runner,
        &FakeMachine::new().with_home(HOME),
        None,
        None,
        &mut out,
    )
    .expect_err("no brew");

    assert!(
        error
            .to_string()
            .contains("run `rayx setup` first to install Homebrew"),
        "{error}"
    );
    assert!(changes(&runner).is_empty());
}

#[test]
fn a_missing_lima_is_installed_with_homebrew_without_asking() {
    let mut mac = Mac::new();
    mac.lima = None;
    let mut runner = mac.runner();

    // `Some(false)`: a terminal that would answer no; a first install is never asked about.
    run(&request(), &mut runner, None, Some(false)).expect("installs lima");

    let changed = changes(&runner);
    assert_eq!(changed[0], "/opt/homebrew/bin/brew install lima");
    assert!(changed[1].starts_with("limactl create"), "{changed:?}");
    assert!(
        runner
            .specs()
            .iter()
            .all(|s| s.privilege == rayx_cli::host::Privilege::None),
        "no step needs sudo"
    );
}

#[test]
fn an_old_lima_is_an_update_asked_about_unless_yes() {
    let mut mac = Mac::new();
    mac.lima = Some("limactl version 1.2.1");

    // Declined: left alone, reported, nothing created.
    let mut runner = mac.runner();
    let error = run(&request(), &mut runner, None, Some(false)).expect_err("declined");
    assert!(matches!(error, LimaError::Declined(_)), "{error}");
    assert!(
        error.to_string().contains("brew upgrade lima") && error.to_string().contains("--yes"),
        "{error}"
    );
    assert!(changes(&runner).is_empty());

    // Unanswerable (a runner with no prompt at all behaves as --yes; the real command always
    // passes one that says no without a terminal).
    let mut runner = mac.runner();
    run(&request(), &mut runner, None, Some(true)).expect("accepted");
    assert_eq!(changes(&runner)[0], "/opt/homebrew/bin/brew upgrade lima");

    // --yes updates without asking.
    let mut runner = mac.runner();
    let mut yes = request();
    yes.yes = true;
    run(&yes, &mut runner, None, Some(false)).expect("yes");
    assert_eq!(changes(&runner)[0], "/opt/homebrew/bin/brew upgrade lima");
}

#[test]
fn a_current_lima_runs_no_brew_command() {
    let mut runner = Mac::new().with_vm("Running").runner();
    run(&request(), &mut runner, None, None).expect("runs");
    assert!(
        runner
            .specs()
            .iter()
            .all(|s| s.program != "/opt/homebrew/bin/brew"),
        "{:?}",
        lines(&runner)
    );
}

#[test]
fn an_absent_vm_is_created_read_only_with_the_pinned_ubuntu_and_started() {
    let mut runner = Mac::new().runner();

    let out = run(&request(), &mut runner, None, None).expect("creates");

    let changed = changes(&runner);
    assert_eq!(
        changed[0],
        "limactl create --tty=false --name=rayx --vm-type=vz --cpus=6 --memory=12 --disk=100 \
         --mount-only=/Users/dev --containerd=none template:ubuntu-24.04"
    );
    assert_eq!(changed[1], "limactl start --tty=false rayx");
    assert_eq!(changed.len(), 2, "{changed:?}");
    assert!(out.contains("creating the VM rayx"), "{out}");
}

#[test]
fn vm_flags_set_the_created_vm_and_a_writable_home_marks_the_mount() {
    let mut asked = request();
    asked.vm = VmFlags {
        cpus: Some(4),
        memory_gib: Some(8),
        disk_gib: Some(60),
        home_writable: true,
    };
    let mut runner = Mac::new().runner();

    run(&asked, &mut runner, None, None).expect("creates");

    assert_eq!(
        changes(&runner)[0],
        "limactl create --tty=false --name=rayx --vm-type=vz --cpus=4 --memory=8 --disk=60 \
         --mount-only=/Users/dev:w --containerd=none template:ubuntu-24.04"
    );
}

#[test]
fn a_stopped_vm_is_started_and_a_running_one_left_alone() {
    let mut stopped = Mac::new().with_vm("Stopped").runner();
    run(&request(), &mut stopped, None, None).expect("starts");
    assert_eq!(changes(&stopped), ["limactl start --tty=false rayx"]);

    let mut running = Mac::new().with_vm("Running").runner();
    run(&request(), &mut running, None, None).expect("runs");
    assert!(
        changes(&running).is_empty(),
        "a second run changes nothing: {:?}",
        changes(&running)
    );
}

#[test]
fn instances_rayx_did_not_create_are_never_touched() {
    let mut mac = Mac::new();
    mac.instances = format!(
        "{}{}",
        instance_line("gpux", "Stopped"),
        instance_line("rayx-22.04", "Stopped")
    );
    let mut runner = mac.runner();

    run(&request(), &mut runner, None, None).expect("creates rayx only");

    let all: Vec<String> = runner
        .specs()
        .iter()
        .map(|s| format!("{} {}", s.program, s.args.join(" ")))
        .collect();
    for line in &all {
        assert!(!line.contains("gpux"), "{line}");
        assert!(!line.contains("rayx-22.04"), "{line}");
    }
    assert_eq!(changes(&runner).len(), 2, "create and start rayx");
}

#[test]
fn the_guest_steps_run_through_limactl_shell_with_the_same_flags() {
    let mut runner = Mac::new().with_vm("Running").runner();

    let out = run(&request(), &mut runner, None, None).expect("runs");

    let shell: Vec<&CommandSpec> = runner
        .specs()
        .iter()
        .filter(|s| s.program == "limactl" && s.args.first().is_some_and(|a| a == "shell"))
        .collect();
    assert_eq!(shell.len(), 4, "version read, probe, installer, setup");
    for spec in &shell {
        assert_eq!(
            spec.args[..7],
            ["shell", "--workdir", "/", "rayx", "--", "bash", "-lc"]
        );
    }
    let scripts = guest_scripts(&runner);
    assert_eq!(scripts[0], ". /etc/os-release; echo $VERSION_ID");
    assert_eq!(scripts[1], "\"$HOME/.rayx/bin/rayx\" --version");
    assert!(
        scripts[2].contains(
            "https://github.com/rayx-ui/RayX.CLI/releases/download/v1.0.0/rayx-cli-installer.sh"
        ),
        "{}",
        scripts[2]
    );
    assert_eq!(scripts[3], "\"$HOME/.rayx/bin/rayx\" setup --web");
    assert!(out.contains("==> lima: rayx setup --web"), "{out}");
}

#[test]
fn clone_copies_the_checkout_under_the_home_and_runs_setup_in_it() {
    let checkout = Checkout {
        root: PathBuf::from("/Users/dev/work/RayX"),
        branch: Some("feat/x".into()),
    };
    let mut asked = request();
    asked.clone = Some(String::new());
    let mut runner = Mac::new().with_vm("Running").runner();

    run(&asked, &mut runner, Some(&checkout), None).expect("clones");

    let scripts = guest_scripts(&runner);
    let clone = scripts
        .iter()
        .find(|s| s.contains(" clone "))
        .expect("clone");
    assert!(
        clone.contains("clone -b 'feat/x' '/Users/dev/work/RayX' \"$HOME/RayX\""),
        "{clone}"
    );
    assert!(clone.contains("\"$HOME/.config/rayx/clones\""), "{clone}");
    assert_eq!(
        scripts.last().expect("setup"),
        "cd \"$HOME/RayX\" && \"$HOME/.rayx/bin/rayx\" setup --web"
    );
}

#[test]
fn a_checkout_outside_the_home_or_none_at_all_stops_the_clone() {
    let mut asked = request();
    asked.clone = Some(String::new());

    let outside = Checkout {
        root: PathBuf::from("/Volumes/Work/RayX"),
        branch: None,
    };
    let mut runner = Mac::new().with_vm("Running").runner();
    let error = run(&asked, &mut runner, Some(&outside), None).expect_err("outside the home");
    assert_eq!(
        error.to_string(),
        "--clone needs a checkout under /Users/dev, which the VM mounts"
    );
    assert!(guest_scripts(&runner).iter().all(|s| !s.contains("clone")));

    let mut runner = Mac::new().with_vm("Running").runner();
    let error = run(&asked, &mut runner, None, None).expect_err("no checkout");
    assert!(error.to_string().contains("git checkout"), "{error}");
}

#[test]
fn an_impossible_clone_is_refused_before_anything_is_installed_or_created() {
    let outside = Checkout {
        root: PathBuf::from("/Volumes/Work/RayX"),
        branch: None,
    };
    let inside = Checkout {
        root: PathBuf::from("/Users/dev/RayX"),
        branch: None,
    };
    for (requested, checkout) in [
        ("", None),
        ("", Some(&outside)),
        ("relative/dir", Some(&inside)),
        ("/srv/a\"b", Some(&inside)),
    ] {
        let mut asked = request();
        asked.clone = Some(requested.to_string());
        let mut mac = Mac::new();
        mac.lima = None; // even the Homebrew install waits for a clone that can succeed
        let mut runner = mac.runner();

        run(&asked, &mut runner, checkout, None).expect_err(requested);

        assert!(
            changes(&runner).is_empty(),
            "{requested}: {:?}",
            changes(&runner)
        );
        assert!(guest_scripts(&runner).is_empty(), "{requested}");
    }
}

#[test]
fn the_checkout_is_detected_only_for_a_clone_and_never_off_macos() {
    let mut asked = request();
    let windows = HostFacts {
        os: Os::Windows,
        ..macos()
    };

    let mut runner = Runner::record();
    assert_eq!(
        detect_checkout(&asked, &windows, &mut runner).expect_err("macOS only"),
        LimaError::NotMacOs
    );
    asked.clone = Some(String::new());
    assert_eq!(
        detect_checkout(&asked, &windows, &mut runner).expect_err("macOS only"),
        LimaError::NotMacOs
    );
    assert!(runner.specs().is_empty(), "no git, nothing, off macOS");

    let mut runner = Runner::record();
    asked.clone = None;
    assert_eq!(detect_checkout(&asked, &macos(), &mut runner), Ok(None));
    assert!(runner.specs().is_empty(), "no clone, no git");

    asked.clone = Some(String::new());
    let mut runner = Runner::record().respond(
        |spec| spec.program == "git" && spec.args.contains(&"--show-toplevel".to_string()),
        Outcome::success().with_stdout("/Users/dev/RayX\n"),
    );
    let found = detect_checkout(&asked, &macos(), &mut runner)
        .expect("macOS")
        .expect("a checkout");
    assert_eq!(found.root, PathBuf::from("/Users/dev/RayX"));
    assert!(runner.specs().iter().all(|s| s.program == "git"));
}

#[test]
fn an_override_binary_replaces_the_release_installer() {
    let mut asked = request();
    asked.binary_override = Some("/Users/dev/rayx-linux/rayx".into());
    let mut runner = Mac::new().with_vm("Running").runner();

    run(&asked, &mut runner, None, None).expect("copies");

    let scripts = guest_scripts(&runner);
    assert!(
        scripts[1].contains("cp \"/Users/dev/rayx-linux/rayx\" \"$HOME/.rayx/bin/rayx\""),
        "{}",
        scripts[1]
    );
    assert!(scripts.iter().all(|s| !s.contains("curl")), "{scripts:?}");
}

#[test]
fn a_limactl_missing_from_path_runs_with_homebrews_bin_first() {
    let machine = FakeMachine::new()
        .with_home(HOME)
        .with_program("brew", "/opt/homebrew/bin/brew");
    let mut runner = Mac::new().runner();

    run_with(
        &request(),
        &macos(),
        &mut runner,
        &machine,
        None,
        None,
        &mut Vec::new(),
    )
    .expect("runs");

    let limactl: Vec<&CommandSpec> = runner
        .specs()
        .iter()
        .filter(|s| s.program == "limactl")
        .collect();
    assert!(!limactl.is_empty());
    for spec in limactl {
        let path = spec
            .env
            .iter()
            .find(|(key, _)| key == "PATH")
            .map(|(_, value)| value.as_str())
            .unwrap_or_default();
        assert!(path.starts_with("/opt/homebrew/bin:"), "{spec:?}: {path}");
    }
}

#[test]
fn limactl_output_is_read_one_object_per_line() {
    let text = format!(
        "{}{}not json\n",
        instance_line("rayx", "Running"),
        instance_line("gpux", "Stopped")
    );
    let instances = parse_instances(&text);
    assert_eq!(instances.len(), 2);
    assert!(instances[0].is_running() && !instances[1].is_running());
    assert_eq!(
        (
            instances[0].cpus,
            instances[0].memory_gib(),
            instances[0].disk_gib()
        ),
        (6, 12, 100)
    );
    assert_eq!(instances[0].dir, "/Users/dev/.lima/rayx");
    assert_eq!(parse_instances("").len(), 0);
    assert_eq!(
        parse_instances(&format!("[{}]", instance_line("a", "Stopped").trim())).len(),
        1
    );

    assert_eq!(
        parse_lima_version("limactl version 2.2.1").as_deref(),
        Some("2.2.1")
    );
    assert_eq!(
        parse_lima_version("limactl version v1.2.1-12-gabc").as_deref(),
        Some("1.2.1")
    );
    assert_eq!(
        parse_lima_version("limactl version 2.0.0-beta.1").as_deref(),
        Some("2.0.0")
    );
    assert_eq!(parse_lima_version("nothing"), None);
}

/// `--check` end to end against stand-in tools: the commands are printed, the create and start
/// lines among them, and the only `limactl` calls that run are reads.
#[cfg(target_os = "macos")]
#[test]
fn check_prints_every_command_and_runs_only_reads() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let log = dir.path().join("calls.log");
    let script = |name: &str, body: &str| {
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    };
    script("sw_vers", "echo 14.5");
    script("brew", "exit 0");
    script(
        "limactl",
        &format!(
            "echo \"$@\" >> '{log}'\n\
             case \"$1\" in --version) echo 'limactl version 2.2.1';; \
             list) echo '{{\"name\":\"gpux\",\"status\":\"Stopped\"}}';; *) exit 1;; esac",
            log = log.display()
        ),
    );

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rayx"))
        .args(["setup", "--lima", "--check", "--web"])
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
        )
        .env("HOME", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("runs rayx");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("limactl create --tty=false --name=rayx"),
        "{stdout}"
    );
    assert!(
        stdout.contains("limactl start --tty=false rayx"),
        "{stdout}"
    );
    assert!(stdout.contains("rayx-cli-installer.sh"), "{stdout}");
    assert!(
        stdout.contains("\"$HOME/.rayx/bin/rayx\" setup --web --check"),
        "{stdout}"
    );
    assert!(
        !dir.path().join(".config/rayx").exists(),
        "--check records no guest"
    );
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    for call in calls.lines() {
        assert!(
            call.starts_with("--version")
                || call.starts_with("list ")
                || call.starts_with("shell "),
            "a --check run executed `limactl {call}`"
        );
        assert!(!call.contains("gpux") || call.starts_with("list"), "{call}");
    }
}
