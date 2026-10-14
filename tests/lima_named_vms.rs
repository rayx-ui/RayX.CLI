//! Named Lima VMs: `rayx setup --lima --vm <name>` creates, edits, migrates, refreshes, removes and
//! reports the VM under its own name (default `rayx`, byte for byte as before), and never adopts a
//! VM that exists in Lima but is not in `guests.json`.
//!
//! GitHub's macOS runners cannot run VMs: every `limactl` answer here is scripted through a
//! recording runner.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use clap::Parser;
use rayx_cli::cli::{Cli, Command, SetupArgs};
use rayx_cli::doctor::report_for;
use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
use rayx_cli::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::guest::GuestKind;
use rayx_cli::setup::lima::sizing::ScriptedPrompter;
use rayx_cli::setup::lima::{
    LimaError, LimaOutcome, LimaRequest, VmFlags, VmName, chosen_name, parse_instances,
    record_outcome, run_with,
};
use rayx_cli::setup::migrate::pending_lima;
use rayx_cli::setup::records::{GuestsFile, VmRecord, read_guests_at};
use rayx_cli::setup::refresh::{GuestOutcome, RefreshOptions, refresh_guests};
use rayx_cli::setup::remove::remove_guest;
use rayx_cli::setup::{FakeMachine, PlanEnv};

const HOME: &str = "/Users/dev";
const GIB: u64 = 1024 * 1024 * 1024;

fn name(text: &str) -> VmName {
    VmName::parse(text).expect("a usable name")
}

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

// ---------------------------------------------------------------------------------------------
// A scripted Lima
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Vm {
    name: String,
    status: &'static str,
    version: &'static str,
    memory_gib: u64,
}

/// Instances that `rename`, `create`, `start` and `stop` change.
#[derive(Clone)]
struct Lab {
    vms: Rc<RefCell<Vec<Vm>>>,
}

impl Lab {
    fn new(vms: &[(&str, &'static str, &'static str)]) -> Self {
        Self::with_memory(
            &vms.iter()
                .map(|(n, s, v)| (*n, *s, *v, 12))
                .collect::<Vec<_>>(),
        )
    }

    fn with_memory(vms: &[(&str, &'static str, &'static str, u64)]) -> Self {
        Self {
            vms: Rc::new(RefCell::new(
                vms.iter()
                    .map(|(name, status, version, memory_gib)| Vm {
                        name: name.to_string(),
                        status,
                        version,
                        memory_gib: *memory_gib,
                    })
                    .collect(),
            )),
        }
    }

    fn runner(&self) -> Runner {
        let lab = self.clone();
        Runner::record()
            .with_os(Os::MacOs)
            .responder(move |spec| lab.answer(spec))
    }

    fn answer(&self, spec: &CommandSpec) -> Option<Outcome> {
        let args: Vec<&str> = spec.args.iter().map(String::as_str).collect();
        match (spec.program.as_str(), args.as_slice()) {
            ("sw_vers", _) => Some(Outcome::success().with_stdout("14.5\n")),
            ("sysctl", _) => Some(Outcome::success().with_stdout("10\n34359738368\n")),
            ("df", _) => Some(Outcome::success().with_stdout("h\n/dev/d 1 1 500000000 1% /\n")),
            ("limactl", ["--version"]) => {
                Some(Outcome::success().with_stdout("limactl version 2.2.1"))
            }
            ("limactl", ["list", "--json"]) => Some(
                Outcome::success().with_stdout(
                    self.vms
                        .borrow()
                        .iter()
                        .map(|vm| {
                            format!(
                                "{{\"name\":\"{}\",\"status\":\"{}\",\"cpus\":6,\"memory\":{},\"disk\":{},\"dir\":\"/x/{}\"}}\n",
                                vm.name,
                                vm.status,
                                vm.memory_gib * GIB,
                                100 * GIB,
                                vm.name
                            )
                        })
                        .collect::<String>(),
                ),
            ),
            ("limactl", ["rename", "--tty=false", from, to]) => {
                for vm in self.vms.borrow_mut().iter_mut().filter(|v| v.name == *from) {
                    vm.name = to.to_string();
                }
                None
            }
            ("limactl", ["stop", name]) => {
                self.set_status(name, "Stopped");
                None
            }
            ("limactl", ["start", _, name]) => {
                self.set_status(name, "Running");
                None
            }
            ("limactl", ["create", rest @ ..]) => {
                let created = rest
                    .iter()
                    .find_map(|a| a.strip_prefix("--name="))
                    .expect("a name");
                self.vms.borrow_mut().push(Vm {
                    name: created.to_string(),
                    status: "Stopped",
                    version: "24.04",
                    memory_gib: 12,
                });
                None
            }
            ("limactl", ["shell", _, _, guest, ..]) => {
                let script = spec.args.last().map(String::as_str).unwrap_or_default();
                let version = self
                    .vms
                    .borrow()
                    .iter()
                    .find(|v| v.name == *guest)
                    .map_or("24.04", |v| v.version);
                if script.contains("os-release") {
                    Some(Outcome::success().with_stdout(format!("{version}\n")))
                } else if script.ends_with("--version") {
                    Some(Outcome::success().with_stdout("rayx 1.0.0\n"))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn set_status(&self, name: &str, status: &'static str) {
        for vm in self.vms.borrow_mut().iter_mut().filter(|v| v.name == name) {
            vm.status = status;
        }
    }
}

fn guest(kind: &str, name: &str, ubuntu: &str, extra: &str) -> String {
    format!(
        r#"{{"kind":"{kind}","name":"{name}","ubuntu":"{ubuntu}","flags":["--web"],"clones":[],"vm":{{"cpus":6,"memory_gib":12,"disk_gib":100,"home_writable":false}},"retired":false{extra}}}"#
    )
}

fn retired(name: &str, ubuntu: &str, migrating: bool) -> String {
    format!(
        r#"{{"kind":"lima","name":"{name}","ubuntu":"{ubuntu}","flags":["--web"],"clones":[],"retired":true,"migrating":{migrating}}}"#
    )
}

fn record(guests: &[String]) -> String {
    format!(r#"{{"version":1,"guests":[{}]}}"#, guests.join(","))
}

fn machine(config: &Path, record: &str) -> FakeMachine {
    let dir = config.join("rayx");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("guests.json"), record).expect("record");
    FakeMachine::new()
        .with_home(HOME)
        .with_env("XDG_CONFIG_HOME", &config.display().to_string())
        .with_program("brew", "/opt/homebrew/bin/brew")
        .with_program("limactl", "/opt/homebrew/bin/limactl")
        .with_text(dir.join("guests.json"), record)
}

fn read_record(config: &Path) -> GuestsFile {
    read_guests_at(&config.join("rayx/guests.json")).expect("record")
}

fn request(vm: &str, yes: bool) -> LimaRequest {
    LimaRequest {
        flags: vec!["--web".into()],
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        release: None,
        name: name(vm),
        vm: VmFlags::default(),
        check: false,
        yes,
    }
}

fn setup_lima(
    request: &LimaRequest,
    machine: &FakeMachine,
    runner: &mut Runner,
    prompter: Option<&mut ScriptedPrompter>,
) -> (Result<LimaOutcome, LimaError>, String) {
    let mut out = Vec::new();
    let result = run_with(
        request,
        &macos(),
        runner,
        machine,
        None,
        prompter.map(|p| p as &mut dyn rayx_cli::setup::lima::sizing::Prompter),
        &mut out,
    );
    (result, String::from_utf8(out).expect("UTF-8"))
}

/// The `limactl` commands that change something: everything but the reads and the guest scripts.
fn limactl_lines(runner: &Runner) -> Vec<String> {
    runner
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
        .collect()
}

fn shell_specs(runner: &Runner) -> Vec<&CommandSpec> {
    runner
        .specs()
        .iter()
        .filter(|s| s.args.first().is_some_and(|a| a == "shell"))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The name
// ---------------------------------------------------------------------------------------------

#[test]
fn a_vm_name_is_one_to_forty_lowercase_letters_digits_and_hyphens_starting_with_a_letter() {
    let forty = format!("a{}", "b".repeat(39));
    for good in ["rayx", "app", "my-project", "a1", "x", forty.as_str()] {
        assert_eq!(VmName::parse(good).expect(good).as_str(), good);
    }
    let forty_one = format!("a{}", "b".repeat(40));
    for bad in [
        "",
        "default",
        "1app",
        "-app",
        "My-App",
        "my_app",
        "a.b",
        "app ",
        "ünï",
        forty_one.as_str(),
    ] {
        let error = VmName::parse(bad).expect_err(bad).to_string();
        assert!(error.contains("not a usable VM name"), "{bad}: {error}");
        assert!(error.contains("1 to 40"), "the rule: {error}");
        assert!(error.contains("`app`"), "an example: {error}");
    }
    assert_eq!(VmName::default().as_str(), "rayx");
    assert!(VmName::default().is_default() && !name("app").is_default());
    assert_eq!(name("app").retired("22.04"), "app-22.04");
    assert_eq!(
        VmName::of_retired("app-22.04", "22.04"),
        Some(name("app")),
        "the entry names its VM"
    );
    assert_eq!(VmName::of_retired("rayx-22.04", "20.04"), None);
}

#[test]
fn vm_is_accepted_with_lima_and_update_and_refused_with_wsl_or_alone() {
    let lima = parse(&["--lima", "--vm", "app"]).expect("--lima --vm");
    assert_eq!(lima.vm, ["app"]);
    assert_eq!(chosen_name(&lima), Ok(name("app")));

    let update = parse(&["--update", "--vm", "a", "--vm", "b"]).expect("--update --vm --vm");
    assert_eq!(update.vm, ["a", "b"], "repeatable");

    assert_eq!(
        chosen_name(&parse(&["--lima"]).expect("plain")),
        Ok(VmName::default())
    );
    for bad in [
        parse(&["--vm", "app"]),
        parse(&["--wsl", "--vm", "app"]),
        parse(&["--web", "--vm", "app"]),
    ] {
        assert_eq!(bad.expect_err("refused").exit_code(), 2);
    }
    let two = parse(&["--lima", "--vm", "a", "--vm", "b"]).expect("clap takes both");
    let error = chosen_name(&two).expect_err("one VM at a time");
    assert!(
        error.contains("one VM") && error.contains("a, b"),
        "{error}"
    );
    let bad = parse(&["--lima", "--vm", "Not Ok"]).expect("clap takes any text");
    assert!(
        chosen_name(&bad)
            .expect_err("bad name")
            .contains("not a usable VM name")
    );
}

#[test]
fn a_bad_vm_name_exits_1_with_the_rule_before_anything_runs() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rayx"))
        .args(["setup", "--lima", "--vm", "Not_Ok", "--check"])
        .output()
        .expect("runs rayx");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a usable VM name"), "{stderr}");
    assert!(stderr.contains("`app`"), "{stderr}");
    assert!(output.stdout.is_empty(), "nothing ran");
}

// ---------------------------------------------------------------------------------------------
// Creating, recording
// ---------------------------------------------------------------------------------------------

#[test]
fn a_named_vm_is_created_started_and_driven_under_its_own_name() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), &record(&[]));
    let mut runner = Lab::new(&[]).runner();

    let (result, out) = setup_lima(&request("app", true), &machine, &mut runner, None);

    result.expect("creates");
    assert_eq!(
        limactl_lines(&runner),
        [
            "create --tty=false --name=app --vm-type=vz --cpus=6 --memory=12 --disk=100 \
             --mount-only=/Users/dev --containerd=none template:ubuntu-24.04",
            "start --tty=false app"
        ]
    );
    assert!(
        out.contains("==> lima: creating the VM app (Ubuntu 24.04"),
        "{out}"
    );
    assert!(out.contains("==> lima: starting the VM app"), "{out}");
    let shells = shell_specs(&runner);
    assert!(!shells.is_empty());
    assert!(
        shells
            .iter()
            .all(|s| s.args[..5] == ["shell", "--workdir", "/", "app", "--"]),
        "every guest step runs in app: {shells:?}"
    );
}

#[test]
fn the_default_name_keeps_every_command_and_message_byte_for_byte() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), &record(&[]));
    let mut runner = Lab::new(&[]).runner();

    let (result, out) = setup_lima(&request("rayx", true), &machine, &mut runner, None);

    result.expect("creates");
    assert_eq!(
        limactl_lines(&runner),
        [
            "create --tty=false --name=rayx --vm-type=vz --cpus=6 --memory=12 --disk=100 \
             --mount-only=/Users/dev --containerd=none template:ubuntu-24.04",
            "start --tty=false rayx"
        ]
    );
    assert!(out.contains("==> lima: creating the VM rayx (Ubuntu 24.04, 6 CPUs, 12 GiB memory, 100 GiB disk, home read-only)"), "{out}");
    assert!(
        shell_specs(&runner)
            .iter()
            .all(|s| s.args[..5] == ["shell", "--workdir", "/", "rayx", "--"])
    );
}

#[test]
fn the_customize_question_names_the_vm() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), &record(&[]));
    let mut runner = Lab::new(&[]).runner();
    let mut prompter = ScriptedPrompter::always(false);

    let (result, _) = setup_lima(
        &request("app", false),
        &machine,
        &mut runner,
        Some(&mut prompter),
    );

    result.expect("creates with the defaults");
    assert_eq!(
        prompter.asked,
        [
            "Create Lima VM app: 6 CPUs, 12 GiB memory, 100 GiB disk, home mounted read-only. Customize?"
        ]
    );
}

#[test]
fn several_named_vms_coexist_each_with_its_own_record() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), &record(&[]));
    let mut runner = Lab::new(&[]).runner();
    let app = request("app", true);
    let mut api = request("api", true);
    api.flags = vec!["--test".into()];
    let small = VmRecord {
        cpus: 4,
        memory_gib: 8,
        disk_gib: 60,
        home_writable: true,
    };

    for (request, vm) in [(&app, None), (&api, Some(small))] {
        let outcome = LimaOutcome {
            vm: vm.or(Some(VmRecord {
                cpus: 6,
                memory_gib: 12,
                disk_gib: 100,
                home_writable: false,
            })),
            clone: None,
        };
        record_outcome(request, &macos(), &machine, &mut runner, &outcome);
    }

    let file = read_record(config.path());
    assert_eq!(file.guests.len(), 2);
    let app = file.find("lima", "app").expect("app");
    let api = file.find("lima", "api").expect("api");
    assert_eq!(app.flags, ["--web"]);
    assert_eq!(api.flags, ["--test"]);
    assert_eq!(api.vm, Some(small));
    assert_eq!(app.vm.expect("vm").cpus, 6);
}

// ---------------------------------------------------------------------------------------------
// Never adopt
// ---------------------------------------------------------------------------------------------

#[test]
fn a_vm_that_is_not_in_the_record_is_never_adopted_whatever_its_name() {
    for (vm, why) in [
        ("gpux", "gpux exists in Lima but was not created by rayx"),
        ("rayx", "rayx exists in Lima but was not created by rayx"),
    ] {
        let config = tempfile::tempdir().expect("temp dir");
        let machine = machine(
            config.path(),
            &record(&[guest("lima", "other", "24.04", "")]),
        );
        let mut runner =
            Lab::new(&[(vm, "Running", "24.04"), ("other", "Running", "24.04")]).runner();

        let (result, _) = setup_lima(&request(vm, true), &machine, &mut runner, None);

        let error = result.expect_err(vm).to_string();
        assert_eq!(
            error,
            format!("{why}: remove or rename it with limactl, or choose another --vm name")
        );
        assert!(
            limactl_lines(&runner).is_empty(),
            "{vm}: nothing started, stopped, edited, renamed or created: {:?}",
            limactl_lines(&runner)
        );
        assert!(
            shell_specs(&runner).is_empty(),
            "{vm}: nothing ran inside it"
        );
    }
}

#[test]
fn a_name_recorded_for_the_other_guest_kind_is_a_different_guest() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), &record(&[guest("wsl", "app", "24.04", "")]));
    let mut runner = Lab::new(&[("app", "Running", "24.04")]).runner();

    let (result, _) = setup_lima(&request("app", true), &machine, &mut runner, None);

    assert!(
        result
            .expect_err("not a Lima guest")
            .to_string()
            .contains("not created by rayx")
    );
    assert!(limactl_lines(&runner).is_empty());
}

#[test]
fn a_recorded_vm_that_is_missing_from_lima_is_created_again() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), &record(&[guest("lima", "app", "24.04", "")]));
    let mut runner = Lab::new(&[("rayx", "Running", "24.04")]).runner();

    let (result, _) = setup_lima(&request("app", true), &machine, &mut runner, None);

    result.expect("creates");
    assert!(limactl_lines(&runner)[0].contains("--name=app"));
}

#[test]
fn an_unreadable_record_is_not_taken_for_an_unrecorded_vm_nor_for_an_empty_record() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), "{ nope");
    let mut runner = Lab::new(&[("app", "Running", "24.04")]).runner();

    let (result, _) = setup_lima(&request("app", true), &machine, &mut runner, None);

    let error = result.expect_err("unreadable").to_string();
    assert!(
        error.contains("cannot tell whether it created it"),
        "{error}"
    );
    assert!(limactl_lines(&runner).is_empty());
}

#[test]
fn an_unreadable_record_stops_a_creation_before_any_vm_exists_and_check_still_reports() {
    for vm in ["app", "rayx"] {
        let config = tempfile::tempdir().expect("temp dir");
        let machine = machine(config.path(), "{ nope");
        let mut runner = Lab::new(&[("gpux", "Running", "24.04")]).runner();

        let (result, _) = setup_lima(&request(vm, true), &machine, &mut runner, None);

        let error = result.expect_err(vm).to_string();
        assert!(error.contains("no VM was created"), "{vm}: {error}");
        assert!(
            error.contains("guests.json"),
            "the record problem is named: {error}"
        );
        assert!(
            limactl_lines(&runner).is_empty(),
            "{vm}: nothing was created or started: {:?}",
            limactl_lines(&runner)
        );
        assert_eq!(
            std::fs::read_to_string(config.path().join("rayx/guests.json")).expect("file"),
            "{ nope",
            "the record is left as it is"
        );
    }

    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), "{ nope");
    let mut runner = Lab::new(&[]).runner();
    let mut check = request("app", true);
    check.check = true;
    let (result, _) = setup_lima(&check, &machine, &mut runner, None);
    result.expect("--check prints the plan");
}

// ---------------------------------------------------------------------------------------------
// Editing, sizing, memory
// ---------------------------------------------------------------------------------------------

#[test]
fn a_size_change_stops_edits_and_starts_that_vm_only() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(
        config.path(),
        &record(&[
            guest("lima", "app", "24.04", ""),
            guest("lima", "rayx", "24.04", ""),
        ]),
    );
    let lab = Lab::new(&[
        ("gpux", "Running", "24.04"),
        ("rayx", "Running", "24.04"),
        ("app", "Running", "24.04"),
    ]);
    let mut runner = lab.runner();
    let mut request = request("app", true);
    request.vm = VmFlags {
        cpus: Some(2),
        ..VmFlags::default()
    };

    let (result, out) = setup_lima(&request, &machine, &mut runner, None);

    result.expect("edits");
    assert_eq!(
        limactl_lines(&runner),
        [
            "stop app",
            "edit --tty=false app --cpus=2",
            "start --tty=false app"
        ],
        "other instances are never touched"
    );
    assert!(
        out.contains("==> lima: changing the VM app: 6 -> 2 CPUs"),
        "{out}"
    );
}

#[test]
fn a_declined_edit_hint_names_the_vm_and_the_default_keeps_its_text() {
    for (vm, flags) in [("app", "--lima --vm app"), ("rayx", "--lima")] {
        let config = tempfile::tempdir().expect("temp dir");
        let machine = machine(config.path(), &record(&[guest("lima", vm, "24.04", "")]));
        let mut runner = Lab::new(&[(vm, "Running", "24.04")]).runner();
        let mut request = request(vm, false);
        request.vm = VmFlags {
            cpus: Some(2),
            ..VmFlags::default()
        };
        let mut no = ScriptedPrompter::always(false);

        let (result, _) = setup_lima(&request, &machine, &mut runner, Some(&mut no));

        let error = result.expect_err("declined").to_string();
        assert_eq!(
            error,
            format!(
                "the VM {vm} was not changed (6 -> 2 CPUs): apply it with `rayx setup {flags} \
                 --yes` and the same --vm-* flags"
            )
        );
        assert_eq!(
            no.asked,
            [format!("Change the VM {vm} (6 -> 2 CPUs). Apply now?")]
        );
    }
}

#[test]
fn the_memory_warning_counts_every_other_running_vm_and_the_new_one() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), &record(&[]));
    let lab = Lab::with_memory(&[("gpux", "Running", "24.04", 30)]);
    let mut runner = lab.runner();

    let (result, out) = setup_lima(&request("app", true), &machine, &mut runner, None);

    result.expect("creates");
    assert!(
        out.contains("note: running VMs gpux (30 GiB) plus app (12 GiB)"),
        "{out}"
    );
}

// ---------------------------------------------------------------------------------------------
// Migration and its resume
// ---------------------------------------------------------------------------------------------

#[test]
fn a_named_vm_migrates_under_its_own_names_and_the_other_vm_is_left_alone() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(
        config.path(),
        &record(&[
            guest("lima", "app", "22.04", ""),
            guest("lima", "rayx", "22.04", ""),
        ]),
    );
    let lab = Lab::new(&[("app", "Running", "22.04"), ("rayx", "Running", "22.04")]);
    let mut runner = lab.runner();
    let mut yes = ScriptedPrompter::always(true);

    let (result, out) = setup_lima(
        &request("app", false),
        &machine,
        &mut runner,
        Some(&mut yes),
    );

    result.expect("migrates");
    assert_eq!(
        yes.asked,
        ["Create app with Ubuntu 24.04 next to app-22.04?"]
    );
    assert_eq!(
        limactl_lines(&runner),
        [
            "stop app",
            "rename --tty=false app app-22.04",
            "create --tty=false --name=app --vm-type=vz --cpus=6 --memory=12 --disk=100 \
             --mount-only=/Users/dev --containerd=none template:ubuntu-24.04",
            "start --tty=false app"
        ]
    );
    assert!(out.contains("rayx lima remove app-22.04"), "{out}");
    let file = read_record(config.path());
    let old = file.find("lima", "app-22.04").expect("retired entry");
    assert!(old.retired, "{old:?}");
    assert!(!file.find("lima", "app").expect("new").retired);
    let rayx = file.find("lima", "rayx").expect("untouched");
    assert!(!rayx.retired && rayx.ubuntu == "22.04", "{rayx:?}");
}

#[test]
fn a_declined_migration_names_the_vm_in_the_command_to_repeat() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(config.path(), &record(&[guest("lima", "app", "22.04", "")]));
    let mut runner = Lab::new(&[("app", "Running", "22.04")]).runner();
    let mut no = ScriptedPrompter::always(false);

    let (result, _) = setup_lima(&request("app", false), &machine, &mut runner, Some(&mut no));

    let error = result.expect_err("declined").to_string();
    assert!(
        error.contains("run `rayx setup --lima --vm app --yes`"),
        "{error}"
    );
    assert!(limactl_lines(&runner).is_empty());
}

#[test]
fn a_retired_vm_counts_as_an_interrupted_migration_of_its_own_name_only() {
    let file: GuestsFile = serde_json::from_str(&record(&[
        retired("app-22.04", "22.04", true),
        retired("rayx-22.04", "22.04", false),
        guest("lima", "rayx", "24.04", ""),
    ]))
    .expect("record");
    let instances = parse_instances(
        "{\"name\":\"app-22.04\",\"status\":\"Stopped\",\"dir\":\"/x\"}\n\
         {\"name\":\"rayx-22.04\",\"status\":\"Stopped\",\"dir\":\"/y\"}\n",
    );

    let app = pending_lima(&file, &instances, &name("app")).expect("app is mid-migration");
    assert_eq!(app.current_name, "app-22.04");
    assert!(pending_lima(&file, &instances, &VmName::default()).is_none());
    assert!(
        pending_lima(&file, &instances, &name("rayx-app")).is_none(),
        "a name that merely starts alike does not match"
    );
    assert!(pending_lima(&file, &instances, &name("rayx")).is_none());
}

#[test]
fn an_interrupted_named_migration_resumes_without_renaming_again() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(
        config.path(),
        &record(&[
            retired("app-22.04", "22.04", true),
            guest("lima", "rayx", "24.04", ""),
        ]),
    );
    let lab = Lab::new(&[
        ("app-22.04", "Stopped", "22.04"),
        ("rayx", "Running", "24.04"),
    ]);
    let mut runner = lab.runner();

    let (result, out) = setup_lima(&request("app", true), &machine, &mut runner, None);

    result.expect("resumes");
    assert_eq!(
        limactl_lines(&runner),
        [
            "create --tty=false --name=app --vm-type=vz --cpus=6 --memory=12 --disk=100 \
             --mount-only=/Users/dev --containerd=none template:ubuntu-24.04",
            "start --tty=false app"
        ]
    );
    assert!(out.contains("app now runs Ubuntu 24.04"), "{out}");
    let file = read_record(config.path());
    assert!(file.find("lima", "app").is_some_and(|g| !g.retired));
    assert!(
        file.find("lima", "app-22.04")
            .is_some_and(|g| g.retired && !g.migrating),
        "the migration is finished"
    );
}

#[test]
fn another_names_retired_vm_is_not_resumed_by_a_run_for_a_new_name() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(
        config.path(),
        &record(&[retired("app-22.04", "22.04", true)]),
    );
    let lab = Lab::new(&[("app-22.04", "Stopped", "22.04")]);
    let mut runner = lab.runner();

    let (result, _) = setup_lima(&request("api", true), &machine, &mut runner, None);

    result.expect("creates api");
    assert!(
        limactl_lines(&runner).iter().all(|l| !l.contains("app")),
        "{:?}",
        limactl_lines(&runner)
    );
}

#[test]
fn the_refresh_reports_an_interrupted_migration_under_the_vms_own_name() {
    let machine = FakeMachine::new().with_home(HOME).with_text(
        format!("{HOME}/.config/rayx/guests.json"),
        &record(&[
            retired("app-22.04", "22.04", true),
            retired("rayx-22.04", "22.04", true),
        ]),
    );
    let options = RefreshOptions {
        version: "1.0.0".into(),
        release: None,
        targets: Vec::new(),
        yes: true,
        check: false,
        lima_binary: None,
        wsl_binary: None,
    };
    let mut runner = Lab::new(&[]).runner();

    let report = refresh_guests(
        &options,
        &macos(),
        &mut runner,
        &machine,
        None,
        &mut Vec::new(),
    );

    let why = |name: &str| match report.guests.iter().find(|(n, _)| n == name) {
        Some((_, GuestOutcome::Failed(why))) => why.clone(),
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(
        why("rayx"),
        "a migration was interrupted: rayx-22.04 is renamed and retired, and rayx is not set up \
         yet; run `rayx setup --lima` to finish it"
    );
    assert_eq!(
        why("app"),
        "a migration was interrupted: app-22.04 is renamed and retired, and app is not set up \
         yet; run `rayx setup --lima --vm app` to finish it"
    );
}

// ---------------------------------------------------------------------------------------------
// Removal and doctor
// ---------------------------------------------------------------------------------------------

#[test]
fn removing_a_named_vm_deletes_only_it_and_says_how_to_create_it_again() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = machine(
        config.path(),
        &record(&[
            guest("lima", "app", "24.04", ""),
            guest("lima", "rayx", "24.04", ""),
        ]),
    );
    let mut runner =
        Lab::new(&[("app", "Running", "24.04"), ("rayx", "Running", "24.04")]).runner();
    let mut out = Vec::new();

    remove_guest(
        GuestKind::Lima,
        "app",
        true,
        &macos(),
        &mut runner,
        &machine,
        None,
        &mut out,
    )
    .expect("removes");

    assert_eq!(limactl_lines(&runner), ["delete --force app"]);
    let out = String::from_utf8(out).expect("UTF-8");
    assert!(
        out.contains("`rayx setup --lima --vm app` creates it again"),
        "{out}"
    );
    let file = read_record(config.path());
    assert!(file.find("lima", "app").is_none() && file.find("lima", "rayx").is_some());

    let mut runner = Lab::new(&[("rayx", "Running", "24.04")]).runner();
    let mut out = Vec::new();
    remove_guest(
        GuestKind::Lima,
        "rayx",
        true,
        &macos(),
        &mut runner,
        &machine,
        None,
        &mut out,
    )
    .expect("removes the default");
    let out = String::from_utf8(out).expect("UTF-8");
    assert!(
        out.contains("`rayx setup --lima` creates it again"),
        "{out}"
    );
}

#[test]
fn doctor_reports_a_named_vm_and_the_command_that_removes_its_retired_one() {
    let machine = FakeMachine::new().with_home(HOME).with_text(
        format!("{HOME}/.config/rayx/guests.json"),
        &record(&[
            guest("lima", "app", "24.04", ""),
            retired("app-22.04", "22.04", false),
        ]),
    );
    let lab = Lab::new(&[
        ("app", "Running", "24.04"),
        ("app-22.04", "Stopped", "22.04"),
    ]);
    let mut runner = lab.runner();
    let env = PlanEnv {
        host: macos(),
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
    };
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));

    let report = report_for(&env, &mut runner, &machine, &mut path, None);

    let text = report.to_text();
    assert!(text.contains("lima app: running"), "{text}");
    assert!(text.contains("rayx lima remove app-22.04"), "{text}");
    let json = report.to_json();
    let guests = json["guests"].as_array().expect("array");
    assert!(
        guests
            .iter()
            .any(|g| g["name"] == "app" && g["state"] == "current")
    );
}
