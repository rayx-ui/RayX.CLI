//! `rayx setup --update --vm <name>` (repeatable): only the named recorded current guests of this
//! host are refreshed, Lima and WSL alike, the host part is skipped, an unknown name is refused
//! with the recorded names before any command runs, and `--release` and `--check` combine.
//!
//! Every `limactl`, `wsl` and `curl` answer is scripted through a recording runner.

use std::path::PathBuf;

use clap::Parser;
use rayx_cli::cli::{Cli, Command, SetupArgs};
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::release;
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::lima::sizing::{Prompter, ScriptedPrompter};
use rayx_cli::setup::refresh::{GuestOutcome, RefreshOptions, RefreshReport, refresh_guests};
use rayx_cli::setup::update::run_with as update_run;
use rayx_cli::wsl::Distribution;

const HOME: &str = "/Users/dev";
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

fn parse(args: &[&str]) -> Result<SetupArgs, clap::Error> {
    let mut argv = vec!["rayx", "setup"];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv).map(|cli| match cli.command {
        Command::Setup(args) => args,
        other => panic!("not setup: {other:?}"),
    })
}

fn guest(kind: &str, name: &str) -> String {
    format!(
        r#"{{"kind":"{kind}","name":"{name}","ubuntu":"24.04","flags":["--web"],"clones":[],"retired":false}}"#
    )
}

fn retired(name: &str, migrating: bool) -> String {
    format!(
        r#"{{"kind":"lima","name":"{name}","ubuntu":"22.04","flags":[],"clones":[],"retired":true,"migrating":{migrating}}}"#
    )
}

fn record(guests: &[String]) -> String {
    format!(r#"{{"version":1,"guests":[{}]}}"#, guests.join(","))
}

fn mac_machine(guests: &[String]) -> FakeMachine {
    FakeMachine::new()
        .with_home(HOME)
        .with_text(format!("{HOME}/.config/rayx/guests.json"), &record(guests))
        .with_text(
            format!("{HOME}/.config/rayx/setup.json"),
            r#"{"version":1,"sets":["base","web"],"roots":[]}"#,
        )
}

fn instance(name: &str, status: &str) -> String {
    format!(
        "{{\"name\":\"{name}\",\"status\":\"{status}\",\"cpus\":6,\"memory\":{},\"disk\":{},\"dir\":\"/x/{name}\"}}\n",
        12 * GIB,
        100 * GIB
    )
}

/// A Mac whose VMs are `vms` (name, status); every guest's `rayx` reports 1.0.0 and curl answers
/// `published`.
fn mac(vms: &[(&str, &str)], published: bool) -> Runner {
    let list: String = vms.iter().map(|(n, s)| instance(n, s)).collect();
    Runner::record().with_os(Os::MacOs).responder(move |spec| {
        let script = spec.args.last().map(String::as_str).unwrap_or_default();
        match (spec.program.as_str(), spec.args.first().map(String::as_str)) {
            ("curl", _) => Some(Outcome::success().with_stdout(if published {
                "\nstatus=200\n"
            } else {
                "\nstatus=404\n"
            })),
            ("limactl", Some("list")) => Some(Outcome::success().with_stdout(list.clone())),
            ("limactl", Some("shell")) if script.contains("os-release") => {
                Some(Outcome::success().with_stdout("24.04\n"))
            }
            ("limactl", Some("shell")) if script.ends_with("--version") => {
                Some(Outcome::success().with_stdout("rayx 1.0.0\n"))
            }
            _ => None,
        }
    })
}

fn options(targets: &[&str], release: Option<&str>, yes: bool, check: bool) -> RefreshOptions {
    let release = release.map(|r| release::parse(r).expect("release"));
    RefreshOptions {
        version: release
            .as_ref()
            .map_or_else(|| "1.2.0".to_string(), ToString::to_string),
        release,
        targets: targets.iter().map(ToString::to_string).collect(),
        yes,
        check,
        lima_binary: None,
        wsl_binary: None,
    }
}

fn refresh(
    options: &RefreshOptions,
    os: Os,
    machine: &FakeMachine,
    runner: &mut Runner,
    prompter: Option<&mut ScriptedPrompter>,
) -> (RefreshReport, String) {
    let mut out = Vec::new();
    let report = refresh_guests(
        options,
        &host(os),
        runner,
        machine,
        prompter.map(|p| p as &mut dyn Prompter),
        &mut out,
    );
    (report, String::from_utf8(out).expect("UTF-8"))
}

/// The guests the guest scripts ran in (`limactl shell ... <name>` and `wsl -d <name>`).
fn guests_touched(runner: &Runner) -> Vec<String> {
    let mut names: Vec<String> = runner
        .specs()
        .iter()
        .filter_map(
            |s| match (s.program.as_str(), s.args.first().map(String::as_str)) {
                ("limactl", Some("shell")) => s.args.get(3).cloned(),
                ("limactl", Some("start" | "stop")) => s.args.last().cloned(),
                ("wsl", Some("-d")) => s.args.get(1).cloned(),
                _ => None,
            },
        )
        .collect();
    names.dedup();
    names.sort();
    names.dedup();
    names
}

fn scripts_in(runner: &Runner, guest: &str) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|s| {
            (s.program == "limactl"
                && s.args.first().is_some_and(|a| a == "shell")
                && s.args.get(3).is_some_and(|n| n == guest))
                || (s.program == "wsl" && s.args.get(1).is_some_and(|n| n == guest))
        })
        .filter_map(|s| s.args.last().cloned())
        .collect()
}

#[test]
fn vm_combines_with_update_and_release_on_the_command_line() {
    let args = parse(&["--update", "--vm", "app", "--release", "1.0.1", "--yes"]).expect("parses");
    assert_eq!(args.vm, ["app"]);
    assert_eq!(args.release.as_deref(), Some("1.0.1"));
    assert!(parse(&["--update", "--check", "--vm", "a", "--vm", "b"]).is_ok());
    assert!(parse(&["--update", "--wsl", "--vm", "a"]).is_err());
}

#[test]
fn only_the_named_guest_is_refreshed() {
    let machine = mac_machine(&[guest("lima", "rayx"), guest("lima", "app")]);
    let mut runner = mac(&[("rayx", "Running"), ("app", "Running")], true);

    let (report, _) = refresh(
        &options(&["app"], None, true, false),
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );

    assert_eq!(
        report.guests,
        [("app".to_string(), GuestOutcome::Refreshed)]
    );
    assert_eq!(report.code(), 0);
    assert_eq!(guests_touched(&runner), ["app"], "rayx was not touched");
    assert!(
        scripts_in(&runner, "app")
            .iter()
            .any(|s| s.contains("setup --update")),
        "{:?}",
        scripts_in(&runner, "app")
    );
}

#[test]
fn the_flag_repeats_and_without_it_every_current_guest_is_refreshed() {
    let machine = mac_machine(&[
        guest("lima", "rayx"),
        guest("lima", "app"),
        guest("lima", "api"),
    ]);
    let vms = [("rayx", "Running"), ("app", "Running"), ("api", "Running")];

    let mut runner = mac(&vms, true);
    let (report, _) = refresh(
        &options(&["api", "app"], None, true, false),
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );
    let names: Vec<&str> = report.guests.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["app", "api"], "in record order");
    assert_eq!(guests_touched(&runner), ["api", "app"]);

    let mut runner = mac(&vms, true);
    let (report, _) = refresh(
        &options(&[], None, true, false),
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );
    assert_eq!(
        report.guests.len(),
        3,
        "no --vm: all current guests as today"
    );
    assert_eq!(guests_touched(&runner), ["api", "app", "rayx"]);
}

#[test]
fn a_name_that_is_not_a_recorded_current_guest_is_refused_with_the_recorded_names() {
    let machine = mac_machine(&[
        guest("lima", "rayx"),
        guest("lima", "app"),
        retired("app-22.04", false),
        guest("wsl", "Ubuntu-24.04"),
    ]);
    for unknown in ["gpux", "app-22.04", "Ubuntu-24.04"] {
        let mut runner = mac(
            &[("rayx", "Running"), ("app", "Running"), ("gpux", "Running")],
            true,
        );

        let (report, _) = refresh(
            &options(&["app", unknown], None, true, false),
            Os::MacOs,
            &machine,
            &mut runner,
            None,
        );

        assert_eq!(
            report.refused.as_deref(),
            Some(
                format!(
                    "--vm {unknown}: not a recorded current guest of this host; recorded: rayx, app"
                )
                .as_str()
            ),
            "{unknown}"
        );
        assert!(report.guests.is_empty());
        assert_eq!(report.code(), 1);
        assert!(
            runner.specs().is_empty(),
            "{unknown}: no command ran, not even for the valid name: {:?}",
            runner.lines()
        );
    }
}

#[test]
fn setup_update_refuses_an_unknown_name_before_the_first_command_the_release_probe_included() {
    let machine = mac_machine(&[guest("lima", "app")]);
    let args = SetupArgs {
        update: true,
        vm: vec!["nope".into()],
        release: Some("1.0.1".into()),
        ..SetupArgs::default()
    };
    let mut runner = mac(&[("app", "Running")], true);

    let (code, out) = run_update(
        &args,
        &options(&["nope"], Some("1.0.1"), true, false),
        Os::MacOs,
        &machine,
        &mut runner,
    );

    assert_eq!(code, 1);
    assert!(out.is_empty(), "{out}");
    assert!(runner.specs().is_empty(), "{:?}", runner.lines());
}

fn run_update(
    args: &SetupArgs,
    options: &RefreshOptions,
    os: Os,
    machine: &FakeMachine,
    runner: &mut Runner,
) -> (u8, String) {
    let mut out = Vec::new();
    let mut user_path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut confirm = |_: &str| true;
    let mut prompter = ScriptedPrompter::always(true);
    let code = update_run(
        args,
        options,
        &host(os),
        runner,
        machine,
        &mut user_path,
        Some(&mut confirm),
        Some(&mut prompter),
        &mut out,
    );
    (code, String::from_utf8(out).expect("UTF-8"))
}

#[test]
fn naming_a_guest_skips_the_host_part_and_naming_none_runs_it() {
    let machine = mac_machine(&[guest("lima", "app")]);
    let mut args = SetupArgs {
        update: true,
        yes: true,
        vm: vec!["app".into()],
        ..SetupArgs::default()
    };

    let mut runner = mac(&[("app", "Running")], true);
    let (code, out) = run_update(
        &args,
        &options(&["app"], None, true, false),
        Os::MacOs,
        &machine,
        &mut runner,
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("the host part is skipped: --vm names guests only"),
        "{out}"
    );
    assert!(!out.contains("==> setup --update"), "{out}");
    assert!(
        runner.specs().iter().all(|s| s.program == "limactl"),
        "only the guest was driven, the host was not touched: {:?}",
        runner.lines()
    );

    args.vm.clear();
    let mut runner = mac(&[("app", "Running")], true);
    let (_, out) = run_update(
        &args,
        &options(&[], None, true, false),
        Os::MacOs,
        &machine,
        &mut runner,
    );
    assert!(
        out.contains("==> setup --update:"),
        "the host part runs: {out}"
    );
    assert!(
        runner.specs().iter().any(|s| s.program != "limactl"),
        "{:?}",
        runner.lines()
    );
}

#[test]
fn a_named_guest_gets_the_chosen_release_and_a_missing_one_stops_everything() {
    let machine = mac_machine(&[guest("lima", "rayx"), guest("lima", "app")]);

    let mut runner = mac(&[("rayx", "Running"), ("app", "Running")], true);
    let (report, _) = refresh(
        &options(&["app"], Some("1.1.0"), true, false),
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );
    assert_eq!(report.code(), 0);
    assert!(
        scripts_in(&runner, "app")
            .iter()
            .any(|s| s.contains("v1.1.0/rayx-cli-installer.sh")),
        "{:?}",
        scripts_in(&runner, "app")
    );
    assert!(scripts_in(&runner, "rayx").is_empty());

    let mut runner = mac(&[("rayx", "Running"), ("app", "Running")], false);
    let (report, _) = refresh(
        &options(&["app"], Some("9.9.9"), true, false),
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );
    assert_eq!(
        report.refused.as_deref(),
        Some("release 9.9.9 was not found")
    );
    assert!(guests_touched(&runner).is_empty());
}

#[test]
fn check_inspects_only_the_named_running_guests_and_starts_nothing() {
    let machine = mac_machine(&[
        guest("lima", "rayx"),
        guest("lima", "app"),
        guest("lima", "api"),
    ]);
    let mut runner = mac(
        &[("rayx", "Running"), ("app", "Running"), ("api", "Stopped")],
        true,
    );

    let (report, out) = refresh(
        &options(&["app", "api"], None, false, true),
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );

    assert_eq!(
        report.guests,
        [
            ("app".to_string(), GuestOutcome::Refreshed),
            ("api".to_string(), GuestOutcome::NotInspected)
        ]
    );
    assert!(out.contains("==> lima: checking app"), "{out}");
    assert!(!out.contains("checking rayx"), "{out}");
    assert!(
        out.contains("api is not running, so it is not inspected"),
        "{out}"
    );
    assert_eq!(
        guests_touched(&runner),
        ["app"],
        "nothing was started, rayx was not looked at"
    );
}

#[test]
fn a_retired_vms_interrupted_migration_is_reported_only_for_a_targeted_name() {
    let machine = mac_machine(&[guest("lima", "rayx"), retired("app-22.04", true)]);

    let mut runner = mac(&[("rayx", "Running"), ("app-22.04", "Stopped")], true);
    let (report, _) = refresh(
        &options(&["rayx"], None, true, false),
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );
    assert_eq!(
        report.guests,
        [("rayx".to_string(), GuestOutcome::Refreshed)]
    );

    let mut runner = mac(&[("rayx", "Running"), ("app-22.04", "Stopped")], true);
    let (report, _) = refresh(
        &options(&[], None, true, false),
        Os::MacOs,
        &machine,
        &mut runner,
        None,
    );
    assert!(
        report
            .guests
            .iter()
            .any(|(n, o)| n == "app" && matches!(o, GuestOutcome::Failed(_))),
        "{:?}",
        report.guests
    );
}

#[test]
fn wsl_guests_are_selected_by_their_recorded_names_too() {
    let distribution = |name: &str| Distribution {
        id: format!("{{{name}}}"),
        name: name.into(),
        base_path: PathBuf::from(format!(r"C:\wsl\{name}")),
        version: Some(2),
        default_uid: Some(1000),
    };
    let config = r"C:\Users\dev\AppData\Local";
    let machine = FakeMachine::new()
        .with_env("LOCALAPPDATA", config)
        .with_text(
            PathBuf::from(config).join("rayx/guests.json"),
            &record(&[guest("wsl", "Ubuntu-24.04"), guest("wsl", "Debian-Test")]),
        )
        .with_wsl_distributions(vec![
            distribution("Ubuntu-24.04"),
            distribution("Debian-Test"),
        ]);
    let mut runner = Runner::record().with_os(Os::Windows).responder(|spec| {
        let script = spec.args.last().map(String::as_str).unwrap_or_default();
        match spec.program.as_str() {
            "wsl" if spec.args.iter().any(|a| a == "--running") => Some(
                Outcome::success().with_stdout(
                    "Ubuntu-24.04\nDebian-Test\n"
                        .chars()
                        .flat_map(|c| [c, '\0'])
                        .collect::<String>(),
                ),
            ),
            "wsl" if script.contains("os-release") => {
                Some(Outcome::success().with_stdout("24.04\n"))
            }
            "wsl" if script.ends_with("--version") => {
                Some(Outcome::success().with_stdout("rayx 1.0.0\n"))
            }
            _ => None,
        }
    });

    let (report, _) = refresh(
        &options(&["Debian-Test"], None, true, false),
        Os::Windows,
        &machine,
        &mut runner,
        None,
    );

    assert_eq!(
        report.guests,
        [("Debian-Test".to_string(), GuestOutcome::Refreshed)]
    );
    assert_eq!(guests_touched(&runner), ["Debian-Test"]);

    let mut runner = Runner::record().with_os(Os::Windows);
    let (report, _) = refresh(
        &options(&["Ubuntu-99.04"], None, true, false),
        Os::Windows,
        &machine,
        &mut runner,
        None,
    );
    assert_eq!(
        report.refused.as_deref(),
        Some(
            "--vm Ubuntu-99.04: not a recorded current guest of this host; recorded: \
             Ubuntu-24.04, Debian-Test"
        )
    );
    assert!(runner.specs().is_empty());
}
