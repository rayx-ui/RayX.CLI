//! `rayx lima remove` and `rayx wsl remove`: only guests `rayx` recorded, always confirmed, the
//! owner's own VMs and distributions untouched, the record updated after the removal.

use std::path::{Path, PathBuf};

use clap::Parser;
use rayx_cli::cli::{Cli, Command, LimaCommand, WslCommand};
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::guest::GuestKind;
use rayx_cli::setup::lima::sizing::ScriptedPrompter;
use rayx_cli::setup::records::{GuestsFile, read_guests_at};
use rayx_cli::setup::remove::{RemoveError, remove_guest};
use rayx_cli::wsl::Distribution;

const GIB: u64 = 1024 * 1024 * 1024;
const RECORD: &str = r#"{"version":1,"guests":[
 {"kind":"lima","name":"rayx","ubuntu":"24.04","flags":[],"clones":[],"retired":false},
 {"kind":"lima","name":"rayx-22.04","ubuntu":"22.04","flags":[],"clones":[],"retired":true},
 {"kind":"wsl","name":"Ubuntu-24.04","ubuntu":"24.04","flags":[],"clones":[],"retired":false},
 {"kind":"wsl","name":"Ubuntu-22.04","ubuntu":"22.04","flags":[],"clones":[],"retired":true}]}"#;

fn host(os: Os) -> HostFacts {
    HostFacts {
        os,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn machine(config: &Path, os: Os) -> FakeMachine {
    let dir = config.join("rayx");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("guests.json"), RECORD).expect("record");
    let mut machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_env("XDG_CONFIG_HOME", &config.display().to_string())
        .with_env("LOCALAPPDATA", &config.display().to_string())
        .with_text(dir.join("guests.json"), RECORD);
    if os == Os::Windows {
        machine = machine.with_wsl_distributions(
            ["Ubuntu-24.04", "Ubuntu-22.04", "Debian"]
                .map(|name| Distribution {
                    id: format!("{{{name}}}"),
                    name: name.into(),
                    base_path: PathBuf::from(format!(r"C:\wsl\{name}")),
                    version: Some(2),
                    default_uid: Some(1000),
                })
                .to_vec(),
        );
    }
    machine
}

fn vms(status_of_old: &str) -> String {
    let line = |name: &str, status: &str| {
        format!(
            "{{\"name\":\"{name}\",\"status\":\"{status}\",\"cpus\":4,\"memory\":{},\"disk\":{},\"dir\":\"/x/{name}\"}}\n",
            8 * GIB,
            60 * GIB
        )
    };
    format!(
        "{}{}{}",
        line("gpux", "Running"),
        line("rayx", "Running"),
        line("rayx-22.04", status_of_old)
    )
}

fn mac_runner(list: String, dirty: &'static str) -> Runner {
    Runner::record().with_os(Os::MacOs).responder(move |spec| {
        let script = spec.args.last().map(String::as_str).unwrap_or_default();
        if spec.args.first().is_some_and(|a| a == "list") {
            Some(Outcome::success().with_stdout(list.clone()))
        } else if script.contains("DIRTY") {
            Some(Outcome::success().with_stdout(dirty))
        } else {
            None
        }
    })
}

fn remove(
    kind: GuestKind,
    name: &str,
    yes: bool,
    os: Os,
    machine: &FakeMachine,
    runner: &mut Runner,
    prompter: Option<&mut ScriptedPrompter>,
) -> (Result<(), RemoveError>, String) {
    let mut out = Vec::new();
    let result = remove_guest(
        kind,
        name,
        yes,
        &host(os),
        runner,
        machine,
        prompter.map(|p| p as &mut dyn rayx_cli::setup::lima::sizing::Prompter),
        &mut out,
    );
    (result, String::from_utf8(out).expect("UTF-8"))
}

fn record(config: &Path) -> GuestsFile {
    read_guests_at(&config.join("rayx/guests.json")).expect("record")
}

fn deletes(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|s| {
            s.args
                .first()
                .is_some_and(|a| a == "delete" || a == "--unregister")
        })
        .map(|s| format!("{} {}", s.program, s.args.join(" ")))
        .collect()
}

#[test]
fn a_recorded_retired_vm_is_deleted_and_dropped_from_the_record() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), Os::MacOs);
    let mut runner = mac_runner(vms("Stopped"), "");
    let mut yes = ScriptedPrompter::always(true);

    let (result, out) = remove(
        GuestKind::Lima,
        "rayx-22.04",
        false,
        Os::MacOs,
        &machine,
        &mut runner,
        Some(&mut yes),
    );

    result.expect("removed");
    assert_eq!(deletes(&runner), ["limactl delete --force rayx-22.04"]);
    assert_eq!(yes.asked, ["Remove rayx-22.04 and everything in it?"]);
    assert!(out.contains("Removed lima guest rayx-22.04."), "{out}");
    assert!(
        !out.contains("creates it again"),
        "a retired guest is not the current one: {out}"
    );
    let kept = record(config.path());
    assert!(kept.find("lima", "rayx-22.04").is_none());
    assert_eq!(kept.guests.len(), 3, "the other guests stay recorded");
    assert!(
        out.contains("stopped, so its clones are not inspected"),
        "a stopped VM is not started to look inside: {out}"
    );
    assert!(
        runner
            .specs()
            .iter()
            .all(|s| s.args.first().is_none_or(|a| a != "shell"))
    );
}

#[test]
fn a_recorded_wsl_distribution_is_unregistered_and_dropped_from_the_record() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), Os::Windows);
    let mut runner = Runner::record().with_os(Os::Windows);

    let (result, _) = remove(
        GuestKind::Wsl,
        "Ubuntu-22.04",
        true,
        Os::Windows,
        &machine,
        &mut runner,
        None,
    );

    result.expect("removed");
    assert_eq!(deletes(&runner), ["wsl --unregister Ubuntu-22.04"]);
    let kept = record(config.path());
    assert!(kept.find("wsl", "Ubuntu-22.04").is_none());
    assert!(kept.find("wsl", "Ubuntu-24.04").is_some());
}

#[test]
fn removing_the_current_guest_says_setup_creates_it_again() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), Os::MacOs);
    let mut runner = mac_runner(vms("Stopped"), "");

    let (result, out) = remove(
        GuestKind::Lima,
        "rayx",
        true,
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );

    result.expect("removed");
    assert!(
        out.contains("`rayx setup --lima` creates it again"),
        "{out}"
    );

    let machine = self::machine(config.path(), Os::Windows);
    let mut runner = Runner::record().with_os(Os::Windows);
    let (result, out) = remove(
        GuestKind::Wsl,
        "Ubuntu-24.04",
        true,
        Os::Windows,
        &machine,
        &mut runner,
        None,
    );
    result.expect("removed");
    assert!(out.contains("`rayx setup --wsl` creates it again"), "{out}");
}

#[test]
fn a_name_rayx_did_not_record_is_refused_and_nothing_runs() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), Os::MacOs);
    let mut runner = mac_runner(vms("Stopped"), "");

    let (result, _) = remove(
        GuestKind::Lima,
        "gpux",
        true,
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );

    let error = result.expect_err("not recorded");
    assert!(matches!(error, RemoveError::NotRecorded(_)));
    assert_eq!(
        error.to_string(),
        "gpux was not created by rayx; remove it with limactl delete yourself"
    );
    assert!(runner.specs().is_empty(), "{:?}", runner.lines());
    assert_eq!(record(config.path()).guests.len(), 4);

    let machine = self::machine(config.path(), Os::Windows);
    let mut runner = Runner::record().with_os(Os::Windows);
    let (result, _) = remove(
        GuestKind::Wsl,
        "Debian",
        true,
        Os::Windows,
        &machine,
        &mut runner,
        None,
    );
    assert!(
        result
            .expect_err("not recorded")
            .to_string()
            .contains("wsl --unregister yourself")
    );
    assert!(runner.specs().is_empty());

    // A name recorded for the other kind is not this kind's.
    let (result, _) = remove(
        GuestKind::Wsl,
        "rayx",
        true,
        Os::Windows,
        &machine,
        &mut runner,
        None,
    );
    assert!(matches!(result, Err(RemoveError::NotRecorded(_))));
}

#[test]
fn the_wrong_os_is_refused_and_nothing_runs() {
    let config = tempfile::tempdir().expect("temp dir");
    for (kind, os) in [
        (GuestKind::Lima, Os::Windows),
        (GuestKind::Lima, Os::Linux),
        (GuestKind::Wsl, Os::MacOs),
    ] {
        let machine = machine(config.path(), os);
        let mut runner = Runner::record().with_os(os);
        let name = if kind == GuestKind::Lima {
            "rayx"
        } else {
            "Ubuntu-24.04"
        };

        let (result, _) = remove(kind, name, true, os, &machine, &mut runner, None);

        assert!(
            matches!(result, Err(RemoveError::WrongOs(_))),
            "{kind:?} on {os:?}"
        );
        assert!(runner.specs().is_empty());
    }
    assert_eq!(record(config.path()).guests.len(), 4);
}

#[test]
fn dirty_clones_are_listed_before_the_question() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), Os::MacOs);
    let mut runner = mac_runner(vms("Running"), "DIRTY\t/home/dev.guest/RayX\n");
    let mut yes = ScriptedPrompter::always(true);

    let (result, out) = remove(
        GuestKind::Lima,
        "rayx-22.04",
        false,
        Os::MacOs,
        &machine,
        &mut runner,
        Some(&mut yes),
    );

    result.expect("removed");
    assert!(
        out.contains("exist nowhere else") && out.contains("/home/dev.guest/RayX"),
        "{out}"
    );
    let shell = runner
        .specs()
        .iter()
        .find(|s| s.args.first().is_some_and(|a| a == "shell"))
        .expect("inspected");
    assert!(shell.args.contains(&"rayx-22.04".to_string()));
}

#[test]
fn declined_or_without_a_terminal_it_refuses_and_removes_nothing() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), Os::MacOs);

    let mut runner = mac_runner(vms("Stopped"), "");
    let mut no = ScriptedPrompter::always(false);
    let (result, _) = remove(
        GuestKind::Lima,
        "rayx-22.04",
        false,
        Os::MacOs,
        &machine,
        &mut runner,
        Some(&mut no),
    );
    assert!(matches!(result, Err(RemoveError::Declined(_))));
    assert!(deletes(&runner).is_empty());

    let mut runner = mac_runner(vms("Stopped"), "");
    let mut silent = ScriptedPrompter::always(true);
    silent.terminal = false;
    let (result, _) = remove(
        GuestKind::Lima,
        "rayx-22.04",
        false,
        Os::MacOs,
        &machine,
        &mut runner,
        Some(&mut silent),
    );
    let error = result.expect_err("no terminal");
    assert!(error.to_string().contains("--yes"), "{error}");
    assert!(silent.asked.is_empty());
    assert!(deletes(&runner).is_empty());
    assert_eq!(record(config.path()).guests.len(), 4);
}

#[test]
fn yes_removes_without_asking() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), Os::MacOs);
    let mut runner = mac_runner(vms("Stopped"), "");
    let mut prompter = ScriptedPrompter::always(false);

    let (result, _) = remove(
        GuestKind::Lima,
        "rayx-22.04",
        true,
        Os::MacOs,
        &machine,
        &mut runner,
        Some(&mut prompter),
    );

    result.expect("removed");
    assert!(prompter.asked.is_empty());
    assert_eq!(deletes(&runner).len(), 1);
}

#[test]
fn a_guest_the_host_no_longer_has_only_loses_its_record() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), Os::MacOs);
    let mut runner = mac_runner(
        "{\"name\":\"gpux\",\"status\":\"Running\",\"cpus\":4,\"memory\":1,\"disk\":1,\"dir\":\"/x\"}\n".into(),
        "",
    );

    let (result, out) = remove(
        GuestKind::Lima,
        "rayx-22.04",
        true,
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );

    result.expect("record dropped");
    assert!(
        deletes(&runner).is_empty(),
        "nothing to delete: {:?}",
        runner.lines()
    );
    assert!(out.contains("only its record is dropped"), "{out}");
    assert!(record(config.path()).find("lima", "rayx-22.04").is_none());
}

#[test]
fn the_command_line_takes_a_name_and_yes() {
    let parsed =
        Cli::try_parse_from(["rayx", "lima", "remove", "rayx-22.04", "--yes"]).expect("lima");
    match parsed.command {
        Command::Lima {
            command: LimaCommand::Remove(args),
        } => {
            assert_eq!((args.name.as_str(), args.yes), ("rayx-22.04", true));
        }
        other => panic!("{other:?}"),
    }
    let parsed = Cli::try_parse_from(["rayx", "wsl", "remove", "Ubuntu-22.04"]).expect("wsl");
    match parsed.command {
        Command::Wsl {
            command: WslCommand::Remove(args),
        } => {
            assert_eq!((args.name.as_str(), args.yes), ("Ubuntu-22.04", false));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        Cli::try_parse_from(["rayx", "lima", "remove"]).is_err(),
        "a name is required"
    );
}

#[test]
fn the_command_refuses_a_name_that_is_not_recorded_with_exit_1() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rayx"))
        .args(["lima", "remove", "gpux", "--yes"])
        .env("HOME", dir.path())
        .env("USERPROFILE", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("LOCALAPPDATA", dir.path().join("local"))
        .output()
        .expect("runs rayx");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("rayx: "));
}
