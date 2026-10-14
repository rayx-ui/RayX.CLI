//! The size of the VM `rayx`: defaults capped to what the Mac can spare, flags validated against
//! the same limits, free-space and memory warnings, and the one `Customize? [y/N]` question.

use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::lima::sizing::{
    HostCapacity, Prompter, ScriptedPrompter, customize, effective, memory_warning,
    parse_df_available, read_capacity,
};
use rayx_cli::setup::lima::{
    Instance, LimaError, LimaOutcome, LimaRequest, VmFlags, VmSettings, parse_instances, run_with,
};

const GIB: u64 = 1024 * 1024 * 1024;
const HOME: &str = "/Users/dev";

fn host(cpus: u32, memory_gib: u64, free_gib: u64) -> HostCapacity {
    HostCapacity {
        cpus,
        memory_bytes: memory_gib * GIB,
        free_kib: free_gib * 1024 * 1024,
    }
}

fn flags() -> VmFlags {
    VmFlags::default()
}

fn error_text(result: Result<impl std::fmt::Debug, LimaError>) -> String {
    result.expect_err("refused").to_string()
}

#[test]
fn the_caps_leave_two_cpus_and_half_the_memory_to_the_mac() {
    for (cpus, cap) in [(1, 1), (2, 1), (3, 1), (4, 2), (8, 6), (10, 8)] {
        assert_eq!(host(cpus, 16, 100).cpu_cap(), cap, "{cpus} CPUs");
    }
    for (memory, cap) in [(2, 2), (4, 2), (8, 4), (16, 8), (24, 12), (64, 32)] {
        assert_eq!(host(8, memory, 100).memory_cap_gib(), cap, "{memory} GiB");
    }
}

#[test]
fn defaults_above_the_cap_are_lowered_with_a_note_naming_the_mac() {
    let defaults = VmSettings::default();

    let small = effective(&defaults, &host(8, 8, 200), &flags()).expect("8 GiB");
    assert_eq!(
        (
            small.settings.cpus,
            small.settings.memory_gib,
            small.settings.disk_gib
        ),
        (6, 4, 100)
    );
    assert_eq!(
        small.notes,
        ["this Mac has 8 GiB memory: the VM gets 4 GiB"]
    );

    let middle = effective(&defaults, &host(10, 16, 200), &flags()).expect("16 GiB");
    assert_eq!(middle.settings.memory_gib, 8);
    assert_eq!(
        middle.notes,
        ["this Mac has 16 GiB memory: the VM gets 8 GiB"]
    );

    let large = effective(&defaults, &host(10, 24, 200), &flags()).expect("24 GiB");
    assert_eq!(large.settings, defaults, "nothing to cap");
    assert!(large.notes.is_empty());

    let few = effective(&defaults, &host(4, 24, 200), &flags()).expect("4 cores");
    assert_eq!(few.settings.cpus, 2);
    assert_eq!(few.notes, ["this Mac has 4 CPUs: the VM gets 2 CPUs"]);
}

#[test]
fn explicit_values_replace_the_defaults_within_the_limits() {
    let asked = VmFlags {
        cpus: Some(2),
        memory_gib: Some(3),
        disk_gib: Some(30),
        home_writable: true,
    };
    let sized = effective(&VmSettings::default(), &host(8, 8, 200), &asked).expect("fits");
    assert_eq!(
        sized.settings,
        VmSettings {
            cpus: 2,
            memory_gib: 3,
            disk_gib: 30,
            home_writable: true
        }
    );
    assert!(
        sized.notes.is_empty(),
        "explicit values are not capped or noted"
    );
}

#[test]
fn explicit_values_outside_the_limits_are_refused_with_the_allowed_range() {
    let capacity = host(8, 8, 200);
    let defaults = VmSettings::default();
    for (asked, expect) in [
        (
            VmFlags {
                cpus: Some(7),
                ..flags()
            },
            "choose 1 to 6 CPUs",
        ),
        (
            VmFlags {
                cpus: Some(0),
                ..flags()
            },
            "choose 1 to 6 CPUs",
        ),
        (
            VmFlags {
                memory_gib: Some(5),
                ..flags()
            },
            "choose 2 to 4 GiB memory",
        ),
        (
            VmFlags {
                memory_gib: Some(1),
                ..flags()
            },
            "choose 2 to 4 GiB memory",
        ),
        (
            VmFlags {
                disk_gib: Some(29),
                ..flags()
            },
            "at least 30 GiB",
        ),
    ] {
        let message = error_text(effective(&defaults, &capacity, &asked));
        assert!(message.contains(expect), "{message}");
    }
    let message = error_text(effective(
        &defaults,
        &capacity,
        &VmFlags {
            memory_gib: Some(12),
            ..flags()
        },
    ));
    assert!(
        message.contains("--vm-memory 12") && message.contains("8 GiB memory"),
        "{message}"
    );
}

#[test]
fn the_mac_is_read_with_sysctl_and_df_on_the_lima_home_volume() {
    let mut runner = Runner::record().responder(|spec| match spec.program.as_str() {
        "sysctl" => Some(Outcome::success().with_stdout("10\n17179869184\n")),
        "df" => Some(Outcome::success().with_stdout(
            "Filesystem 1024-blocks Used Available Capacity iused ifree %iused Mounted on\n\
             /dev/disk3s5 971350180 400000000 123456789 45% 1 2 0% /System/Volumes/Data\n",
        )),
        _ => None,
    });
    let machine = FakeMachine::new().with_dir("/Users/dev/.lima");

    let capacity = read_capacity(&mut runner, &machine, std::path::Path::new(HOME)).expect("reads");

    assert_eq!(capacity.cpus, 10);
    assert_eq!(capacity.memory_gib(), 16);
    assert_eq!(capacity.free_kib, 123_456_789);
    let lines = runner.lines();
    assert!(
        lines[0].contains("sysctl -n hw.ncpu hw.memsize"),
        "{lines:?}"
    );
    assert!(lines[1].ends_with("df -k /Users/dev/.lima"), "{lines:?}");

    let mut elsewhere = Runner::record().responder(|spec| match spec.program.as_str() {
        "sysctl" => Some(Outcome::success().with_stdout("8\n8589934592\n")),
        "df" => Some(Outcome::success().with_stdout("h\n/dev/x 1 1 100 1% /\n")),
        _ => None,
    });
    let machine = FakeMachine::new()
        .with_env("LIMA_HOME", "/Volumes/Fast/lima")
        .with_dir("/Volumes/Fast/lima");
    let _ = read_capacity(&mut elsewhere, &machine, std::path::Path::new(HOME));
    assert!(
        elsewhere.lines()[1].ends_with("df -k /Volumes/Fast/lima"),
        "LIMA_HOME wins"
    );

    assert_eq!(
        parse_df_available("Filesystem 1K Used Available\n/dev/x 10 4 6 40%\n"),
        Some(6)
    );
    assert_eq!(parse_df_available(""), None);
}

fn request() -> LimaRequest {
    LimaRequest {
        flags: Vec::new(),
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        vm: VmFlags::default(),
        check: false,
        yes: false,
    }
}

struct Mac {
    cpus: u32,
    memory_gib: u64,
    free_gib: u64,
    instances: String,
}

impl Mac {
    fn new(cpus: u32, memory_gib: u64, free_gib: u64) -> Self {
        Self {
            cpus,
            memory_gib,
            free_gib,
            instances: String::new(),
        }
    }

    fn runner(&self) -> Runner {
        let (cpus, memory, free, instances) = (
            self.cpus,
            self.memory_gib * GIB,
            self.free_gib * 1024 * 1024,
            self.instances.clone(),
        );
        Runner::record().with_os(Os::MacOs).responder(move |spec| {
            match (spec.program.as_str(), spec.args.as_slice()) {
                ("sw_vers", _) => Some(Outcome::success().with_stdout("14.5\n")),
                ("sysctl", _) => Some(Outcome::success().with_stdout(format!("{cpus}\n{memory}\n"))),
                ("df", _) => Some(Outcome::success().with_stdout(format!(
                    "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/d 1 1 {free} 1% /\n"
                ))),
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
}

fn running(name: &str, memory_gib: u64) -> String {
    format!(
        "{{\"name\":\"{name}\",\"status\":\"Running\",\"cpus\":4,\"memory\":{},\"disk\":107374182400,\"dir\":\"/x\"}}\n",
        memory_gib * GIB
    )
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

fn create(
    request: &LimaRequest,
    mac: &Mac,
    prompter: Option<&mut dyn Prompter>,
) -> (Result<LimaOutcome, LimaError>, Runner, String) {
    let mut runner = mac.runner();
    let machine = FakeMachine::new()
        .with_home(HOME)
        .with_program("brew", "/opt/homebrew/bin/brew")
        .with_program("limactl", "/opt/homebrew/bin/limactl");
    let mut out = Vec::new();
    let result = run_with(
        request,
        &macos(),
        &mut runner,
        &machine,
        None,
        prompter,
        &mut out,
    );
    (result, runner, String::from_utf8(out).expect("UTF-8"))
}

fn create_line(runner: &Runner) -> Option<String> {
    runner
        .specs()
        .iter()
        .find(|s| s.program == "limactl" && s.args.first().is_some_and(|a| a == "create"))
        .map(|s| s.args.join(" "))
}

fn limactl_changes(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|s| s.program == "limactl" || s.program.ends_with("brew"))
        .filter(|s| {
            !matches!(
                s.args.first().map(String::as_str),
                Some("list" | "--version" | "shell")
            )
        })
        .map(|s| format!("{} {}", s.program, s.args.join(" ")))
        .collect()
}

#[test]
fn a_flag_outside_the_limits_exits_before_any_limactl_command() {
    let mut asked = request();
    asked.vm.memory_gib = Some(12);
    let (result, runner, _) = create(&asked, &Mac::new(8, 8, 200), None);

    let message = result.expect_err("refused").to_string();

    assert!(message.contains("choose 2 to 4 GiB memory"), "{message}");
    assert!(
        runner
            .specs()
            .iter()
            .all(|s| !matches!(s.program.as_str(), "limactl") && !s.program.ends_with("brew")),
        "no limactl or brew command: {:?}",
        runner.lines()
    );
}

#[test]
fn a_small_mac_creates_the_vm_with_the_capped_defaults_and_says_so() {
    let (result, runner, out) = create(&request(), &Mac::new(8, 8, 200), None);

    result.expect("creates");

    let line = create_line(&runner).expect("create");
    assert!(line.contains("--cpus=6 --memory=4 --disk=100"), "{line}");
    assert!(
        out.contains("note: this Mac has 8 GiB memory: the VM gets 4 GiB"),
        "{out}"
    );
}

#[test]
fn fewer_than_ten_gib_free_stops_and_less_than_the_disk_only_notes() {
    let (result, runner, _) = create(&request(), &Mac::new(10, 24, 9), None);
    let message = result.expect_err("no room").to_string();
    assert!(message.contains("only 9 GiB are free"), "{message}");
    assert!(create_line(&runner).is_none());

    let (result, runner, out) = create(&request(), &Mac::new(10, 24, 60), None);
    result.expect("sparse disk");
    assert!(create_line(&runner).is_some());
    assert!(
        out.contains("60 GiB are free") && out.contains("sparse"),
        "{out}"
    );
}

#[test]
fn running_vms_over_three_quarters_of_the_memory_are_named_in_a_warning() {
    let settings = VmSettings::default();
    let capacity = host(10, 16, 200);
    let instances = parse_instances(&format!("{}{}", running("gpux", 10), {
        "{\"name\":\"idle\",\"status\":\"Stopped\",\"memory\":17179869184}\n"
    }));
    let warning = memory_warning(&instances, &settings, &capacity, "rayx").expect("warns");
    assert!(
        warning.contains("gpux (10 GiB)") && !warning.contains("idle"),
        "{warning}"
    );
    assert!(warning.contains("rayx (12 GiB)"), "{warning}");

    let quiet: Vec<Instance> = parse_instances(&running("gpux", 2));
    assert!(
        memory_warning(&quiet, &settings, &capacity, "rayx").is_some(),
        "14 of 16 GiB"
    );
    let roomy = host(10, 32, 200);
    assert_eq!(
        memory_warning(&quiet, &settings, &roomy, "rayx"),
        None,
        "14 of 32 GiB"
    );
}

#[test]
fn the_warning_reaches_the_owner_when_the_vm_is_created() {
    let mut mac = Mac::new(10, 16, 300);
    mac.instances = running("gpux", 8);

    let (result, runner, out) = create(&request(), &mac, None);

    result.expect("creates anyway");
    assert!(create_line(&runner).is_some(), "a warning does not stop");
    assert!(
        out.contains("gpux (8 GiB)") && out.contains("three quarters"),
        "{out}"
    );
}

#[test]
fn customize_no_keeps_the_effective_settings_and_asks_exactly_once() {
    let mut prompter = ScriptedPrompter::always(false);

    let (result, runner, _) = create(&request(), &Mac::new(10, 32, 300), Some(&mut prompter));

    result.expect("creates");
    assert_eq!(
        prompter.asked,
        [
            "Create Lima VM rayx: 6 CPUs, 12 GiB memory, 100 GiB disk, home mounted read-only. Customize?"
        ]
    );
    assert!(
        create_line(&runner)
            .expect("create")
            .contains("--cpus=6 --memory=12 --disk=100 --mount-only=/Users/dev ")
    );
}

#[test]
fn the_question_shows_the_capped_settings_and_the_flags() {
    let mut asked = request();
    asked.vm.cpus = Some(3);
    asked.vm.home_writable = true;
    let mut prompter = ScriptedPrompter::always(false);

    let (result, _, _) = create(&asked, &Mac::new(10, 16, 300), Some(&mut prompter));

    result.expect("creates");
    assert_eq!(
        prompter.asked[0],
        "Create Lima VM rayx: 3 CPUs, 8 GiB memory, 100 GiB disk, home mounted writable. Customize?"
    );
}

#[test]
fn customize_yes_asks_each_value_and_the_home_and_creates_with_the_answers() {
    let mut prompter = ScriptedPrompter::always(false)
        .with_confirms([true, true])
        .with_answers(["4", "", "50"]);

    let (result, runner, _) = create(&request(), &Mac::new(10, 32, 300), Some(&mut prompter));

    result.expect("creates");
    assert_eq!(
        create_line(&runner).expect("create"),
        "create --tty=false --name=rayx --vm-type=vz --cpus=4 --memory=12 --disk=50 \
         --mount-only=/Users/dev:w --containerd=none template:ubuntu-24.04",
        "Enter kept the shown 12 GiB"
    );
    assert_eq!(
        prompter.asked[1..],
        [
            "CPUs [6]",
            "Memory in GiB [12]",
            "Disk in GiB [100]",
            "Mount the home writable?"
        ]
    );
}

#[test]
fn invalid_input_asks_again_with_the_range_until_a_valid_value() {
    let mut prompter = ScriptedPrompter::always(false)
        .with_confirms([true])
        .with_answers(["99", "abc", "0", "3", "", ""]);
    let mut out = Vec::new();

    let chosen = customize(
        "rayx",
        VmSettings::default(),
        &host(10, 32, 300),
        &mut prompter,
        &mut out,
    );

    assert_eq!(chosen.cpus, 3);
    let text = String::from_utf8(out).expect("UTF-8");
    assert_eq!(
        text.matches("CPUs: enter a whole number, 1 to 8").count(),
        3,
        "{text}"
    );
    assert_eq!(
        prompter
            .asked
            .iter()
            .filter(|q| q.starts_with("CPUs"))
            .count(),
        4,
        "asked again after each invalid answer"
    );
}

#[test]
fn nothing_is_asked_with_yes_without_a_terminal_in_check_mode_or_for_an_existing_vm() {
    let mac = Mac::new(10, 32, 300);

    let mut yes = request();
    yes.yes = true;
    let mut prompter = ScriptedPrompter::always(true);
    let (result, runner, _) = create(&yes, &mac, Some(&mut prompter));
    result.expect("creates");
    assert!(
        prompter.asked.is_empty(),
        "--yes asks nothing: {:?}",
        prompter.asked
    );
    assert!(create_line(&runner).is_some());

    let mut no_terminal = ScriptedPrompter::always(true);
    no_terminal.terminal = false;
    let (result, runner, _) = create(&request(), &mac, Some(&mut no_terminal));
    result.expect("creates");
    assert!(no_terminal.asked.is_empty());
    assert!(
        create_line(&runner)
            .expect("create")
            .contains("--cpus=6 --memory=12")
    );

    let mut check = request();
    check.check = true;
    let mut prompter = ScriptedPrompter::always(true);
    let (result, _, _) = create(&check, &mac, Some(&mut prompter));
    result.expect("check");
    assert!(
        prompter.asked.is_empty(),
        "--check creates nothing, so asks nothing"
    );

    let mut existing = Mac::new(10, 32, 300);
    existing.instances = "{\"name\":\"rayx\",\"status\":\"Running\",\"cpus\":6,\"memory\":12884901888,\"disk\":107374182400,\"dir\":\"/x\"}\n".into();
    let mut prompter = ScriptedPrompter::always(true);
    let (result, runner, _) = create(&request(), &existing, Some(&mut prompter));
    result.expect("runs");
    assert!(prompter.asked.is_empty());
    assert!(limactl_changes(&runner).is_empty());
}

#[test]
fn the_values_the_owner_typed_pass_through_the_same_limits_as_the_flags() {
    let mut prompter = ScriptedPrompter::always(false)
        .with_confirms([true])
        .with_answers(["9", "6", "5", "4", "29", "40"]);
    let mut out = Vec::new();

    let chosen = customize(
        "rayx",
        VmSettings::default(),
        &host(8, 8, 300),
        &mut prompter,
        &mut out,
    );

    assert_eq!(
        (chosen.cpus, chosen.memory_gib, chosen.disk_gib),
        (6, 4, 40)
    );
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(
        text.contains("1 to 6") && text.contains("2 to 4") && text.contains("at least 30"),
        "{text}"
    );
}

#[test]
fn a_home_the_flag_made_writable_is_not_asked_about_and_stays_writable() {
    let settings = VmSettings {
        home_writable: true,
        ..VmSettings::default()
    };
    let mut prompter = ScriptedPrompter::always(false).with_confirms([true]);
    let mut out = Vec::new();

    let chosen = customize(
        "rayx",
        settings,
        &host(10, 32, 300),
        &mut prompter,
        &mut out,
    );

    assert!(
        chosen.home_writable,
        "pressing Enter everywhere keeps the flag"
    );
    assert!(
        prompter.asked.iter().all(|q| !q.contains("writable?")),
        "{:?}",
        prompter.asked
    );
    assert!(
        prompter.asked[0].contains("home mounted writable"),
        "the summary says so"
    );
}
