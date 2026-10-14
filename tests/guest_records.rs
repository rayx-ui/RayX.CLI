//! `guests.json`: written on a successful `setup --wsl` / `--lima`, merged on a rerun, tolerant of
//! a missing file and respectful of an unparsable one; and the guests section of `rayx doctor`.

use std::path::{Path, PathBuf};

use rayx_cli::doctor::report_for;
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::guest::{Checkout, Guest, record_run};
use rayx_cli::setup::lima::{
    LimaOutcome, LimaRequest, VmFlags, record_outcome as lima_record, run_with,
};
use rayx_cli::setup::records::{
    CloneRecord, GuestUpdate, GuestsFile, RecordError, VmRecord, config_dir, guests_path,
    read_guests_at, record_guest,
};
use rayx_cli::setup::wsl::{WslRequest, record_outcome as wsl_record};
use rayx_cli::setup::{FakeMachine, PlanEnv};
use rayx_cli::wsl::Distribution;

fn update(kind: &'static str, name: &str, flags: &[&str]) -> GuestUpdate {
    GuestUpdate {
        kind,
        name: name.into(),
        ubuntu: "24.04".into(),
        flags: flags.iter().map(ToString::to_string).collect(),
        clone: None,
        vm: None,
    }
}

fn config_machine(dir: &Path) -> FakeMachine {
    FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("XDG_CONFIG_HOME", &dir.display().to_string())
        .with_env("LOCALAPPDATA", &dir.display().to_string())
}

#[test]
fn the_record_lives_in_the_config_directory_of_the_platform() {
    let unix = FakeMachine::new().with_home("/Users/dev");
    assert_eq!(
        config_dir(&unix, Os::MacOs),
        Some(PathBuf::from("/Users/dev/.config/rayx"))
    );
    let xdg = unix.with_env("XDG_CONFIG_HOME", "/cfg");
    assert_eq!(
        guests_path(&xdg, Os::Linux),
        Some(PathBuf::from("/cfg/rayx/guests.json"))
    );
    let windows = FakeMachine::new().with_env("LOCALAPPDATA", r"C:\Users\dev\AppData\Local");
    assert!(
        config_dir(&windows, Os::Windows)
            .expect("dir")
            .to_string_lossy()
            .starts_with(r"C:\Users\dev\AppData\Local")
    );
    assert_eq!(
        config_dir(&FakeMachine::new(), Os::MacOs),
        None,
        "no home, no record"
    );
}

#[test]
fn a_guest_is_recorded_with_its_flags_clone_and_vm_settings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = config_machine(dir.path());
    let mut first = update("lima", "rayx", &["--web", "--check", "--yes"]);
    first.clone = Some(CloneRecord {
        source: "/Users/dev/RayX".into(),
        branch: Some("main".into()),
        target: "$HOME/RayX".into(),
    });
    first.vm = Some(VmRecord {
        cpus: 6,
        memory_gib: 12,
        disk_gib: 100,
        home_writable: false,
    });

    record_guest(&machine, Os::MacOs, &first, false).expect("records");

    let text = std::fs::read_to_string(dir.path().join("rayx/guests.json")).expect("file");
    let document: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(document["version"], 1);
    let guest = &document["guests"][0];
    assert_eq!(guest["kind"], "lima");
    assert_eq!(guest["name"], "rayx");
    assert_eq!(guest["ubuntu"], "24.04");
    assert_eq!(
        guest["flags"],
        serde_json::json!(["--web"]),
        "--check and --yes are not recorded"
    );
    assert_eq!(guest["clones"][0]["target"], "$HOME/RayX");
    assert_eq!(guest["clones"][0]["branch"], "main");
    assert_eq!(guest["vm"]["memory_gib"], 12);
    assert_eq!(guest["retired"], false);
    assert!(
        !dir.path().join("rayx/guests.json.tmp").exists(),
        "no temporary file is left"
    );
}

#[test]
fn a_rerun_updates_the_entry_instead_of_adding_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = config_machine(dir.path());
    let mut first = update("lima", "rayx", &["--web"]);
    first.vm = Some(VmRecord {
        cpus: 6,
        memory_gib: 12,
        disk_gib: 100,
        home_writable: false,
    });
    record_guest(&machine, Os::MacOs, &first, false).expect("first");

    let mut second = update("lima", "rayx", &["--web", "--test"]);
    second.ubuntu = "24.10".into();
    second.clone = Some(CloneRecord {
        source: "/Users/dev/RayX".into(),
        branch: None,
        target: "$HOME/RayX".into(),
    });
    record_guest(&machine, Os::MacOs, &second, false).expect("second");
    record_guest(&machine, Os::MacOs, &second, false).expect("again");
    record_guest(&machine, Os::MacOs, &update("wsl", "rayx", &[]), false).expect("other kind");

    let file = read_guests_at(&dir.path().join("rayx/guests.json")).expect("reads");
    assert_eq!(
        file.guests.len(),
        2,
        "one lima entry, one wsl entry of the same name"
    );
    let lima = file.find("lima", "rayx").expect("lima rayx");
    assert_eq!(lima.flags, ["--web", "--test"], "a union over runs");
    assert_eq!(lima.ubuntu, "24.10");
    assert_eq!(lima.clones.len(), 1, "one clone per target");
    assert_eq!(
        lima.vm.expect("kept").cpus,
        6,
        "a run that did not touch the VM keeps its settings"
    );
}

#[test]
fn a_missing_file_is_an_empty_record_and_an_unparsable_one_is_left_untouched() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("rayx/guests.json");
    assert_eq!(
        read_guests_at(&path).expect("missing is empty"),
        GuestsFile::default()
    );

    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(&path, "{ not json").expect("write");
    let machine = config_machine(dir.path());

    let error = record_guest(&machine, Os::MacOs, &update("lima", "rayx", &[]), false)
        .expect_err("unparsable");

    assert!(matches!(error, RecordError::Unparsable { .. }), "{error}");
    assert!(error.to_string().contains("left untouched"));
    assert_eq!(std::fs::read_to_string(&path).expect("file"), "{ not json");
    std::fs::write(&path, "  \n").expect("blank");
    assert_eq!(
        read_guests_at(&path).expect("blank is empty"),
        GuestsFile::default()
    );
}

#[test]
fn a_dry_run_records_nothing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = config_machine(dir.path());

    record_guest(&machine, Os::MacOs, &update("lima", "rayx", &[]), true).expect("dry run");

    assert!(!dir.path().join("rayx").exists());
}

#[test]
fn a_finished_run_records_the_guests_own_ubuntu_version() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = config_machine(dir.path());
    let mut runner = Runner::record().respond(
        |spec| spec.args.last().is_some_and(|a| a.contains("os-release")),
        Outcome::success().with_stdout("24.04\n"),
    );

    record_run(
        &machine,
        Os::MacOs,
        &Guest::lima("rayx"),
        &["--web".into()],
        None,
        None,
        &mut runner,
    );

    let file = read_guests_at(&dir.path().join("rayx/guests.json")).expect("reads");
    assert_eq!(file.find("lima", "rayx").expect("recorded").ubuntu, "24.04");
    assert_eq!(
        runner.specs()[0].program,
        "limactl",
        "read through the guest layer"
    );
}

fn host(os: Os) -> HostFacts {
    HostFacts {
        os,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn env(os: Os) -> PlanEnv {
    PlanEnv {
        host: host(os),
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
    }
}

const RECORD: &str = r#"{"version":1,"guests":[
 {"kind":"lima","name":"rayx","ubuntu":"24.04","flags":["--web"],"clones":[],"retired":false},
 {"kind":"lima","name":"rayx-22.04","ubuntu":"22.04","flags":[],"clones":[],"retired":true},
 {"kind":"lima","name":"rayx-old","ubuntu":"22.04","flags":[],"clones":[],"retired":false},
 {"kind":"lima","name":"gone","ubuntu":"24.04","flags":[],"clones":[],"retired":false},
 {"kind":"wsl","name":"Ubuntu-24.04","ubuntu":"24.04","flags":[],"clones":[],"retired":false}
]}"#;

fn instance(name: &str, status: &str, cpus: u32, memory: u64, disk: u64) -> String {
    format!(
        "{{\"name\":\"{name}\",\"status\":\"{status}\",\"cpus\":{cpus},\"memory\":{},\"disk\":{},\"dir\":\"/Users/dev/.lima/{name}\"}}\n",
        memory * 1024 * 1024 * 1024,
        disk * 1024 * 1024 * 1024
    )
}

fn mac_runner(lima: Option<&'static str>, list: String) -> Runner {
    Runner::record().with_os(Os::MacOs).responder(move |spec| {
        match (spec.program.as_str(), spec.args.as_slice()) {
            ("sw_vers", _) => Some(Outcome::success().with_stdout("14.5\n")),
            ("sysctl", _) => Some(Outcome::success().with_stdout("10\n34359738368\n")),
            ("df", _) => Some(Outcome::success().with_stdout("h\n/dev/d 1 1 500000000 1% /\n")),
            ("limactl", [flag]) if flag == "--version" => Some(match lima {
                Some(text) => Outcome::success().with_stdout(text),
                None => Outcome::failure(127),
            }),
            ("limactl", [a, b]) if a == "list" && b == "--json" => {
                Some(Outcome::success().with_stdout(list.clone()))
            }
            _ => None,
        }
    })
}

fn mac_machine(record: Option<&str>) -> FakeMachine {
    let mut machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_text(
            "/Users/dev/.lima/rayx/lima.yaml",
            "mounts:\n- location: \"~\"\n  writable: true\n",
        )
        .with_text(
            "/Users/dev/.lima/rayx-22.04/lima.yaml",
            "mounts:\n- location: \"/Users/dev\"\n",
        );
    if let Some(record) = record {
        machine = machine.with_text("/Users/dev/.config/rayx/guests.json", record);
    }
    machine
}

fn doctor(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> rayx_cli::doctor::Report {
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    report_for(env, runner, machine, &mut path, None)
}

#[test]
fn doctor_joins_recorded_lima_guests_with_what_limactl_reports() {
    let list = format!(
        "{}{}{}",
        instance("rayx", "Running", 6, 12, 100),
        instance("rayx-22.04", "Stopped", 4, 8, 60),
        instance("rayx-old", "Stopped", 2, 4, 30)
    );
    let mut runner = mac_runner(Some("limactl version 2.2.1"), list);

    let report = doctor(&env(Os::MacOs), &mac_machine(Some(RECORD)), &mut runner);

    let json = report.to_json();
    assert_eq!(json["lima"]["installed"], true);
    assert_eq!(json["lima"]["version"], "2.2.1");
    let guests: Vec<&serde_json::Value> = json["guests"]
        .as_array()
        .expect("array")
        .iter()
        .filter(|g| g["kind"] == "lima")
        .collect();
    assert_eq!(guests.len(), 4);
    let by_name = |name: &str| *guests.iter().find(|g| g["name"] == name).expect(name);

    let rayx = by_name("rayx");
    assert_eq!(rayx["status"], "running");
    assert_eq!(
        (
            rayx["cpus"].as_u64(),
            rayx["memoryGib"].as_u64(),
            rayx["diskGib"].as_u64()
        ),
        (Some(6), Some(12), Some(100))
    );
    assert_eq!(rayx["home"], "writable");
    assert_eq!(rayx["ubuntu"], "24.04");
    assert_eq!(
        (rayx["current"].clone(), rayx["state"].clone()),
        (true.into(), "current".into())
    );
    assert!(
        rayx["remove"].is_null(),
        "a current guest has no removal command"
    );

    let retired = by_name("rayx-22.04");
    assert_eq!(retired["status"], "stopped");
    assert_eq!(retired["home"], "read-only");
    assert_eq!(retired["state"], "retired");
    assert_eq!(retired["remove"], "rayx lima remove rayx-22.04");

    assert_eq!(
        by_name("rayx-old")["state"],
        "outdated",
        "older than the pin, not replaced yet"
    );
    let gone = by_name("gone");
    assert_eq!(gone["status"], "missing");
    assert_eq!(gone["remove"], "rayx lima remove gone");
    assert!(gone["cpus"].is_null());

    assert!(
        runner.specs().iter().all(|s| s.program != "limactl"
            || matches!(
                s.args.first().map(String::as_str),
                Some("--version" | "list")
            )),
        "doctor only reads: {:?}",
        runner.lines()
    );
}

#[test]
fn doctor_text_lists_each_guest_and_the_removal_command() {
    let list = format!(
        "{}{}",
        instance("rayx", "Running", 6, 12, 100),
        instance("rayx-22.04", "Stopped", 4, 8, 60)
    );
    let mut runner = mac_runner(Some("limactl version 2.2.1"), list);

    let text = doctor(&env(Os::MacOs), &mac_machine(Some(RECORD)), &mut runner).to_text();

    assert!(text.contains("Lima: limactl 2.2.1"), "{text}");
    assert!(
        text.contains("lima rayx: running, 6 CPUs, 12 GiB memory, 100 GiB disk, home writable, Ubuntu 24.04, current"),
        "{text}"
    );
    assert!(text.contains("lima rayx-22.04: stopped"), "{text}");
    assert!(
        text.contains("remove: rayx lima remove rayx-22.04"),
        "{text}"
    );
    assert!(text.contains("lima gone: missing"), "{text}");
    assert!(
        text.contains("  ubuntu: 24.04 (default)"),
        "the pin is listed: {text}"
    );
}

#[test]
fn doctor_says_when_lima_is_missing_and_still_reports_the_recorded_guests() {
    let mut runner = mac_runner(None, String::new());

    let report = doctor(&env(Os::MacOs), &mac_machine(Some(RECORD)), &mut runner);

    let json = report.to_json();
    assert_eq!(json["lima"]["installed"], false);
    assert!(
        report
            .to_text()
            .contains("Lima: missing (rayx setup --lima installs it)")
    );
    let rayx = json["guests"]
        .as_array()
        .expect("array")
        .iter()
        .find(|g| g["name"] == "rayx")
        .expect("rayx")
        .clone();
    assert_eq!(rayx["status"], "missing");
    assert!(
        runner
            .specs()
            .iter()
            .all(|s| s.program != "limactl" || !s.args.contains(&"list".to_string())),
        "no list without limactl"
    );
}

#[test]
fn doctor_without_a_record_or_with_a_broken_one_still_succeeds() {
    let mut runner = mac_runner(Some("limactl version 2.2.1"), String::new());
    let none = doctor(&env(Os::MacOs), &mac_machine(None), &mut runner);
    assert_eq!(none.to_json()["guests"], serde_json::json!([]));
    assert!(none.to_text().contains("no guests recorded"));

    let mut runner = mac_runner(Some("limactl version 2.2.1"), String::new());
    let broken = doctor(&env(Os::MacOs), &mac_machine(Some("{ nope")), &mut runner);
    let text = broken.to_text();
    assert!(
        text.contains("not a valid guest record") && text.contains("left untouched"),
        "{text}"
    );
    assert!(broken.to_json()["guestsProblem"].is_string());
}

#[test]
fn on_windows_the_recorded_wsl_guests_are_reported_the_same_way() {
    let record = r#"{"version":1,"guests":[
     {"kind":"wsl","name":"Ubuntu-24.04","ubuntu":"24.04","flags":[],"clones":[],"retired":false},
     {"kind":"wsl","name":"Ubuntu-22.04","ubuntu":"22.04","flags":[],"clones":[],"retired":true},
     {"kind":"wsl","name":"Ubuntu-20.04","ubuntu":"20.04","flags":[],"clones":[],"retired":false}]}"#;
    let distribution = |name: &str| Distribution {
        id: format!("{{{name}}}"),
        name: name.into(),
        base_path: PathBuf::from(format!(r"C:\wsl\{name}")),
        version: Some(2),
        default_uid: Some(1000),
    };
    let machine = FakeMachine::new()
        .with_env("LOCALAPPDATA", r"C:\Users\dev\AppData\Local")
        .with_text(
            config_dir(
                &FakeMachine::new().with_env("LOCALAPPDATA", r"C:\Users\dev\AppData\Local"),
                Os::Windows,
            )
            .expect("dir")
            .join("guests.json"),
            record,
        )
        .with_wsl_distributions(vec![
            distribution("Ubuntu-24.04"),
            distribution("Ubuntu-22.04"),
        ])
        .with_size(distribution("Ubuntu-24.04").vhdx(), 4 * 1024 * 1024 * 1024);
    let mut runner = Runner::record().with_os(Os::Windows).respond(
        |spec| spec.program == "wsl" && spec.args.contains(&"--running".to_string()),
        Outcome::success().with_stdout(
            "Ubuntu-24.04\n"
                .chars()
                .flat_map(|c| [c, '\0'])
                .collect::<String>(),
        ),
    );

    let report = doctor(&env(Os::Windows), &machine, &mut runner);

    let json = report.to_json();
    let wsl: Vec<&serde_json::Value> = json["guests"].as_array().expect("array").iter().collect();
    let by_name = |name: &str| *wsl.iter().find(|g| g["name"] == name).expect(name);
    assert_eq!(by_name("Ubuntu-24.04")["status"], "running");
    assert_eq!(
        by_name("Ubuntu-24.04")["diskBytes"],
        4u64 * 1024 * 1024 * 1024
    );
    assert_eq!(by_name("Ubuntu-22.04")["status"], "stopped");
    assert_eq!(
        by_name("Ubuntu-22.04")["remove"],
        "rayx wsl remove Ubuntu-22.04"
    );
    assert_eq!(by_name("Ubuntu-20.04")["status"], "missing");
    assert!(json["lima"].is_null(), "no Lima on Windows");
}

#[test]
fn a_linux_host_reports_no_guests() {
    let mut runner = Runner::record().with_os(Os::Linux);
    let report = doctor(
        &env(Os::Linux),
        &FakeMachine::new().with_home("/home/dev"),
        &mut runner,
    );
    assert_eq!(report.to_json()["guests"], serde_json::json!([]));
    assert!(!report.to_text().contains("Guests:"));
}

fn lima_request(check: bool) -> LimaRequest {
    LimaRequest {
        flags: vec!["--web".into()],
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        vm: VmFlags::default(),
        check,
        yes: true,
    }
}

fn mac_host() -> HostFacts {
    host(Os::MacOs)
}

fn lima_outcome(request: &LimaRequest, list: String, checkout: Option<&Checkout>) -> LimaOutcome {
    let mut runner = mac_runner(Some("limactl version 2.2.1"), list);
    let machine = mac_machine(None)
        .with_program("brew", "/opt/homebrew/bin/brew")
        .with_program("limactl", "/opt/homebrew/bin/limactl");
    run_with(
        request,
        &mac_host(),
        &mut runner,
        &machine,
        checkout,
        None,
        &mut Vec::new(),
    )
    .expect("runs")
}

#[test]
fn a_created_vm_reports_the_settings_it_was_created_with() {
    let outcome = lima_outcome(&lima_request(false), String::new(), None);

    assert_eq!(
        outcome.vm,
        Some(VmRecord {
            cpus: 6,
            memory_gib: 12,
            disk_gib: 100,
            home_writable: false
        })
    );
    assert_eq!(outcome.clone, None);
}

#[test]
fn an_existing_vm_reports_what_it_is_now_edited_or_not() {
    // Unchanged (and unrecorded before): the entry still gets the VM's real settings, home from lima.yaml.
    let unchanged = lima_outcome(
        &lima_request(false),
        instance("rayx", "Running", 4, 8, 60),
        None,
    );
    assert_eq!(
        unchanged.vm,
        Some(VmRecord {
            cpus: 4,
            memory_gib: 8,
            disk_gib: 60,
            home_writable: true
        })
    );

    let mut asked = lima_request(false);
    asked.vm = VmFlags {
        cpus: Some(5),
        disk_gib: Some(80),
        ..VmFlags::default()
    };
    let edited = lima_outcome(&asked, instance("rayx", "Stopped", 4, 8, 60), None);
    assert_eq!(
        edited.vm,
        Some(VmRecord {
            cpus: 5,
            memory_gib: 8,
            disk_gib: 80,
            home_writable: true
        })
    );
}

#[test]
fn a_clone_is_part_of_the_outcome() {
    let mut asked = lima_request(false);
    asked.clone = Some(String::new());
    let checkout = Checkout {
        root: PathBuf::from("/Users/dev/RayX"),
        branch: Some("main".into()),
    };

    let outcome = lima_outcome(
        &asked,
        instance("rayx", "Running", 6, 12, 100),
        Some(&checkout),
    );

    assert_eq!(
        outcome.clone,
        Some(CloneRecord {
            source: "/Users/dev/RayX".into(),
            branch: Some("main".into()),
            target: "$HOME/RayX".into()
        })
    );
}

#[test]
fn check_records_nothing_and_a_real_run_records_the_vm_and_clone() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = config_machine(dir.path());
    let outcome = LimaOutcome {
        vm: Some(VmRecord {
            cpus: 6,
            memory_gib: 12,
            disk_gib: 100,
            home_writable: false,
        }),
        clone: Some(CloneRecord {
            source: "/Users/dev/RayX".into(),
            branch: None,
            target: "$HOME/RayX".into(),
        }),
    };
    let mut runner = Runner::record().respond(
        |spec| spec.args.last().is_some_and(|a| a.contains("os-release")),
        Outcome::success().with_stdout("24.04\n"),
    );

    lima_record(
        &lima_request(true),
        &mac_host(),
        &machine,
        &mut runner,
        &outcome,
    );
    assert!(!dir.path().join("rayx").exists(), "--check writes nothing");
    assert!(
        runner.specs().is_empty(),
        "--check does not even read the guest"
    );

    lima_record(
        &lima_request(false),
        &mac_host(),
        &machine,
        &mut runner,
        &outcome,
    );
    let file = read_guests_at(&dir.path().join("rayx/guests.json")).expect("reads");
    let guest = file.find("lima", "rayx").expect("recorded");
    assert_eq!(guest.vm, outcome.vm);
    assert_eq!(guest.clones.len(), 1);
    assert_eq!(guest.flags, ["--web"]);
}

#[test]
fn a_wsl_run_records_the_distribution_and_its_clone_but_check_does_not() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = config_machine(dir.path());
    let request = WslRequest {
        flags: vec!["--web".into(), "--yes".into()],
        clone: Some(String::new()),
        binary_override: None,
        version: "1.0.0".into(),
        windows_gpu: None,
    };
    let checkout = Checkout {
        root: PathBuf::from(r"D:\Work\RayX"),
        branch: Some("main".into()),
    };
    let windows = host(Os::Windows);
    let mut runner = Runner::record().respond(
        |spec| spec.args.last().is_some_and(|a| a.contains("os-release")),
        Outcome::success().with_stdout("24.04\n"),
    );

    wsl_record(
        &request,
        true,
        &windows,
        &machine,
        &mut runner,
        Some(&checkout),
    );
    assert!(!dir.path().join("rayx").exists(), "--check writes nothing");

    wsl_record(
        &request,
        false,
        &windows,
        &machine,
        &mut runner,
        Some(&checkout),
    );
    let file = read_guests_at(&dir.path().join("rayx/guests.json")).expect("reads");
    let guest = file.find("wsl", "Ubuntu-24.04").expect("recorded");
    assert_eq!(guest.flags, ["--web"]);
    assert_eq!(
        guest.clones,
        [CloneRecord {
            source: r"D:\Work\RayX".into(),
            branch: Some("main".into()),
            target: "$HOME/RayX".into()
        }]
    );
    assert!(guest.vm.is_none());
}

#[test]
fn a_record_that_is_not_text_is_left_untouched_too() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("rayx/guests.json");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(&path, [0xff, 0xfe, 0x00, 0x7b]).expect("write");
    let machine = config_machine(dir.path());

    let error = record_guest(&machine, Os::MacOs, &update("lima", "rayx", &[]), false)
        .expect_err("not text");

    assert!(matches!(error, RecordError::Unreadable { .. }), "{error}");
    assert_eq!(
        std::fs::read(&path).expect("file"),
        [0xff, 0xfe, 0x00, 0x7b]
    );
}

#[test]
fn an_unreadable_guest_version_keeps_what_the_record_says() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = config_machine(dir.path());
    let mut old = update("lima", "rayx", &[]);
    old.ubuntu = "22.04".into();
    record_guest(&machine, Os::MacOs, &old, false).expect("recorded");

    // The guest does not answer: the old version stays rather than becoming the pin.
    let mut silent = Runner::record().respond(
        |spec| spec.args.last().is_some_and(|a| a.contains("os-release")),
        Outcome::failure(1),
    );
    let path = dir.path().join("rayx/guests.json");
    let seen = config_machine(dir.path())
        .with_text(&path, &std::fs::read_to_string(&path).expect("record"));
    record_run(
        &seen,
        Os::MacOs,
        &Guest::lima("rayx"),
        &[],
        None,
        None,
        &mut silent,
    );
    let file = read_guests_at(&dir.path().join("rayx/guests.json")).expect("reads");
    assert_eq!(file.find("lima", "rayx").expect("recorded").ubuntu, "22.04");
}
