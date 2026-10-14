//! `--release` for guests: `rayx setup --lima|--wsl --release X` and `rayx setup --update
//! --release X` put exactly that release of the Linux `rayx` in the guest, after probing that it
//! exists, asking before an older release replaces a newer one, skipping the inner update for a
//! release that lacks it, and recording nothing.
//!
//! Every `limactl`, `wsl` and `curl` answer is scripted through a recording runner; nothing here
//! touches the network or a VM.

use std::path::PathBuf;

use clap::Parser;
use rayx_cli::cli::{Cli, Command, SetupArgs};
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Runner};
use rayx_cli::release::{self, Release};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::guest::{chosen_release, guest_version};
use rayx_cli::setup::lima::sizing::{Prompter, ScriptedPrompter};
use rayx_cli::setup::lima::{LimaError, LimaRequest, VmFlags, run_with as lima_run};
use rayx_cli::setup::records::read_guests_at;
use rayx_cli::setup::refresh::{GuestOutcome, RefreshOptions, RefreshReport, refresh_guests};
use rayx_cli::setup::update::run_with as update_run;
use rayx_cli::setup::wsl::{
    WslError, WslRequest, distribution, migrate_older_guest, run_with_prompter as wsl_run,
};
use rayx_cli::wsl::Distribution;

const HOME: &str = "/Users/dev";
const GIB: u64 = 1024 * 1024 * 1024;

fn r(text: &str) -> Release {
    release::parse(text).expect("release")
}

fn installer(version: &str) -> String {
    format!(
        "https://github.com/rayx-ui/RayX.CLI/releases/download/v{version}/rayx-cli-installer.sh"
    )
}

fn parse(args: &[&str]) -> Result<SetupArgs, clap::Error> {
    let mut argv = vec!["rayx", "setup"];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv).map(|cli| match cli.command {
        Command::Setup(args) => args,
        other => panic!("not setup: {other:?}"),
    })
}

/// What the scripted `curl --head` for a release answers.
#[derive(Clone, Copy, Debug)]
enum Published {
    Yes,
    No,
    Unreachable,
}

fn curl_answer(published: Published) -> Outcome {
    match published {
        Published::Yes => Outcome::success(),
        Published::No => Outcome::failure(22),
        Published::Unreachable => Outcome::failure(6),
    }
}

/// The scripts the guest received (`limactl shell ... bash -lc <script>` and `wsl ... bash -lc`).
fn guest_scripts(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|spec| {
            (spec.program == "limactl" && spec.args.first().is_some_and(|a| a == "shell"))
                || (spec.program == "wsl" && spec.args.iter().any(|a| a == "bash"))
        })
        .filter_map(|spec| spec.args.last().cloned())
        .collect()
}

/// The guest scripts that run a release installer.
fn installs(runner: &Runner) -> Vec<String> {
    guest_scripts(runner)
        .into_iter()
        .filter(|script| script.contains("rayx-cli-installer.sh"))
        .collect()
}

fn existence_probes(runner: &Runner) -> Vec<&CommandSpec> {
    runner
        .specs()
        .iter()
        .filter(|spec| spec.program == "curl")
        .collect()
}

fn is_version_probe(spec: &CommandSpec) -> bool {
    spec.args
        .last()
        .is_some_and(|script| script.ends_with("rayx\" --version"))
}

// ---------------------------------------------------------------------------------------------
// The command line
// ---------------------------------------------------------------------------------------------

#[test]
fn release_is_accepted_with_a_guest_target_and_refused_without_one() {
    for target in ["--lima", "--wsl", "--update"] {
        let args =
            parse(&[target, "--release", "1.0.1"]).unwrap_or_else(|e| panic!("{target}: {e}"));
        assert_eq!(args.release.as_deref(), Some("1.0.1"), "{target}");
    }
    assert!(parse(&["--lima", "--release", "1.0.1", "--all", "--check"]).is_ok());

    for bare in [
        parse(&["--release", "1.0.1"]),
        parse(&["--web", "--release", "1.0.1"]),
        parse(&["--check", "--release", "1.0.1"]),
    ] {
        let error = bare.expect_err("a release needs a guest target");
        assert_eq!(error.exit_code(), 2);
    }
    assert_eq!(
        parse(&["--lima"]).expect("plain").release,
        None,
        "without --release the running version is used"
    );
}

#[test]
fn the_update_release_keeps_the_conflicts_of_update() {
    for conflicting in [
        vec!["--web"],
        vec!["--all"],
        vec!["--wsl"],
        vec!["--lima"],
        vec!["--clone"],
        vec!["--vm-cpus", "4"],
    ] {
        let mut args = vec!["--update", "--release", "1.0.1"];
        args.extend(conflicting.iter().copied());
        assert!(
            parse(&args).is_err(),
            "{conflicting:?} still conflicts with --update --release"
        );
    }
    assert!(parse(&["--update", "--release", "1.0.1", "--yes", "--check"]).is_ok());
}

#[test]
fn a_chosen_release_is_validated_and_names_the_version_the_guest_gets() {
    let args = parse(&["--lima", "--release", "v1.0.1"]).expect("--lima");
    let chosen = chosen_release(&args).expect("valid");
    assert_eq!(chosen, Some(r("1.0.1")));
    assert_eq!(guest_version(chosen.as_ref()), "1.0.1");

    let plain = parse(&["--lima"]).expect("--lima");
    assert_eq!(chosen_release(&plain), Ok(None));
    assert_eq!(guest_version(None), env!("CARGO_PKG_VERSION"));

    for bad in ["latest", "1.0.0-rc.1", "1.0", ">=1.0.0", ""] {
        let args = parse(&["--lima", "--release", bad]).expect("clap takes any text");
        let error = chosen_release(&args).expect_err(bad);
        assert!(error.contains("is not a release name"), "{bad}: {error}");
        assert!(error.contains("1.0.1"), "an example is given: {error}");
    }
}

#[test]
fn a_bad_release_exits_1_before_anything_runs_for_every_guest_target() {
    for target in ["--lima", "--wsl", "--update"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_rayx"))
            .args(["setup", target, "--release", "latest", "--check"])
            .output()
            .expect("runs rayx");

        assert_eq!(output.status.code(), Some(1), "{target}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("is not a release name"),
            "{target}: {stderr}"
        );
        assert!(stderr.contains("1.0.1"), "{target}: {stderr}");
        assert!(
            output.stdout.is_empty(),
            "{target}: nothing was run or printed"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Lima
// ---------------------------------------------------------------------------------------------

fn macos() -> HostFacts {
    HostFacts {
        os: Os::MacOs,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn mac_machine() -> FakeMachine {
    FakeMachine::new()
        .with_home(HOME)
        .with_program("brew", "/opt/homebrew/bin/brew")
        .with_program("limactl", "/opt/homebrew/bin/limactl")
}

fn instance_line(name: &str, status: &str) -> String {
    format!(
        "{{\"name\":\"{name}\",\"status\":\"{status}\",\"cpus\":6,\"memory\":{},\"disk\":{},\
         \"dir\":\"{HOME}/.lima/{name}\",\"vmType\":\"vz\"}}\n",
        12 * GIB,
        100 * GIB
    )
}

/// A Mac with a running VM `rayx` whose Linux `rayx` reports `guest` (`None`: it has none).
fn mac(published: Published, guest: Option<&'static str>) -> Runner {
    Runner::record().with_os(Os::MacOs).responder(move |spec| {
        let script = spec.args.last().map(String::as_str).unwrap_or_default();
        match (spec.program.as_str(), spec.args.first().map(String::as_str)) {
            ("curl", _) => Some(curl_answer(published)),
            ("sw_vers", _) => Some(Outcome::success().with_stdout("14.5\n")),
            ("sysctl", _) => Some(Outcome::success().with_stdout("10\n34359738368\n")),
            ("df", _) => Some(Outcome::success().with_stdout(
                "Filesystem 1024-blocks Used Available Capacity iused ifree %iused Mounted on\n\
                 /dev/disk3s5 971350180 400000000 500000000 45% 1 2 0% /System/Volumes/Data\n",
            )),
            ("limactl", Some("--version")) => {
                Some(Outcome::success().with_stdout("limactl version 2.2.1"))
            }
            ("limactl", Some("list")) => {
                Some(Outcome::success().with_stdout(instance_line("rayx", "Running")))
            }
            ("limactl", Some("shell")) if script.contains("os-release") => {
                Some(Outcome::success().with_stdout("24.04\n"))
            }
            ("limactl", Some("shell")) if is_version_probe(spec) => Some(match guest {
                Some(version) => Outcome::success().with_stdout(format!("rayx {version}\n")),
                None => Outcome::failure(127),
            }),
            _ => None,
        }
    })
}

fn lima_request(release: Option<&str>) -> LimaRequest {
    let release = release.map(r);
    LimaRequest {
        flags: vec!["--web".to_string()],
        clone: None,
        binary_override: None,
        version: guest_version(release.as_ref()),
        release,
        vm: VmFlags::default(),
        check: false,
        yes: false,
    }
}

fn lima(
    request: &LimaRequest,
    runner: &mut Runner,
    prompter: Option<&mut ScriptedPrompter>,
) -> Result<String, LimaError> {
    let mut out = Vec::new();
    lima_run(
        request,
        &macos(),
        runner,
        &mac_machine(),
        None,
        prompter.map(|p| p as &mut dyn Prompter),
        &mut out,
    )
    .map(|_| String::from_utf8(out).expect("UTF-8"))
}

/// The commands that change a VM, Lima or the guest: everything but the reads.
fn lima_changes(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|spec| {
            let first = spec.args.first().map(String::as_str);
            match spec.program.as_str() {
                "limactl" => !matches!(first, Some("list" | "--version")) && first != Some("shell"),
                "sw_vers" | "sysctl" | "df" | "curl" => false,
                _ => true,
            }
        })
        .map(|spec| format!("{} {}", spec.program, spec.args.join(" ")))
        .collect()
}

#[test]
fn lima_gets_the_chosen_release_from_that_tags_own_installer() {
    let mut runner = mac(Published::Yes, None);

    lima(&lima_request(Some("1.0.1")), &mut runner, None).expect("sets up");

    let installs = installs(&runner);
    assert_eq!(installs.len(), 1, "{installs:?}");
    assert!(
        installs[0].contains(&format!("'{}'", installer("1.0.1"))),
        "{}",
        installs[0]
    );
    assert!(!installs[0].contains("v1.0.0"), "{}", installs[0]);
    let install = runner
        .specs()
        .iter()
        .find(|s| {
            s.program == "limactl"
                && s.args
                    .last()
                    .is_some_and(|a| a.contains("rayx-cli-installer.sh"))
        })
        .expect("install spec");
    assert_eq!(
        install.args[..5],
        ["shell", "--workdir", "/", "rayx", "--"],
        "through limactl shell into the VM"
    );
    let probes = existence_probes(&runner);
    assert_eq!(probes.len(), 1);
    assert_eq!(
        probes[0].args,
        [
            "--proto",
            "=https",
            "--tlsv1.2",
            "-sfIL",
            &installer("1.0.1")
        ]
    );
}

#[test]
fn without_release_the_running_version_is_used_and_nothing_is_probed() {
    let mut runner = mac(Published::Yes, None);

    lima(&lima_request(None), &mut runner, None).expect("sets up");

    let installs = installs(&runner);
    assert_eq!(installs.len(), 1);
    assert!(
        installs[0].contains(&installer(env!("CARGO_PKG_VERSION"))),
        "{}",
        installs[0]
    );
    assert!(existence_probes(&runner).is_empty());
}

#[test]
fn a_guest_that_already_reports_the_release_gets_nothing_installed() {
    let mut runner = mac(Published::Yes, Some("1.0.1"));

    lima(&lima_request(Some("1.0.1")), &mut runner, None).expect("sets up");

    assert!(installs(&runner).is_empty(), "{:?}", guest_scripts(&runner));
    let scripts = guest_scripts(&runner);
    assert!(
        scripts.iter().any(|s| s.contains("\" setup --web")),
        "the inner setup still runs: {scripts:?}"
    );
}

#[test]
fn an_older_guest_is_replaced_without_a_question_and_a_newer_one_after_a_yes() {
    let mut runner = mac(Published::Yes, Some("1.0.0"));
    let mut prompter = ScriptedPrompter::always(false);
    lima(
        &lima_request(Some("1.0.1")),
        &mut runner,
        Some(&mut prompter),
    )
    .expect("upgrade");
    assert_eq!(
        installs(&runner).len(),
        1,
        "an older guest gets the release"
    );
    assert!(prompter.asked.is_empty(), "{:?}", prompter.asked);

    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(true);
    lima(
        &lima_request(Some("1.0.1")),
        &mut runner,
        Some(&mut prompter),
    )
    .expect("downgrade");
    assert_eq!(
        prompter.asked,
        ["Install rayx 1.0.1 in rayx over 1.3.0, an older release?"]
    );
    assert_eq!(installs(&runner).len(), 1, "a newer guest is replaced");
}

#[test]
fn a_declined_downgrade_leaves_the_guest_as_it_is_and_fails() {
    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(false);

    let error = lima(
        &lima_request(Some("1.0.1")),
        &mut runner,
        Some(&mut prompter),
    )
    .expect_err("declined");

    assert!(matches!(error, LimaError::Declined(_)), "{error:?}");
    assert!(
        error.to_string().contains("keeps rayx 1.3.0"),
        "the run reports it: {error}"
    );
    assert!(installs(&runner).is_empty());
    assert!(
        !guest_scripts(&runner)
            .iter()
            .any(|s| s.contains("\" setup ")),
        "no inner setup after a refusal"
    );
}

#[test]
fn yes_skips_the_downgrade_question_and_no_terminal_without_yes_refuses() {
    let mut yes = lima_request(Some("1.0.1"));
    yes.yes = true;
    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(false);
    lima(&yes, &mut runner, Some(&mut prompter)).expect("--yes");
    assert!(prompter.asked.is_empty(), "--yes does not ask");
    assert_eq!(installs(&runner).len(), 1);

    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let mut silent = ScriptedPrompter::always(true);
    silent.terminal = false;
    let error = lima(&lima_request(Some("1.0.1")), &mut runner, Some(&mut silent))
        .expect_err("no terminal");
    assert!(silent.asked.is_empty(), "nobody can answer");
    assert!(
        error
            .to_string()
            .contains("needs an answer: repeat it with `--yes` to confirm without a terminal"),
        "{error}"
    );
    assert!(installs(&runner).is_empty());

    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let error = lima(&lima_request(Some("1.0.1")), &mut runner, None).expect_err("no prompter");
    assert!(
        error
            .to_string()
            .contains("needs an answer: repeat it with `--yes` to confirm without a terminal"),
        "{error}"
    );
}

#[test]
fn a_missing_or_unreachable_release_stops_before_any_change() {
    let mut runner = mac(Published::No, Some("1.0.0"));
    let error = lima(&lima_request(Some("9.9.9")), &mut runner, None).expect_err("missing");
    assert_eq!(error.to_string(), "release 9.9.9 was not found");
    assert_eq!(lima_changes(&runner), Vec::<String>::new());
    assert!(guest_scripts(&runner).is_empty(), "nothing ran in the VM");

    let mut runner = mac(Published::Unreachable, Some("1.0.0"));
    let error = lima(&lima_request(Some("9.9.9")), &mut runner, None).expect_err("unknown");
    assert_eq!(
        error.to_string(),
        "cannot check whether release 9.9.9 exists: curl exited with 6"
    );
    assert_eq!(lima_changes(&runner), Vec::<String>::new());
    assert!(guest_scripts(&runner).is_empty());
}

#[test]
fn check_still_probes_the_release_and_only_prints_the_downgrade_subject() {
    let mut check = lima_request(Some("1.0.1"));
    check.check = true;
    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(false);

    let out = lima(&check, &mut runner, Some(&mut prompter)).expect("--check");

    assert_eq!(existence_probes(&runner).len(), 1, "the probe only reads");
    assert!(prompter.asked.is_empty(), "--check asks nothing");
    assert!(
        out.contains(
            "rayx has rayx 1.3.0; installing 1.0.1 is a downgrade and would ask: Install rayx 1.0.1 in rayx over 1.3.0, an older release?"
        ),
        "{out}"
    );

    let mut missing = mac(Published::No, None);
    let error = lima(&check, &mut missing, None).expect_err("missing under --check");
    assert_eq!(error.to_string(), "release 1.0.1 was not found");
    assert_eq!(existence_probes(&missing).len(), 1);
}

#[test]
fn a_release_and_a_binary_override_never_combine() {
    let mut request = lima_request(Some("1.0.1"));
    request.binary_override = Some("/Users/dev/rayx-linux".to_string());
    let mut runner = mac(Published::Yes, None);

    let error = lima(&request, &mut runner, None).expect_err("both");

    assert_eq!(
        error.to_string(),
        "RAYX_LIMA_BINARY is set and --release 1.0.1 names a release: choose one"
    );
    assert_eq!(lima_changes(&runner), Vec::<String>::new());
    assert!(
        existence_probes(&runner).is_empty(),
        "refused before the network is asked"
    );
}

#[test]
fn a_chosen_release_is_recorded_nowhere() {
    let dir = tempfile::tempdir().expect("temp dir");
    let machine = FakeMachine::new()
        .with_home(HOME)
        .with_env("XDG_CONFIG_HOME", &dir.path().display().to_string());
    let request = lima_request(Some("1.0.1"));
    let mut runner = mac(Published::Yes, None);
    let outcome = {
        let mut out = Vec::new();
        lima_run(
            &request,
            &macos(),
            &mut runner,
            &mac_machine(),
            None,
            None,
            &mut out,
        )
        .expect("sets up")
    };

    rayx_cli::setup::lima::record_outcome(&request, &macos(), &machine, &mut runner, &outcome);

    let text = std::fs::read_to_string(dir.path().join("rayx/guests.json")).expect("recorded");
    assert!(
        text.contains("\"name\": \"rayx\"") || text.contains("\"name\":\"rayx\""),
        "{text}"
    );
    assert!(
        !text.contains("1.0.1") && !text.to_lowercase().contains("release"),
        "the choice is not stored: {text}"
    );
    let file = read_guests_at(&dir.path().join("rayx/guests.json")).expect("reads");
    assert!(file.find("lima", "rayx").is_some());
}

// ---------------------------------------------------------------------------------------------
// WSL
// ---------------------------------------------------------------------------------------------

fn windows() -> HostFacts {
    HostFacts {
        os: Os::Windows,
        arch: Arch::X64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn wsl_machine() -> FakeMachine {
    FakeMachine::new().with_wsl_distributions(vec![Distribution {
        id: "{abc}".into(),
        name: distribution(),
        base_path: PathBuf::from(r"C:\Users\dev\AppData\Local\wsl\ubuntu"),
        version: Some(2),
        default_uid: Some(1000),
    }])
}

/// Windows with WSL 2 and a distribution whose Linux `rayx` reports `guest`.
fn windows_runner(published: Published, guest: Option<&'static str>) -> Runner {
    Runner::record()
        .with_os(Os::Windows)
        .responder(move |spec| match spec.program.as_str() {
            "curl" => Some(curl_answer(published)),
            "wsl" if spec.args == ["--version"] => Some(
                Outcome::success().with_stdout(
                    "WSL version: 2.6.1"
                        .chars()
                        .flat_map(|c| [c, '\0'])
                        .collect::<String>(),
                ),
            ),
            "wsl" if is_version_probe(spec) => Some(match guest {
                Some(version) => Outcome::success().with_stdout(format!("rayx {version}\n")),
                None => Outcome::failure(127),
            }),
            _ => None,
        })
}

fn wsl_request(release: Option<&str>) -> WslRequest {
    let release = release.map(r);
    WslRequest {
        flags: vec!["--web".to_string()],
        clone: None,
        binary_override: None,
        version: guest_version(release.as_ref()),
        release,
        windows_gpu: None,
    }
}

fn wsl(
    request: &WslRequest,
    runner: &mut Runner,
    prompter: Option<&mut ScriptedPrompter>,
) -> Result<String, WslError> {
    let mut out = Vec::new();
    wsl_run(
        request,
        &windows(),
        runner,
        &wsl_machine(),
        None,
        prompter.map(|p| p as &mut dyn Prompter),
        &mut out,
    )
    .map(|()| String::from_utf8(out).expect("UTF-8"))
}

/// The commands that change WSL or the distribution (everything but the reads and guest probes).
fn wsl_changes(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|spec| spec.program != "curl")
        .filter(|spec| !(spec.program == "wsl" && spec.args == ["--version"]))
        .filter(|spec| !(spec.program == "wsl" && spec.args.iter().any(|a| a == "bash")))
        .map(|spec| format!("{} {}", spec.program, spec.args.join(" ")))
        .collect()
}

#[test]
fn wsl_gets_the_chosen_release_from_that_tags_own_installer() {
    let mut runner = windows_runner(Published::Yes, None);

    wsl(&wsl_request(Some("1.0.1")), &mut runner, None).expect("sets up");

    let installs = installs(&runner);
    assert_eq!(installs.len(), 1, "{installs:?}");
    assert!(installs[0].contains(&installer("1.0.1")), "{}", installs[0]);
    assert_eq!(existence_probes(&runner).len(), 1);

    let mut current = windows_runner(Published::Yes, Some("1.0.1"));
    wsl(&wsl_request(Some("1.0.1")), &mut current, None).expect("sets up");
    assert!(
        installs_none(&current),
        "a guest that reports the release gets nothing"
    );
}

fn installs_none(runner: &Runner) -> bool {
    installs(runner).is_empty()
}

#[test]
fn wsl_asks_before_a_downgrade_and_refuses_without_an_answer() {
    let mut runner = windows_runner(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(true);
    wsl(
        &wsl_request(Some("1.0.1")),
        &mut runner,
        Some(&mut prompter),
    )
    .expect("yes");
    assert_eq!(
        prompter.asked,
        [format!(
            "Install rayx 1.0.1 in {} over 1.3.0, an older release?",
            distribution()
        )]
    );
    assert_eq!(installs(&runner).len(), 1);

    let mut runner = windows_runner(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(false);
    let error = wsl(
        &wsl_request(Some("1.0.1")),
        &mut runner,
        Some(&mut prompter),
    )
    .expect_err("declined");
    assert!(error.to_string().contains("keeps rayx 1.3.0"), "{error}");
    assert!(installs(&runner).is_empty());

    let mut runner = windows_runner(Published::Yes, Some("1.3.0"));
    let error = wsl(&wsl_request(Some("1.0.1")), &mut runner, None).expect_err("nobody to ask");
    assert!(
        error
            .to_string()
            .contains("needs an answer: repeat it with `--yes` to confirm without a terminal"),
        "{error}"
    );

    let mut yes = wsl_request(Some("1.0.1"));
    yes.flags.push("--yes".to_string());
    let mut runner = windows_runner(Published::Yes, Some("1.3.0"));
    wsl(&yes, &mut runner, None).expect("--yes needs no answer");
    assert_eq!(installs(&runner).len(), 1);
}

#[test]
fn wsl_stops_before_any_change_for_a_missing_release_or_an_override() {
    let mut runner = windows_runner(Published::No, None);
    let error = wsl(&wsl_request(Some("9.9.9")), &mut runner, None).expect_err("missing");
    assert_eq!(error.to_string(), "release 9.9.9 was not found");
    assert_eq!(wsl_changes(&runner), Vec::<String>::new());
    assert!(guest_scripts(&runner).is_empty());

    let mut runner = windows_runner(Published::Unreachable, None);
    let error = wsl(&wsl_request(Some("9.9.9")), &mut runner, None).expect_err("unknown");
    assert_eq!(
        error.to_string(),
        "cannot check whether release 9.9.9 exists: curl exited with 6"
    );
    assert_eq!(wsl_changes(&runner), Vec::<String>::new());

    let mut request = wsl_request(Some("1.0.1"));
    request.binary_override = Some(PathBuf::from(r"C:\build\rayx"));
    let mut runner = windows_runner(Published::Yes, None);
    let error = wsl(&request, &mut runner, None).expect_err("both");
    assert_eq!(
        error.to_string(),
        "RAYX_WSL_BINARY is set and --release 1.0.1 names a release: choose one"
    );
    assert_eq!(wsl_changes(&runner), Vec::<String>::new());
}

#[test]
fn wsl_probes_the_release_before_an_older_distribution_is_replaced() {
    let machine = FakeMachine::new()
        .with_env("LOCALAPPDATA", r"C:\Users\dev\AppData\Local")
        .with_text(
            PathBuf::from(r"C:\Users\dev\AppData\Local").join("rayx/guests.json"),
            r#"{"version":1,"guests":[{"kind":"wsl","name":"Ubuntu-22.04","ubuntu":"22.04","flags":["--web"],"clones":[],"retired":false}]}"#,
        );
    let mut runner = windows_runner(Published::No, None);
    let mut out = Vec::new();

    let error = migrate_older_guest(
        &wsl_request(Some("9.9.9")),
        false,
        true,
        &windows(),
        &mut runner,
        &machine,
        None,
        &mut out,
    )
    .expect_err("the release is missing");

    assert_eq!(error, "release 9.9.9 was not found");
    assert_eq!(
        wsl_changes(&runner),
        Vec::<String>::new(),
        "no distribution was installed or retired"
    );
    assert!(
        runner
            .specs()
            .iter()
            .all(|spec| !spec.args.iter().any(|a| a == "-d")),
        "the old distribution was not even asked a question (which starts it): {:?}",
        runner.lines()
    );
}

// ---------------------------------------------------------------------------------------------
// setup --update --release
// ---------------------------------------------------------------------------------------------

fn host(os: Os) -> HostFacts {
    HostFacts {
        os,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn recorded_guest(kind: &str, name: &str) -> String {
    format!(
        r#"{{"kind":"{kind}","name":"{name}","ubuntu":"24.04","flags":["--web"],"clones":[],"retired":false}}"#
    )
}

fn refresh_machine() -> FakeMachine {
    FakeMachine::new().with_home(HOME).with_text(
        format!("{HOME}/.config/rayx/guests.json"),
        &format!(
            r#"{{"version":1,"guests":[{}]}}"#,
            recorded_guest("lima", "rayx")
        ),
    )
}

fn options(release: Option<&str>, yes: bool, check: bool) -> RefreshOptions {
    let release = release.map(r);
    RefreshOptions {
        version: release
            .as_ref()
            .map_or_else(|| "1.2.0".to_string(), Release::to_string),
        release,
        yes,
        check,
        lima_binary: None,
        wsl_binary: None,
    }
}

fn refresh(
    options: &RefreshOptions,
    runner: &mut Runner,
    prompter: Option<&mut ScriptedPrompter>,
) -> (RefreshReport, String) {
    let mut out = Vec::new();
    let report = refresh_guests(
        options,
        &host(Os::MacOs),
        runner,
        &refresh_machine(),
        prompter.map(|p| p as &mut dyn Prompter),
        &mut out,
    );
    (report, String::from_utf8(out).expect("UTF-8"))
}

fn shell_scripts(runner: &Runner) -> Vec<String> {
    guest_scripts(runner)
}

#[test]
fn the_inner_update_runs_for_a_release_that_has_it_and_is_skipped_for_one_that_lacks_it() {
    let mut runner = mac(Published::Yes, Some("1.0.0"));
    let (report, _) = refresh(&options(Some("1.1.0"), true, false), &mut runner, None);
    assert_eq!(
        report.guests,
        [("rayx".to_string(), GuestOutcome::Refreshed)]
    );
    let scripts = shell_scripts(&runner);
    assert!(
        scripts.contains(&"\"$HOME/.rayx/bin/rayx\" setup --update --yes".to_string()),
        "{scripts:?}"
    );
    assert!(
        scripts.iter().any(|s| s.contains(&installer("1.1.0"))),
        "that release's installer: {scripts:?}"
    );

    let mut runner = mac(Published::Yes, Some("1.1.0"));
    let (report, out) = refresh(&options(Some("1.0.0"), true, false), &mut runner, None);
    assert_eq!(
        report.guests,
        [("rayx".to_string(), GuestOutcome::Refreshed)],
        "the guest counts as refreshed"
    );
    assert_eq!(report.code(), 0);
    assert!(
        out.contains("rayx 1.0.0 predates the guest refresh: the inner update was skipped"),
        "{out}"
    );
    let scripts = shell_scripts(&runner);
    assert!(
        scripts.iter().any(|s| s.contains("apt-get upgrade -y")),
        "apt still upgrades: {scripts:?}"
    );
    assert!(
        !scripts.iter().any(|s| s.contains("setup --update")),
        "no inner update for 1.0.0: {scripts:?}"
    );
    assert!(
        scripts.iter().any(|s| s.contains(&installer("1.0.0"))),
        "the older release is installed: {scripts:?}"
    );
}

#[test]
fn a_later_plain_refresh_brings_the_guest_back_to_the_running_version() {
    let mut runner = mac(Published::Yes, Some("1.2.0"));
    refresh(&options(Some("1.1.0"), true, false), &mut runner, None);

    // The guest now has 1.1.0; nothing remembers the choice.
    let mut later = mac(Published::Yes, Some("1.1.0"));
    let (report, _) = refresh(&options(None, true, false), &mut later, None);

    assert_eq!(report.code(), 0);
    assert!(
        shell_scripts(&later)
            .iter()
            .any(|s| s.contains(&installer("1.2.0"))),
        "the host's running version is installed again: {:?}",
        shell_scripts(&later)
    );
    assert!(existence_probes(&later).is_empty(), "no release, no probe");
}

#[test]
fn a_missing_release_stops_the_refresh_before_any_guest_is_touched() {
    for (published, why) in [
        (Published::No, "release 9.9.9 was not found"),
        (
            Published::Unreachable,
            "cannot check whether release 9.9.9 exists: curl exited with 6",
        ),
    ] {
        let mut runner = mac(published, Some("1.0.0"));
        let mut prompter = ScriptedPrompter::always(true);

        let (report, _) = refresh(
            &options(Some("9.9.9"), false, false),
            &mut runner,
            Some(&mut prompter),
        );

        assert_eq!(report.refused.as_deref(), Some(why));
        assert!(report.guests.is_empty());
        assert_eq!(report.code(), 1);
        assert!(prompter.asked.is_empty(), "no question before the probe");
        assert!(guest_scripts(&runner).is_empty());
        assert_eq!(lima_changes(&runner), Vec::<String>::new());
    }
}

#[test]
fn a_binary_override_and_a_release_never_combine_in_the_refresh() {
    let mut runner = mac(Published::Yes, Some("1.0.0"));
    let mut with_override = options(Some("1.0.1"), true, false);
    with_override.lima_binary = Some("/Users/dev/rayx-linux".to_string());

    let (report, _) = refresh(&with_override, &mut runner, None);

    assert_eq!(
        report.refused.as_deref(),
        Some("RAYX_LIMA_BINARY is set and --release 1.0.1 names a release: choose one")
    );
    assert_eq!(report.code(), 1);
    assert!(guest_scripts(&runner).is_empty());
    assert!(existence_probes(&runner).is_empty());
}

#[test]
fn the_refresh_asks_before_a_downgrade_and_a_no_stops_before_apt() {
    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(false).with_confirms([true, false]);

    let (report, _) = refresh(
        &options(Some("1.1.0"), false, false),
        &mut runner,
        Some(&mut prompter),
    );

    assert_eq!(
        prompter.asked,
        [
            "Update lima guest rayx?",
            "Install rayx 1.1.0 in rayx over 1.3.0, an older release?"
        ]
    );
    assert!(
        matches!(report.guests[0].1, GuestOutcome::NotUpdated(_)),
        "{:?}",
        report.guests
    );
    assert_eq!(report.code(), 1);
    let scripts = shell_scripts(&runner);
    assert!(
        !scripts.iter().any(|s| s.contains("apt-get")),
        "{scripts:?}"
    );
    assert!(installs(&runner).is_empty(), "the guest keeps its rayx");

    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(true);
    let (report, _) = refresh(
        &options(Some("1.1.0"), false, false),
        &mut runner,
        Some(&mut prompter),
    );
    assert_eq!(report.code(), 0);
    assert_eq!(
        installs(&runner).len(),
        1,
        "a yes installs the older release"
    );

    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(false);
    let (report, _) = refresh(
        &options(Some("1.1.0"), true, false),
        &mut runner,
        Some(&mut prompter),
    );
    assert_eq!(report.code(), 0);
    assert!(prompter.asked.is_empty(), "--yes asks nothing");
    assert_eq!(installs(&runner).len(), 1);
}

#[test]
fn the_refresh_check_probes_prints_the_downgrade_subject_and_skips_the_missing_inner_check() {
    let mut runner = mac(Published::Yes, Some("1.3.0"));
    let mut prompter = ScriptedPrompter::always(false);

    let (report, out) = refresh(
        &options(Some("1.0.0"), false, true),
        &mut runner,
        Some(&mut prompter),
    );

    assert_eq!(existence_probes(&runner).len(), 1);
    assert!(prompter.asked.is_empty(), "--check asks nothing");
    assert!(
        out.contains("Install rayx 1.0.0 in rayx over 1.3.0, an older release?"),
        "{out}"
    );
    assert!(
        out.contains("rayx 1.0.0 predates the guest refresh: the inner update was skipped"),
        "{out}"
    );
    assert_eq!(report.code(), 0);
    assert!(
        !shell_scripts(&runner)
            .iter()
            .any(|s| s.contains("setup --update")),
        "the old rayx has no `setup --update --check` to ask"
    );
}

#[test]
fn nothing_about_the_release_is_written_by_a_refresh() {
    let dir = tempfile::tempdir().expect("temp dir");
    let record = dir.path().join("rayx/guests.json");
    let text = format!(
        r#"{{"version":1,"guests":[{}]}}"#,
        recorded_guest("lima", "rayx")
    );
    std::fs::create_dir_all(record.parent().expect("dir")).expect("dir");
    std::fs::write(&record, &text).expect("record");
    let machine = FakeMachine::new()
        .with_home(HOME)
        .with_env("XDG_CONFIG_HOME", &dir.path().display().to_string())
        .with_text(&record, &text);
    let mut runner = mac(Published::Yes, Some("1.0.0"));
    let mut out = Vec::new();

    let report = refresh_guests(
        &options(Some("1.1.0"), true, false),
        &host(Os::MacOs),
        &mut runner,
        &machine,
        None,
        &mut out,
    );

    assert_eq!(
        report.guests,
        [("rayx".to_string(), GuestOutcome::Refreshed)]
    );
    assert_eq!(
        std::fs::read_to_string(&record).expect("file"),
        text,
        "the record is byte for byte what it was"
    );
    let left: Vec<_> = std::fs::read_dir(dir.path().join("rayx"))
        .expect("dir")
        .collect();
    assert_eq!(left.len(), 1, "nothing else was written next to it");
}

// ---------------------------------------------------------------------------------------------
// setup --update --release: the order of the host part, the probe and the refresh
// ---------------------------------------------------------------------------------------------

fn update_args(release: &str, check: bool) -> SetupArgs {
    SetupArgs {
        update: true,
        check,
        release: Some(release.to_string()),
        ..SetupArgs::default()
    }
}

/// A machine whose `setup.json` has work for the host part, and one recorded Lima VM.
fn machine_with_host_work() -> FakeMachine {
    refresh_machine().with_text(
        format!("{HOME}/.config/rayx/setup.json"),
        r#"{"version":1,"sets":["base","web"],"roots":[]}"#,
    )
}

fn update_run_with(
    args: &SetupArgs,
    options: &RefreshOptions,
    os: Os,
    runner: &mut Runner,
    machine: &FakeMachine,
) -> (u8, String) {
    let mut out = Vec::new();
    let mut user_path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let mut prompter = ScriptedPrompter::always(true);
    let mut confirm = |_: &str| true;
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
fn a_missing_release_stops_setup_update_before_the_host_part_runs() {
    for (published, why) in [
        (Published::No, "release 9.9.9 was not found"),
        (
            Published::Unreachable,
            "cannot check whether release 9.9.9 exists: curl exited with 6",
        ),
    ] {
        let mut runner = mac(published, Some("1.0.0"));

        let (code, out) = update_run_with(
            &update_args("9.9.9", false),
            &options(Some("9.9.9"), true, false),
            Os::MacOs,
            &mut runner,
            &machine_with_host_work(),
        );

        assert_eq!(code, 1, "{why}");
        assert!(out.is_empty(), "nothing was reported as done: {out}");
        let programs: Vec<&str> = runner.specs().iter().map(|s| s.program.as_str()).collect();
        assert_eq!(
            programs,
            ["curl"],
            "{why}: only the probe ran, neither the host part nor a guest ({:?})",
            runner.lines()
        );
    }
}

#[test]
fn setup_update_probes_the_release_before_the_host_part_and_the_refresh_follows() {
    let mut runner = mac(Published::Yes, Some("1.0.0"));
    let mut check = options(Some("1.1.0"), true, true);
    check.yes = true;

    let (code, out) = update_run_with(
        &update_args("1.1.0", true),
        &check,
        Os::MacOs,
        &mut runner,
        &machine_with_host_work(),
    );

    let _ = code; // the fake Mac lacks Homebrew, so the host part reports it under --check
    let lines = runner.lines();
    assert!(
        lines[0].starts_with("curl ") && lines[0].contains(&installer("1.1.0")),
        "the probe comes first: {lines:?}"
    );
    assert!(
        lines.iter().skip(1).any(|l| l.contains("limactl")),
        "then the work: {lines:?}"
    );
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("curl ")).count(),
        2,
        "the refresh probes again, which only reads: {lines:?}"
    );
    assert!(!out.contains("nothing recorded"), "{out}");
}

#[test]
fn setup_update_refuses_a_release_with_the_override_of_its_host_before_anything_runs() {
    let mut runner = mac(Published::Yes, Some("1.0.0"));
    let mut with_override = options(Some("1.0.1"), true, false);
    with_override.lima_binary = Some("/Users/dev/rayx-linux".to_string());

    let (code, _) = update_run_with(
        &update_args("1.0.1", false),
        &with_override,
        Os::MacOs,
        &mut runner,
        &machine_with_host_work(),
    );
    assert_eq!(code, 1);
    assert!(runner.specs().is_empty(), "{:?}", runner.lines());

    let mut windows_runner = windows_runner(Published::Yes, None);
    let mut with_override = options(Some("1.0.1"), true, false);
    with_override.wsl_binary = Some("/mnt/c/build/rayx".to_string());
    let (code, _) = update_run_with(
        &update_args("1.0.1", false),
        &with_override,
        Os::Windows,
        &mut windows_runner,
        &machine_with_host_work(),
    );
    assert_eq!(code, 1);
    assert!(windows_runner.specs().is_empty());
}
