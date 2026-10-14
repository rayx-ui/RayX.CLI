//! The guest refresh of `rayx setup --update`: per current guest a question, the Linux `rayx`,
//! `apt-get update` and `upgrade` inside, and `rayx setup --update` inside; a stopped Lima VM is
//! started and stopped again, a failing guest does not stop the next, `--check` starts nothing.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use rayx_cli::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::lima::sizing::ScriptedPrompter;
use rayx_cli::setup::refresh::{
    GuestOutcome, RefreshOptions, RefreshReport, older_than_pin, refresh_guests,
};
use rayx_cli::wsl::Distribution;

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

fn options(yes: bool, check: bool) -> RefreshOptions {
    RefreshOptions {
        version: "1.0.0".into(),
        yes,
        check,
        lima_binary: None,
        wsl_binary: None,
    }
}

fn guest_json(kind: &str, name: &str, ubuntu: &str, retired: bool) -> String {
    format!(
        r#"{{"kind":"{kind}","name":"{name}","ubuntu":"{ubuntu}","flags":["--web"],"clones":[],"retired":{retired}}}"#
    )
}

fn machine(guests: &[String]) -> FakeMachine {
    FakeMachine::new().with_home("/Users/dev").with_text(
        "/Users/dev/.config/rayx/guests.json",
        &format!(r#"{{"version":1,"guests":[{}]}}"#, guests.join(",")),
    )
}

fn vm(name: &str, status: &str) -> String {
    format!(
        "{{\"name\":\"{name}\",\"status\":\"{status}\",\"cpus\":6,\"memory\":{},\"disk\":{},\"dir\":\"/x/{name}\"}}\n",
        12 * GIB,
        100 * GIB
    )
}

/// Scripts a Mac: the instances, each guest's Ubuntu and `rayx` version, and which guests' apt fails.
fn mac(instances: String, ubuntu: &'static str, failing: &'static [&'static str]) -> Runner {
    Runner::record().with_os(Os::MacOs).responder(move |spec| {
        let script = spec.args.last().map(String::as_str).unwrap_or_default();
        match (spec.program.as_str(), spec.args.first().map(String::as_str)) {
            ("limactl", Some("list")) => Some(Outcome::success().with_stdout(instances.clone())),
            ("limactl", Some("shell")) => {
                let guest = &spec.args[3];
                if script.contains("os-release") {
                    Some(Outcome::success().with_stdout(format!("{ubuntu}\n")))
                } else if script.ends_with("--version") {
                    Some(Outcome::success().with_stdout("rayx 1.0.0\n"))
                } else if script.contains("apt-get") && failing.contains(&guest.as_str()) {
                    Some(Outcome::failure(100))
                } else {
                    None
                }
            }
            _ => None,
        }
    })
}

fn refresh(
    options: &RefreshOptions,
    machine: &FakeMachine,
    runner: &mut Runner,
    prompter: Option<&mut ScriptedPrompter>,
) -> (RefreshReport, String) {
    let mut out = Vec::new();
    let report = refresh_guests(
        options,
        &host(Os::MacOs),
        runner,
        machine,
        prompter.map(|p| p as &mut dyn rayx_cli::setup::lima::sizing::Prompter),
        &mut out,
    );
    (report, String::from_utf8(out).expect("UTF-8"))
}

fn lines(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|s| !(s.program == "limactl" && s.args.first().is_some_and(|a| a == "list")))
        .map(|s| format!("{} {}", s.program, s.args.join(" ")))
        .collect()
}

fn script_of(spec: &CommandSpec) -> &str {
    spec.args.last().map(String::as_str).unwrap_or_default()
}

#[test]
fn each_current_guest_is_asked_about_and_refreshed_in_order() {
    let machine = machine(&[guest_json("lima", "rayx", "24.04", false)]);
    let mut runner = mac(vm("rayx", "Running"), "24.04", &[]);
    let mut prompter = ScriptedPrompter::always(true);

    let (report, out) = refresh(
        &options(false, false),
        &machine,
        &mut runner,
        Some(&mut prompter),
    );

    assert_eq!(
        report.guests,
        [("rayx".to_string(), GuestOutcome::Refreshed)]
    );
    assert_eq!(report.code(), 0);
    assert_eq!(prompter.asked, ["Update lima guest rayx?"]);
    let scripts: Vec<&str> = runner
        .specs()
        .iter()
        .filter(|s| s.args.first().is_some_and(|a| a == "shell"))
        .map(script_of)
        .collect();
    assert_eq!(scripts[0], ". /etc/os-release; echo $VERSION_ID");
    assert_eq!(scripts[1], "\"$HOME/.rayx/bin/rayx\" --version");
    assert_eq!(scripts[2], "sudo apt-get update && sudo apt-get upgrade");
    assert_eq!(scripts[3], "\"$HOME/.rayx/bin/rayx\" setup --update");
    assert_eq!(
        scripts.len(),
        4,
        "a current rayx is not reinstalled: {scripts:?}"
    );
    assert!(out.contains("==> lima: refreshing rayx"), "{out}");
}

#[test]
fn yes_asks_nothing_and_passes_y_to_apt_and_to_the_inner_update() {
    let machine = machine(&[guest_json("lima", "rayx", "24.04", false)]);
    let mut runner = mac(vm("rayx", "Running"), "24.04", &[]);
    let mut prompter = ScriptedPrompter::always(false);

    let (report, _) = refresh(
        &options(true, false),
        &machine,
        &mut runner,
        Some(&mut prompter),
    );

    assert_eq!(report.code(), 0);
    assert!(prompter.asked.is_empty());
    let shell: Vec<&str> = runner
        .specs()
        .iter()
        .filter(|s| s.args.first().is_some_and(|a| a == "shell"))
        .map(script_of)
        .collect();
    assert!(
        shell.contains(&"sudo apt-get update && sudo apt-get upgrade -y"),
        "{shell:?}"
    );
    assert!(
        shell.contains(&"\"$HOME/.rayx/bin/rayx\" setup --update --yes"),
        "{shell:?}"
    );
    assert!(
        runner
            .specs()
            .iter()
            .filter(|s| s.args.first().is_some_and(|a| a == "shell"))
            .all(|s| s.interactive
                || script_of(s).contains("--version")
                || script_of(s).contains("os-release")),
        "apt and the inner update run in this console"
    );
}

#[test]
fn a_declined_guest_or_no_terminal_is_reported_not_updated_with_exit_1() {
    let machine = machine(&[guest_json("lima", "rayx", "24.04", false)]);

    let mut runner = mac(vm("rayx", "Running"), "24.04", &[]);
    let mut no = ScriptedPrompter::always(false);
    let (report, _) = refresh(&options(false, false), &machine, &mut runner, Some(&mut no));
    assert!(matches!(report.guests[0].1, GuestOutcome::NotUpdated(_)));
    assert_eq!(report.code(), 1);
    assert!(
        lines(&runner).iter().all(|l| !l.contains("apt-get")),
        "{:?}",
        lines(&runner)
    );

    let mut runner = mac(vm("rayx", "Running"), "24.04", &[]);
    let mut silent = ScriptedPrompter::always(true);
    silent.terminal = false;
    let (report, _) = refresh(
        &options(false, false),
        &machine,
        &mut runner,
        Some(&mut silent),
    );
    assert!(matches!(report.guests[0].1, GuestOutcome::NotUpdated(_)));
    assert!(silent.asked.is_empty(), "nobody can answer");
    assert_eq!(report.code(), 1);
}

#[test]
fn a_stopped_vm_is_started_for_the_refresh_and_stopped_again() {
    let machine = machine(&[guest_json("lima", "rayx", "24.04", false)]);
    let mut runner = mac(vm("rayx", "Stopped"), "24.04", &[]);

    let (report, _) = refresh(&options(true, false), &machine, &mut runner, None);

    assert_eq!(report.code(), 0);
    let all = lines(&runner);
    let start = all
        .iter()
        .position(|l| l == "limactl start --tty=false rayx")
        .expect("start");
    let apt = all
        .iter()
        .position(|l| l.contains("apt-get update"))
        .expect("apt");
    let stop = all
        .iter()
        .position(|l| l == "limactl stop rayx")
        .expect("stop");
    assert!(start < apt && apt < stop, "{all:?}");

    let mut running = mac(vm("rayx", "Running"), "24.04", &[]);
    refresh(&options(true, false), &machine, &mut running, None);
    assert!(
        lines(&running)
            .iter()
            .all(|l| !l.contains("limactl start") && !l.contains("limactl stop")),
        "a running VM stays running"
    );
}

#[test]
fn a_failing_guest_is_reported_and_the_next_one_still_runs() {
    let machine = machine(&[
        guest_json("lima", "rayx", "24.04", false),
        guest_json("lima", "second", "24.04", false),
    ]);
    let instances = format!("{}{}", vm("rayx", "Stopped"), vm("second", "Running"));
    let mut runner = mac(instances, "24.04", &["rayx"]);

    let (report, _) = refresh(&options(true, false), &machine, &mut runner, None);

    assert!(
        matches!(report.guests[0].1, GuestOutcome::Failed(_)),
        "{:?}",
        report.guests
    );
    assert_eq!(
        report.guests[1],
        ("second".to_string(), GuestOutcome::Refreshed)
    );
    assert_eq!(report.code(), 1);
    assert!(
        lines(&runner).iter().any(|l| l == "limactl stop rayx"),
        "the VM started for the refresh is stopped even though it failed"
    );
    assert!(
        lines(&runner)
            .iter()
            .any(|l| l.contains("second") && l.contains("setup --update")),
        "{:?}",
        lines(&runner)
    );
}

#[test]
fn only_current_guests_of_this_hosts_kind_are_refreshed() {
    let machine = machine(&[
        guest_json("lima", "rayx", "24.04", false),
        guest_json("lima", "rayx-22.04", "22.04", true),
        guest_json("wsl", "Ubuntu-24.04", "24.04", false),
    ]);
    let instances = format!(
        "{}{}{}",
        vm("rayx", "Running"),
        vm("rayx-22.04", "Running"),
        vm("gpux", "Running")
    );
    let mut runner = mac(instances, "24.04", &[]);

    let (report, _) = refresh(&options(true, false), &machine, &mut runner, None);

    assert_eq!(report.guests.len(), 1, "{:?}", report.guests);
    for line in lines(&runner) {
        assert!(
            !line.contains("rayx-22.04") && !line.contains("gpux") && !line.contains("wsl"),
            "{line}"
        );
    }
}

#[test]
fn a_guest_older_than_the_pin_is_reported_outdated_and_left_alone() {
    let machine = machine(&[guest_json("lima", "rayx", "22.04", false)]);
    let mut runner = mac(vm("rayx", "Running"), "22.04", &[]);

    let (report, _) = refresh(&options(true, false), &machine, &mut runner, None);

    assert!(matches!(&report.guests[0].1, GuestOutcome::Outdated(why) if why.contains("22.04")));
    assert_eq!(report.code(), 1);
    assert!(lines(&runner).iter().all(|l| !l.contains("apt-get")));
    assert!(
        older_than_pin("22.04")
            && older_than_pin("9.10")
            && !older_than_pin("24.04")
            && !older_than_pin("26.04")
    );
}

#[test]
fn an_old_linux_rayx_is_replaced_by_the_release_installer_first() {
    let machine = machine(&[guest_json("lima", "rayx", "24.04", false)]);
    let mut runner = Runner::record().with_os(Os::MacOs).responder(|spec| {
        let script = spec.args.last().map(String::as_str).unwrap_or_default();
        match spec.args.first().map(String::as_str) {
            Some("list") => Some(Outcome::success().with_stdout(vm("rayx", "Running"))),
            Some("shell") if script.contains("os-release") => {
                Some(Outcome::success().with_stdout("24.04\n"))
            }
            Some("shell") if script.ends_with("--version") => {
                Some(Outcome::success().with_stdout("rayx 0.9.0\n"))
            }
            _ => None,
        }
    });

    let (report, _) = refresh(&options(true, false), &machine, &mut runner, None);

    assert_eq!(report.code(), 0);
    let scripts: Vec<&str> = runner.specs().iter().map(script_of).collect();
    let installer = scripts
        .iter()
        .position(|s| s.contains("rayx-cli-installer.sh"))
        .expect("installer");
    let apt = scripts
        .iter()
        .position(|s| s.contains("apt-get update"))
        .expect("apt");
    assert!(
        installer < apt,
        "the Linux rayx is current before the inner update runs"
    );
}

#[test]
fn check_reports_versions_and_commands_and_starts_nothing() {
    let machine = machine(&[
        guest_json("lima", "rayx", "24.04", false),
        guest_json("lima", "napping", "24.04", false),
    ]);
    let instances = format!("{}{}", vm("rayx", "Running"), vm("napping", "Stopped"));
    let mut runner = Runner::record().with_os(Os::MacOs).responder({
        let instances = instances.clone();
        move |spec| {
            let script = spec.args.last().map(String::as_str).unwrap_or_default();
            match spec.args.first().map(String::as_str) {
                Some("list") => Some(Outcome::success().with_stdout(instances.clone())),
                Some("shell") if script.contains("os-release") => {
                    Some(Outcome::success().with_stdout("24.04\n"))
                }
                Some("shell") if script.ends_with("--version") => {
                    Some(Outcome::success().with_stdout("rayx 0.9.0\n"))
                }
                Some("shell") if script.ends_with("--update --check") => {
                    Some(Outcome::success().with_stdout("Everything is installed.\n"))
                }
                _ => None,
            }
        }
    });
    let mut prompter = ScriptedPrompter::always(true);

    let (report, out) = refresh(
        &options(false, true),
        &machine,
        &mut runner,
        Some(&mut prompter),
    );

    assert!(prompter.asked.is_empty(), "--check asks nothing");
    assert!(
        out.contains("rayx in the guest: rayx 0.9.0 (this rayx is 1.0.0)"),
        "{out}"
    );
    assert!(out.contains("Ubuntu: 24.04 (pinned 24.04)"), "{out}");
    assert!(out.contains("Everything is installed."), "{out}");
    assert!(
        out.contains("napping is not running, so it is not inspected"),
        "{out}"
    );
    assert_eq!(
        report.guests[1],
        ("napping".to_string(), GuestOutcome::NotInspected)
    );
    assert_eq!(report.code(), 0);
    let all = lines(&runner);
    assert!(
        all.iter()
            .all(|l| !l.contains("limactl start") && !l.contains("limactl stop")),
        "{all:?}"
    );
    assert!(
        all.iter()
            .all(|l| !l.contains("napping") || l.contains("list")),
        "{all:?}"
    );
}

#[test]
fn a_recorded_guest_the_host_no_longer_has_is_reported() {
    let machine = machine(&[guest_json("lima", "gone", "24.04", false)]);
    let mut runner = mac(String::new(), "24.04", &[]);

    let (report, _) = refresh(&options(true, false), &machine, &mut runner, None);

    assert!(
        matches!(&report.guests[0].1, GuestOutcome::Failed(why) if why.contains("rayx lima remove gone"))
    );
    assert!(lines(&runner).is_empty());
}

#[test]
fn on_windows_the_current_wsl_distribution_is_refreshed_through_wsl_exec() {
    let distribution = Distribution {
        id: "{a}".into(),
        name: "Ubuntu-24.04".into(),
        base_path: PathBuf::from(r"C:\wsl\u"),
        version: Some(2),
        default_uid: Some(1000),
    };
    let machine = FakeMachine::new()
        .with_env("LOCALAPPDATA", r"C:\Users\dev\AppData\Local")
        .with_wsl_distributions(vec![distribution]);
    let path = rayx_cli::setup::records::guests_path(&machine, Os::Windows).expect("path");
    let machine = machine.with_text(
        path,
        &format!(
            r#"{{"version":1,"guests":[{}]}}"#,
            guest_json("wsl", "Ubuntu-24.04", "24.04", false)
        ),
    );
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let mut runner = Runner::record().with_os(Os::Windows).responder({
        let seen = seen.clone();
        move |spec| {
            seen.borrow_mut()
                .push(format!("{} {}", spec.program, spec.args.join(" ")));
            let script = spec.args.last().map(String::as_str).unwrap_or_default();
            if spec.args.contains(&"--running".to_string()) {
                Some(Outcome::success().with_stdout("Ubuntu-24.04\n"))
            } else if script.contains("os-release") {
                Some(Outcome::success().with_stdout("24.04\n"))
            } else if script.ends_with("--version") {
                Some(Outcome::success().with_stdout("rayx 1.0.0\n"))
            } else {
                None
            }
        }
    });
    let mut out = Vec::new();

    let report = refresh_guests(
        &options(true, false),
        &host(Os::Windows),
        &mut runner,
        &machine,
        None,
        &mut out,
    );

    assert_eq!(report.code(), 0, "{:?}", report.guests);
    let apt = runner
        .specs()
        .iter()
        .find(|s| script_of(s).contains("apt-get update"))
        .expect("apt");
    assert_eq!(apt.program, "wsl");
    assert_eq!(
        apt.args[..5],
        ["-d", "Ubuntu-24.04", "--exec", "bash", "-lc"]
    );
    assert!(seen.borrow().iter().all(|l| !l.contains("limactl")));
}

#[test]
fn a_linux_host_refreshes_no_guests() {
    let mut runner = Runner::record().with_os(Os::Linux);
    let mut out = Vec::new();

    let report = refresh_guests(
        &options(true, false),
        &host(Os::Linux),
        &mut runner,
        &FakeMachine::new(),
        None,
        &mut out,
    );

    assert!(report.guests.is_empty());
    assert!(runner.specs().is_empty());
}

#[test]
fn an_unparsable_guest_record_fails_the_refresh_instead_of_refreshing_nothing() {
    let machine = FakeMachine::new()
        .with_home("/Users/dev")
        .with_text("/Users/dev/.config/rayx/guests.json", "{ nope");
    let mut runner = mac(vm("rayx", "Running"), "24.04", &[]);

    let (report, _) = refresh(&options(true, false), &machine, &mut runner, None);

    assert!(report.unreadable);
    assert!(report.guests.is_empty());
    assert_eq!(report.code(), 1);
    assert!(runner.specs().is_empty());
}
