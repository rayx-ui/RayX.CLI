//! Failed reads are not missing guests, `--check` neither prompts nor starts anything, and the
//! records and clones are read honestly.

use std::path::{Path, PathBuf};

use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::guest::GuestKind;
use rayx_cli::setup::lima::sizing::{Prompter, ScriptedPrompter};
use rayx_cli::setup::lima::{LimaRequest, VmFlags, run_with};
use rayx_cli::setup::records::{
    GuestUpdate, GuestsFile, RecordError, read_guests, read_guests_at, read_setup, record_guest,
    write_guests_at,
};
use rayx_cli::setup::refresh::{GuestOutcome, RefreshOptions, refresh_guests};
use rayx_cli::setup::remove::{RemoveError, remove_guest};
use rayx_cli::setup::update::wsl_binary_for_refresh;
use rayx_cli::setup::wsl::{WslRequest, migrate_older_guest};
use rayx_cli::wsl::{Distribution, parse_status};

const GIB: u64 = 1024 * 1024 * 1024;

fn host(os: Os) -> HostFacts {
    HostFacts {
        os,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

const RECORD: &str = r#"{"version":1,"guests":[{"kind":"lima","name":"rayx","ubuntu":"24.04","flags":[],"clones":[],"retired":false}]}"#;

fn mac_machine(record: &str) -> FakeMachine {
    FakeMachine::new()
        .with_home("/Users/dev")
        .with_program("brew", "/opt/homebrew/bin/brew")
        .with_program("limactl", "/opt/homebrew/bin/limactl")
        .with_text("/Users/dev/.config/rayx/guests.json", record)
}

/// A Mac whose `limactl list` fails (Lima cannot run), whatever else is asked.
fn broken_lima() -> Runner {
    Runner::record().with_os(Os::MacOs).responder(|spec| {
        if spec.program == "limactl" && spec.args.first().is_some_and(|a| a == "list") {
            Some(Outcome::failure(1))
        } else {
            None
        }
    })
}

fn options(yes: bool, check: bool) -> RefreshOptions {
    RefreshOptions {
        version: "1.0.0".into(),
        release: None,
        targets: Vec::new(),
        yes,
        check,
        lima_binary: None,
        wsl_binary: None,
    }
}

#[test]
fn a_failing_limactl_list_is_not_a_missing_guest_in_the_refresh() {
    let mut runner = broken_lima();

    let report = refresh_guests(
        &options(true, false),
        &host(Os::MacOs),
        &mut runner,
        &mac_machine(RECORD),
        None,
        &mut Vec::new(),
    );

    match &report.guests[0].1 {
        GuestOutcome::Failed(why) => {
            assert!(why.contains("cannot read Lima"), "{why}");
            assert!(
                !why.contains("remove"),
                "no advice to drop the record: {why}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(report.code(), 1);
    assert!(
        runner
            .specs()
            .iter()
            .all(|s| s.args.first().is_none_or(|a| a != "shell"))
    );
}

#[test]
fn a_failing_limactl_list_stops_a_removal_before_the_question_and_keeps_the_record() {
    let config = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(config.path().join("rayx")).expect("dir");
    std::fs::write(config.path().join("rayx/guests.json"), RECORD).expect("record");
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("XDG_CONFIG_HOME", &config.path().display().to_string())
        .with_text(config.path().join("rayx/guests.json"), RECORD);
    let mut runner = broken_lima();
    let mut prompter = ScriptedPrompter::always(true);

    let error = remove_guest(
        GuestKind::Lima,
        "rayx",
        false,
        &host(Os::MacOs),
        &mut runner,
        &machine,
        Some(&mut prompter as &mut dyn Prompter),
        &mut Vec::new(),
    )
    .expect_err("cannot read Lima");

    assert!(matches!(error, RemoveError::Failed(_)), "{error}");
    assert!(error.to_string().contains("cannot read Lima"));
    assert!(
        prompter.asked.is_empty(),
        "nothing is asked about a guest that could not be looked at"
    );
    assert_eq!(
        read_guests_at(&config.path().join("rayx/guests.json"))
            .expect("record")
            .guests
            .len(),
        1
    );
}

fn lima_request(check: bool, yes: bool) -> LimaRequest {
    LimaRequest {
        flags: Vec::new(),
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        release: None,
        name: rayx_cli::setup::lima::VmName::default(),
        vm: VmFlags::default(),
        check,
        yes,
    }
}

fn old_lima_mac() -> Runner {
    Runner::record().with_os(Os::MacOs).responder(|spec| {
        match (spec.program.as_str(), spec.args.as_slice()) {
            ("sw_vers", _) => Some(Outcome::success().with_stdout("14.5\n")),
            ("sysctl", _) => Some(Outcome::success().with_stdout("10\n34359738368\n")),
            ("df", _) => Some(Outcome::success().with_stdout("h\n/dev/d 1 1 500000000 1% /\n")),
            ("limactl", [flag]) if flag == "--version" => {
                Some(Outcome::success().with_stdout("limactl version 1.2.1"))
            }
            ("limactl", [a, b]) if a == "list" && b == "--json" => Some(Outcome::success().with_stdout(format!(
                "{{\"name\":\"rayx\",\"status\":\"Running\",\"cpus\":6,\"memory\":{},\"disk\":{},\"dir\":\"/x\"}}\n",
                12 * GIB,
                100 * GIB
            ))),
            _ => None,
        }
    })
}

#[test]
fn check_with_an_old_lima_prints_the_upgrade_and_asks_nothing() {
    let mut runner = old_lima_mac();
    let mut terminal = ScriptedPrompter::always(false);
    let mut out = Vec::new();

    let result = run_with(
        &lima_request(true, false),
        &host(Os::MacOs),
        &mut runner,
        &mac_machine(RECORD),
        None,
        Some(&mut terminal as &mut dyn Prompter),
        &mut out,
    );

    result.expect("--check continues");
    assert!(terminal.asked.is_empty(), "{:?}", terminal.asked);
    let out = String::from_utf8(out).expect("UTF-8");
    assert!(
        out.contains("would run: /opt/homebrew/bin/brew upgrade lima"),
        "{out}"
    );
    assert!(
        runner
            .specs()
            .iter()
            .all(|s| !s.args.contains(&"upgrade".to_string())),
        "nothing is upgraded: {:?}",
        runner.lines()
    );
}

fn windows_machine(config: &Path, record: &str, registered: &[&str]) -> FakeMachine {
    std::fs::create_dir_all(config.join("rayx")).expect("dir");
    std::fs::write(config.join("rayx/guests.json"), record).expect("record");
    FakeMachine::new()
        .with_env("LOCALAPPDATA", &config.display().to_string())
        .with_wsl_distributions(
            registered
                .iter()
                .map(|name| Distribution {
                    id: format!("{{{name}}}"),
                    name: (*name).into(),
                    base_path: PathBuf::from(format!(r"C:\wsl\{name}")),
                    version: Some(2),
                    default_uid: Some(1000),
                })
                .collect(),
        )
        .with_text(config.join("rayx/guests.json"), record)
}

const OLD_WSL: &str = r#"{"version":1,"guests":[{"kind":"wsl","name":"Ubuntu-22.04","ubuntu":"22.04","flags":["--web"],"clones":[],"retired":false}]}"#;

fn wsl_request() -> WslRequest {
    WslRequest {
        flags: vec!["--web".into()],
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        release: None,
    }
}

fn wsl_runner(running: &'static str) -> Runner {
    Runner::record()
        .with_os(Os::Windows)
        .responder(move |spec| {
            if spec.args.contains(&"--running".to_string()) {
                Some(Outcome::success().with_stdout(running))
            } else if spec.args.last().is_some_and(|a| a.contains("os-release")) {
                Some(Outcome::success().with_stdout("22.04\n"))
            } else {
                None
            }
        })
}

#[test]
fn check_never_starts_a_stopped_wsl_distribution_to_look_inside_it() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = windows_machine(config.path(), OLD_WSL, &["Ubuntu-22.04"]);

    let mut stopped = wsl_runner("");
    let mut out = Vec::new();
    migrate_older_guest(
        &wsl_request(),
        true,
        false,
        &host(Os::Windows),
        &mut stopped,
        &machine,
        None,
        &mut out,
    )
    .expect("--check");
    assert!(
        stopped
            .specs()
            .iter()
            .all(|s| s.args.first().is_none_or(|a| a != "-d")),
        "no command ran inside the distribution: {:?}",
        stopped.lines()
    );

    let mut running = wsl_runner("Ubuntu-22.04\n");
    migrate_older_guest(
        &wsl_request(),
        true,
        false,
        &host(Os::Windows),
        &mut running,
        &machine,
        None,
        &mut Vec::new(),
    )
    .expect("--check");
    assert!(
        running
            .specs()
            .iter()
            .any(|s| s.args.first().is_some_and(|a| a == "-d")),
        "a running distribution is inspected: {:?}",
        running.lines()
    );
}

#[test]
fn only_a_missing_record_is_empty_and_a_file_that_cannot_be_read_is_left_alone() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert_eq!(
        read_guests_at(&dir.path().join("none.json")).expect("missing"),
        GuestsFile::default()
    );

    // A directory where the file should be: not "empty", not overwritten.
    let in_the_way = dir.path().join("rayx/guests.json");
    std::fs::create_dir_all(&in_the_way).expect("dir in the way");
    let error = read_guests_at(&in_the_way).expect_err("unreadable");
    assert!(matches!(error, RecordError::Unreadable { .. }), "{error}");
    assert!(error.to_string().contains("left untouched"));

    let machine = FakeMachine::new()
        .with_env("XDG_CONFIG_HOME", &dir.path().display().to_string())
        .with_home("/home/dev");
    let update = GuestUpdate {
        kind: "lima",
        name: "rayx".into(),
        ubuntu: "24.04".into(),
        flags: vec![],
        clone: None,
        vm: None,
    };
    assert!(record_guest(&machine, Os::Linux, &update, false).is_err());
    assert!(in_the_way.is_dir(), "the thing in the way is still there");
}

#[test]
fn a_machine_that_cannot_read_an_existing_record_reports_it_for_both_records() {
    // The file exists but its text cannot be read (not UTF-8): neither guests nor setup read as empty.
    let machine = FakeMachine::new()
        .with_home("/home/dev")
        .with_file("/home/dev/.config/rayx/guests.json")
        .with_file("/home/dev/.config/rayx/setup.json");

    assert!(matches!(
        read_guests(&machine, Os::Linux),
        Err(RecordError::Unreadable { .. })
    ));
    assert!(matches!(
        read_setup(&machine, Os::Linux),
        Err(RecordError::Unreadable { .. })
    ));

    let missing = FakeMachine::new().with_home("/home/dev");
    assert_eq!(
        read_guests(&missing, Os::Linux).expect("missing is empty"),
        GuestsFile::default()
    );
}

#[test]
fn concurrent_writes_never_leave_a_corrupt_or_temporary_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("rayx/guests.json");
    let handles: Vec<_> = (0..4)
        .map(|n| {
            let path = path.clone();
            std::thread::spawn(move || {
                for round in 0..25 {
                    let mut file = GuestsFile::default();
                    file.upsert(&GuestUpdate {
                        kind: "lima",
                        name: format!("vm-{n}-{round}"),
                        ubuntu: "24.04".into(),
                        flags: vec![],
                        clone: None,
                        vm: None,
                    });
                    write_guests_at(&path, &file).expect("write");
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("thread");
    }

    assert_eq!(
        read_guests_at(&path).expect("a valid record").guests.len(),
        1
    );
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().expect("dir"))
        .expect("dir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_clone_in_both_clones_files_is_counted_once() {
    let text = "DF\t100\t200\nOUT\t10\t/home/dev/RayX/target\nOUT\t10\t/home/dev/RayX/target\nOUT\t5\t/home/dev/RayX/artifacts\n";

    let (_, _, outputs) = parse_status(text);

    assert_eq!(outputs.len(), 2, "{outputs:?}");
    assert_eq!(outputs.iter().map(|o| o.bytes).sum::<u64>(), 15);
}

#[test]
fn a_wsl_binary_the_distribution_cannot_reach_is_reported_not_silently_dropped() {
    let mut warnings = Vec::new();

    let reachable = wsl_binary_for_refresh(Some(PathBuf::from(r"D:\rayx\linux\rayx")), &mut |w| {
        warnings.push(w)
    });
    let outside = wsl_binary_for_refresh(Some(PathBuf::from(r"\\server\share\rayx")), &mut |w| {
        warnings.push(w)
    });
    let none = wsl_binary_for_refresh(None, &mut |w| warnings.push(w));

    assert_eq!(reachable.as_deref(), Some("/mnt/d/rayx/linux/rayx"));
    assert_eq!(outside, None);
    assert_eq!(none, None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("RAYX_WSL_BINARY is ignored"),
        "{}",
        warnings[0]
    );
}

#[test]
fn doctor_reports_a_failing_limactl_list_instead_of_calling_the_guests_missing() {
    use rayx_cli::doctor::report_for;
    use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
    use rayx_cli::project::Pins;
    use rayx_cli::setup::PlanEnv;

    let env = PlanEnv {
        host: host(Os::MacOs),
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
    };
    let mut runner = Runner::record().with_os(Os::MacOs).responder(|spec| {
        match (spec.program.as_str(), spec.args.as_slice()) {
            ("limactl", [flag]) if flag == "--version" => {
                Some(Outcome::success().with_stdout("limactl version 2.2.1"))
            }
            ("limactl", [a, _]) if a == "list" => Some(Outcome::failure(1)),
            _ => None,
        }
    });
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));

    let report = report_for(&env, &mut runner, &mac_machine(RECORD), &mut path, None);

    let json = report.to_json();
    let guest = &json["guests"][0];
    assert_eq!(guest["status"], "unknown");
    assert!(
        guest["remove"].is_null(),
        "no advice to drop the record: {guest}"
    );
    assert!(
        json["guestsProblem"]
            .as_str()
            .is_some_and(|p| p.contains("cannot read Lima")),
        "{json}"
    );
    assert!(report.to_text().contains("cannot read Lima"));
}

#[test]
fn the_refresh_wiring_reports_an_unreachable_wsl_binary_on_stderr() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rayx"))
        .args(["setup", "--update", "--check"])
        .env("RAYX_WSL_BINARY", "/not/on/a/windows/drive/rayx")
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("LOCALAPPDATA", dir.path().join("local"))
        .current_dir(dir.path())
        .output()
        .expect("runs rayx");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("RAYX_WSL_BINARY is ignored and the release installer is used"),
        "{stderr}"
    );
}
