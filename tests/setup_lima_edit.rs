//! Explicit `--vm-*` flags on an existing VM `rayx`: the differences are an update, asked about
//! like any outdated requirement, applied as stop (when running), `limactl edit`, start.

use std::path::Path;

use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::lima::edit::home_is_writable;
use rayx_cli::setup::lima::sizing::{Prompter, ScriptedPrompter};
use rayx_cli::setup::lima::{LimaError, LimaOutcome, LimaRequest, VmFlags, run_with};

const GIB: u64 = 1024 * 1024 * 1024;
const HOME: &str = "/Users/dev";
const READ_ONLY: &str = "mounts:\n- location: \"/Users/dev\"\n  mountPoint: \"/Users/dev\"\n";
const WRITABLE: &str =
    "mounts:\n- location: \"~\"\n  writable: true\n- location: /tmp/lima\n  writable: true\n";

fn instance(name: &str, status: &str, cpus: u32, memory_gib: u64, disk_gib: u64) -> String {
    format!(
        "{{\"name\":\"{name}\",\"status\":\"{status}\",\"cpus\":{cpus},\"memory\":{},\"disk\":{},\"dir\":\"/Users/dev/.lima/{name}\"}}\n",
        memory_gib * GIB,
        disk_gib * GIB
    )
}

fn runner(instances: String) -> Runner {
    Runner::record().with_os(Os::MacOs).responder(move |spec| {
        match (spec.program.as_str(), spec.args.as_slice()) {
            ("sw_vers", _) => Some(Outcome::success().with_stdout("14.5\n")),
            ("sysctl", _) => Some(Outcome::success().with_stdout("10\n34359738368\n")),
            ("df", _) => Some(Outcome::success().with_stdout("h\n/dev/d 1 1 500000000 1% /\n")),
            ("limactl", [flag]) if flag == "--version" => {
                Some(Outcome::success().with_stdout("limactl version 2.2.1"))
            }
            ("limactl", [list, json]) if list == "list" && json == "--json" => {
                Some(Outcome::success().with_stdout(instances.clone()))
            }
            _ => None,
        }
    })
}

fn request(vm: VmFlags) -> LimaRequest {
    LimaRequest {
        flags: Vec::new(),
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        vm,
        check: false,
        yes: false,
    }
}

fn flags() -> VmFlags {
    VmFlags::default()
}

fn run(
    request: &LimaRequest,
    instances: String,
    yaml: &str,
    prompter: Option<&mut dyn Prompter>,
) -> (Result<LimaOutcome, LimaError>, Vec<String>, String) {
    let mut runner = runner(instances);
    let machine = FakeMachine::new()
        .with_home(HOME)
        .with_program("brew", "/opt/homebrew/bin/brew")
        .with_program("limactl", "/opt/homebrew/bin/limactl")
        .with_text("/Users/dev/.lima/rayx/lima.yaml", yaml);
    let host = HostFacts {
        os: Os::MacOs,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    };
    let mut out = Vec::new();
    let result = run_with(
        request,
        &host,
        &mut runner,
        &machine,
        None,
        prompter,
        &mut out,
    );
    let changes = runner
        .specs()
        .iter()
        .filter(|s| s.program == "limactl")
        .filter(|s| {
            !matches!(
                s.args.first().map(String::as_str),
                Some("list" | "--version" | "shell")
            )
        })
        .map(|s| s.args.join(" "))
        .collect();
    (result, changes, String::from_utf8(out).expect("UTF-8"))
}

#[test]
fn a_running_vm_is_stopped_edited_and_started_after_one_question() {
    let vm = VmFlags {
        cpus: Some(8),
        ..flags()
    };
    let mut prompter = ScriptedPrompter::always(true);

    let (result, changes, out) = run(
        &request(vm),
        instance("rayx", "Running", 6, 12, 100),
        READ_ONLY,
        Some(&mut prompter),
    );

    result.expect("applies");
    assert_eq!(
        changes,
        [
            "stop rayx",
            "edit --tty=false rayx --cpus=8",
            "start --tty=false rayx"
        ]
    );
    assert_eq!(
        prompter.asked,
        ["Change the VM rayx (6 -> 8 CPUs). Apply now?"]
    );
    assert!(out.contains("changing the VM rayx: 6 -> 8 CPUs"), "{out}");
}

#[test]
fn a_stopped_vm_is_edited_without_a_stop_and_every_changed_flag_is_passed() {
    let vm = VmFlags {
        cpus: Some(4),
        memory_gib: Some(16),
        disk_gib: Some(150),
        home_writable: true,
    };
    let mut prompter = ScriptedPrompter::always(true);

    let (result, changes, out) = run(
        &request(vm),
        instance("rayx", "Stopped", 6, 12, 100),
        READ_ONLY,
        Some(&mut prompter),
    );

    result.expect("applies");
    assert_eq!(
        changes,
        [
            "edit --tty=false rayx --cpus=4 --memory=16 --disk=150 --mount-only=/Users/dev:w",
            "start --tty=false rayx"
        ]
    );
    assert!(
        out.contains(
            "6 -> 4 CPUs, 12 -> 16 GiB memory, 100 -> 150 GiB disk, home read-only -> writable"
        ),
        "{out}"
    );
}

#[test]
fn flags_equal_to_the_vm_change_nothing_and_ask_nothing() {
    let vm = VmFlags {
        cpus: Some(6),
        memory_gib: Some(12),
        disk_gib: Some(100),
        home_writable: true,
    };
    let mut prompter = ScriptedPrompter::always(true);

    let (result, changes, _) = run(
        &request(vm),
        instance("rayx", "Running", 6, 12, 100),
        WRITABLE,
        Some(&mut prompter),
    );

    result.expect("nothing to do");
    assert!(changes.is_empty(), "{changes:?}");
    assert!(prompter.asked.is_empty());

    // No flags at all: a re-run asks and changes nothing either.
    let (result, changes, _) = run(
        &request(flags()),
        instance("rayx", "Running", 2, 2, 30),
        READ_ONLY,
        Some(&mut ScriptedPrompter::always(true)),
    );
    result.expect("nothing to do");
    assert!(changes.is_empty());
}

#[test]
fn a_smaller_disk_is_refused_and_nothing_runs() {
    let vm = VmFlags {
        cpus: Some(8),
        disk_gib: Some(60),
        ..flags()
    };
    let mut prompter = ScriptedPrompter::always(true);

    let (result, changes, _) = run(
        &request(vm),
        instance("rayx", "Running", 6, 12, 100),
        READ_ONLY,
        Some(&mut prompter),
    );

    let message = result.expect_err("refused").to_string();
    assert!(
        message.contains("Lima cannot shrink a disk") && message.contains("100 GiB"),
        "{message}"
    );
    assert!(
        changes.is_empty(),
        "the cpu change is not applied either: {changes:?}"
    );
    assert!(prompter.asked.is_empty());
}

#[test]
fn a_declined_or_unanswerable_update_changes_nothing_and_reports() {
    let vm = VmFlags {
        memory_gib: Some(8),
        ..flags()
    };

    let mut declined = ScriptedPrompter::always(false);
    let (result, changes, _) = run(
        &request(vm),
        instance("rayx", "Running", 6, 12, 100),
        READ_ONLY,
        Some(&mut declined),
    );
    let error = result.expect_err("declined");
    assert!(matches!(error, LimaError::Declined(_)), "{error}");
    assert!(
        error.to_string().contains("12 -> 8 GiB memory") && error.to_string().contains("--yes")
    );
    assert!(changes.is_empty());

    // No terminal and no --yes: reported, not applied, even if the script would say yes.
    let mut no_terminal = ScriptedPrompter::always(true);
    no_terminal.terminal = false;
    let (result, changes, _) = run(
        &request(vm),
        instance("rayx", "Running", 6, 12, 100),
        READ_ONLY,
        Some(&mut no_terminal),
    );
    assert!(matches!(result, Err(LimaError::Declined(_))));
    assert!(changes.is_empty());
    assert!(no_terminal.asked.is_empty(), "nobody can answer");
}

#[test]
fn yes_applies_without_asking() {
    let mut asked = request(VmFlags {
        memory_gib: Some(8),
        ..flags()
    });
    asked.yes = true;
    let mut prompter = ScriptedPrompter::always(false);

    let (result, changes, _) = run(
        &asked,
        instance("rayx", "Running", 6, 12, 100),
        READ_ONLY,
        Some(&mut prompter),
    );

    result.expect("applies");
    assert_eq!(changes[1], "edit --tty=false rayx --memory=8");
    assert!(prompter.asked.is_empty());
}

#[test]
fn only_the_instance_named_rayx_is_edited() {
    let instances = format!(
        "{}{}",
        instance("gpux", "Running", 4, 8, 60),
        instance("rayx", "Stopped", 6, 12, 100)
    );
    let vm = VmFlags {
        cpus: Some(2),
        ..flags()
    };

    let (result, changes, _) = run(
        &request(vm),
        instances,
        READ_ONLY,
        Some(&mut ScriptedPrompter::always(true)),
    );

    result.expect("applies");
    assert_eq!(
        changes,
        ["edit --tty=false rayx --cpus=2", "start --tty=false rayx"]
    );
}

#[test]
fn a_flag_above_the_cap_is_refused_before_the_vm_is_read() {
    let vm = VmFlags {
        cpus: Some(9),
        ..flags()
    };

    let (result, changes, _) = run(
        &request(vm),
        instance("rayx", "Running", 6, 12, 100),
        READ_ONLY,
        None,
    );

    assert!(
        result
            .expect_err("refused")
            .to_string()
            .contains("choose 1 to 8 CPUs")
    );
    assert!(changes.is_empty());
}

#[test]
fn home_writability_is_read_from_the_lima_yaml_mounts() {
    let home = Path::new(HOME);
    assert!(
        !home_is_writable(READ_ONLY, home),
        "writable absent means read-only"
    );
    assert!(home_is_writable(WRITABLE, home), "`~` is the home");
    assert!(home_is_writable(
        "mounts:\n- location: /Users/dev/\n  writable: true\n",
        home
    ));
    assert!(!home_is_writable(
        "mounts:\n- location: /tmp/lima\n  writable: true\n",
        home
    ));
    assert!(!home_is_writable(
        "mounts:\n- location: /Users/dev\n  writable: false\n",
        home
    ));
    assert!(
        !home_is_writable("mounts: [", home),
        "unparsable is read-only"
    );
    assert!(!home_is_writable("", home));
}
