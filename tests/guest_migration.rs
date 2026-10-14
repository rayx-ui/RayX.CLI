//! Side-by-side Ubuntu migration: an older guest is never upgraded in place; a new one is made
//! from the pinned image next to it, its clones and flags are restored from the record, and the
//! old guest is kept, retired, until the owner removes it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rayx_cli::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::lima::sizing::ScriptedPrompter;
use rayx_cli::setup::lima::{LimaError, LimaRequest, VmFlags, run_with};
use rayx_cli::setup::migrate::{older_than_pin, outdated_wsl, pending_lima};
use rayx_cli::setup::records::{GuestsFile, read_guests_at};
use rayx_cli::setup::refresh::{GuestOutcome, RefreshOptions, refresh_guests};
use rayx_cli::setup::wsl::{WslRequest, distribution, migrate_older_guest};
use rayx_cli::wsl::Distribution;

const GIB: u64 = 1024 * 1024 * 1024;
const HOME: &str = "/Users/dev";
const OLD_RECORD: &str = r#"{"version":1,"guests":[{"kind":"lima","name":"rayx","ubuntu":"22.04","flags":["--web"],
  "clones":[{"source":"/Users/dev/RayX","branch":"main","target":"$HOME/RayX"}],
  "vm":{"cpus":6,"memory_gib":12,"disk_gib":100,"home_writable":false},"retired":false}]}"#;

#[derive(Clone)]
struct Vm {
    name: String,
    status: &'static str,
    version: &'static str,
}

/// A scripted Lima: instances that `rename`, `create`, `start` and `stop` change.
#[derive(Clone)]
struct Lab {
    vms: Rc<RefCell<Vec<Vm>>>,
    fail_create: Rc<RefCell<bool>>,
}

impl Lab {
    fn new(vms: &[(&str, &'static str, &'static str)]) -> Self {
        Self {
            vms: Rc::new(RefCell::new(
                vms.iter()
                    .map(|(name, status, version)| Vm {
                        name: name.to_string(),
                        status,
                        version,
                    })
                    .collect(),
            )),
            fail_create: Rc::new(RefCell::new(false)),
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
            ("limactl", ["--version"]) => Some(Outcome::success().with_stdout("limactl version 2.2.1")),
            ("limactl", ["list", "--json"]) => Some(Outcome::success().with_stdout(
                self.vms
                    .borrow()
                    .iter()
                    .map(|vm| {
                        format!(
                            "{{\"name\":\"{}\",\"status\":\"{}\",\"cpus\":6,\"memory\":{},\"disk\":{},\"dir\":\"/x/{}\"}}\n",
                            vm.name,
                            vm.status,
                            12 * GIB,
                            100 * GIB,
                            vm.name
                        )
                    })
                    .collect::<String>(),
            )),
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
            ("limactl", ["create", ..]) => {
                if std::mem::take(&mut *self.fail_create.borrow_mut()) {
                    return Some(Outcome::failure(1));
                }
                self.vms.borrow_mut().push(Vm {
                    name: "rayx".into(),
                    status: "Stopped",
                    version: "24.04",
                });
                None
            }
            ("limactl", ["shell", _, _, name, ..]) => {
                let script = spec.args.last().map(String::as_str).unwrap_or_default();
                let version = self
                    .vms
                    .borrow()
                    .iter()
                    .find(|v| v.name == *name)
                    .map_or("24.04", |v| v.version);
                if script.contains("os-release") {
                    Some(Outcome::success().with_stdout(format!("{version}\n")))
                } else if script.ends_with("--version") {
                    Some(Outcome::success().with_stdout("rayx 1.0.0\n"))
                } else if script.contains("DIRTY") {
                    Some(Outcome::success().with_stdout("DIRTY\t/home/dev.guest/RayX\n"))
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

fn host(os: Os) -> HostFacts {
    HostFacts {
        os,
        arch: Arch::Arm64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn write_record(config: &Path, json: &str) {
    let dir = config.join("rayx");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(dir.join("guests.json"), json).expect("record");
}

fn read_record(config: &Path) -> GuestsFile {
    read_guests_at(&config.join("rayx/guests.json")).expect("record")
}

fn mac_machine(config: &Path, record: &str) -> FakeMachine {
    write_record(config, record);
    FakeMachine::new()
        .with_home(HOME)
        .with_env("XDG_CONFIG_HOME", &config.display().to_string())
        .with_program("brew", "/opt/homebrew/bin/brew")
        .with_program("limactl", "/opt/homebrew/bin/limactl")
        .with_dir("/Users/dev/RayX")
        .with_text(config.join("rayx/guests.json"), record)
        .with_text(
            "/x/rayx/lima.yaml",
            "mounts:\n- location: \"~\"\n  writable: true\n",
        )
        .with_text(
            "/x/rayx-22.04/lima.yaml",
            "mounts:\n- location: \"~\"\n  writable: true\n",
        )
}

fn request(yes: bool, check: bool) -> LimaRequest {
    LimaRequest {
        flags: vec!["--web".into()],
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        vm: VmFlags::default(),
        check,
        yes,
    }
}

fn setup_lima(
    request: &LimaRequest,
    machine: &FakeMachine,
    runner: &mut Runner,
    prompter: Option<&mut ScriptedPrompter>,
) -> (Result<(), LimaError>, String) {
    let mut out = Vec::new();
    let result = run_with(
        request,
        &host(Os::MacOs),
        runner,
        machine,
        None,
        prompter.map(|p| p as &mut dyn rayx_cli::setup::lima::sizing::Prompter),
        &mut out,
    )
    .map(|_| ());
    (result, String::from_utf8(out).expect("UTF-8"))
}

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

fn guest_scripts(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|s| s.args.first().is_some_and(|a| a == "shell"))
        .filter_map(|s| s.args.last().cloned())
        .collect()
}

#[test]
fn versions_are_compared_numerically_and_the_pin_itself_is_current() {
    assert!(older_than_pin("22.04") && older_than_pin("20.04") && older_than_pin("9.10"));
    assert!(!older_than_pin("24.04") && !older_than_pin("24.10") && !older_than_pin("26.04"));
}

#[test]
fn declined_or_unanswerable_changes_nothing_lists_the_dirty_clones_and_reports_outdated() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);

    let lab = Lab::new(&[("rayx", "Running", "22.04")]);
    let mut runner = lab.runner();
    let mut no = ScriptedPrompter::always(false);
    let (result, out) = setup_lima(&request(false, false), &machine, &mut runner, Some(&mut no));

    let error = result.expect_err("declined");
    assert!(matches!(error, LimaError::Declined(_)), "{error}");
    assert!(
        error.to_string().contains("older than 24.04")
            && error.to_string().contains("nothing was changed")
    );
    assert_eq!(
        no.asked,
        ["Create rayx with Ubuntu 24.04 next to rayx-22.04?"]
    );
    assert!(
        out.contains("/home/dev.guest/RayX"),
        "the dirty clone is listed: {out}"
    );
    assert!(out.contains("never upgraded in place"), "{out}");
    assert!(
        limactl_lines(&runner).is_empty(),
        "{:?}",
        limactl_lines(&runner)
    );
    assert_eq!(
        read_record(config.path()).guests[0].name,
        "rayx",
        "the record is unchanged"
    );

    let mut silent = ScriptedPrompter::always(true);
    silent.terminal = false;
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();
    let (result, _) = setup_lima(
        &request(false, false),
        &machine,
        &mut runner,
        Some(&mut silent),
    );
    assert!(matches!(result, Err(LimaError::Declined(_))));
    assert!(limactl_lines(&runner).is_empty());
}

#[test]
fn the_dirty_clone_query_looks_for_changes_and_commits_on_no_remote() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();

    let mut no = ScriptedPrompter::always(false);
    setup_lima(&request(false, false), &machine, &mut runner, Some(&mut no))
        .0
        .expect_err("declined");

    let query = guest_scripts(&runner)
        .into_iter()
        .find(|s| s.contains("DIRTY"))
        .expect("the dirty clone query ran");
    assert!(query.contains("status --porcelain"), "{query}");
    assert!(query.contains("log --branches --not --remotes"), "{query}");
}

#[test]
fn the_lima_migration_runs_stop_rename_create_start_and_restores_the_clones() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let lab = Lab::new(&[("gpux", "Stopped", "24.04"), ("rayx", "Running", "22.04")]);
    let mut runner = lab.runner();
    let mut yes = ScriptedPrompter::always(true);

    let (result, out) = setup_lima(
        &request(false, false),
        &machine,
        &mut runner,
        Some(&mut yes),
    );

    result.expect("migrates");
    let changes = limactl_lines(&runner);
    assert_eq!(
        changes,
        [
            "stop rayx",
            "rename --tty=false rayx rayx-22.04",
            "create --tty=false --name=rayx --vm-type=vz --cpus=6 --memory=12 --disk=100 \
             --mount-only=/Users/dev:w --containerd=none template:ubuntu-24.04",
            "start --tty=false rayx",
        ],
        "the old VM's CPUs, memory, disk and home mount; no customize question"
    );
    assert_eq!(yes.asked.len(), 1, "one question: {:?}", yes.asked);
    let scripts = guest_scripts(&runner);
    let clone = scripts
        .iter()
        .find(|s| s.contains(" clone "))
        .expect("clone");
    assert!(
        clone.contains("clone -b 'main' '/Users/dev/RayX' \"$HOME/RayX\""),
        "re-cloned from the host source: {clone}"
    );
    assert!(
        scripts
            .iter()
            .any(|s| s == "cd \"$HOME/RayX\" && \"$HOME/.rayx/bin/rayx\" setup --web"),
        "setup per clone with the recorded flags: {scripts:?}"
    );
    assert!(out.contains("rayx lima remove rayx-22.04"), "{out}");
    for command in runner.lines() {
        assert!(
            !command.contains("do-release-upgrade")
                && !command.contains("delete")
                && !command.contains("remove rayx"),
            "{command}"
        );
        assert!(
            !command.contains("gpux"),
            "another instance is never touched: {command}"
        );
    }

    let record = read_record(config.path());
    let old = record.find("lima", "rayx-22.04").expect("the old entry");
    assert!(old.retired);
    assert_eq!(old.ubuntu, "22.04");
    let new = record.find("lima", "rayx").expect("the new entry");
    assert!(!new.retired);
    assert_eq!(new.ubuntu, "24.04");
    assert_eq!(new.flags, ["--web"]);
    assert_eq!(new.clones.len(), 1);
    assert_eq!(new.vm.expect("settings").cpus, 6);
}

#[test]
fn the_record_is_updated_right_after_the_rename_and_a_failed_create_resumes() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let lab = Lab::new(&[("rayx", "Running", "22.04")]);
    *lab.fail_create.borrow_mut() = true;
    let mut runner = lab.runner();

    let (result, _) = setup_lima(&request(true, false), &machine, &mut runner, None);

    result.expect_err("the create failed");
    let record = read_record(config.path());
    assert_eq!(record.guests.len(), 1, "{:?}", record.guests);
    assert_eq!(record.guests[0].name, "rayx-22.04");
    assert!(
        record.guests[0].retired,
        "renamed and retired before the new VM exists"
    );
    assert_eq!(
        record.guests[0].clones.len(),
        1,
        "the clones survive for the resume"
    );

    // The rerun finds the retired VM and no current `rayx`: it resumes without renaming again.
    let resume_machine = mac_machine(
        config.path(),
        &std::fs::read_to_string(config.path().join("rayx/guests.json")).expect("record"),
    );
    let mut runner = lab.runner();
    let (result, out) = setup_lima(&request(true, false), &resume_machine, &mut runner, None);

    result.expect("resumes");
    let changes = limactl_lines(&runner);
    assert!(
        changes
            .iter()
            .all(|c| !c.starts_with("rename") && !c.starts_with("stop")),
        "{changes:?}"
    );
    assert!(
        changes[0].starts_with("create --tty=false --name=rayx") && changes[0].contains("--cpus=6"),
        "{changes:?}"
    );
    assert_eq!(changes[1], "start --tty=false rayx");
    assert!(out.contains("rayx lima remove rayx-22.04"), "{out}");
    let record = read_record(config.path());
    assert!(
        record
            .find("lima", "rayx")
            .is_some_and(|g| !g.retired && g.ubuntu == "24.04")
    );
    assert!(record.find("lima", "rayx-22.04").is_some_and(|g| g.retired));
}

#[test]
fn a_new_vm_that_an_interrupted_run_already_made_is_reused() {
    let config = tempfile::tempdir().expect("temp dir");
    let retired = r#"{"version":1,"guests":[{"kind":"lima","name":"rayx-22.04","ubuntu":"22.04","flags":["--web"],"clones":[],"retired":true,"migrating":true}]}"#;
    let machine = mac_machine(config.path(), retired);
    let lab = Lab::new(&[
        ("rayx-22.04", "Stopped", "22.04"),
        ("rayx", "Stopped", "24.04"),
    ]);
    let mut runner = lab.runner();

    let (result, _) = setup_lima(&request(true, false), &machine, &mut runner, None);

    result.expect("resumes");
    let changes = limactl_lines(&runner);
    assert_eq!(
        changes,
        ["start --tty=false rayx"],
        "no second create, no rename"
    );
    assert!(
        read_record(config.path())
            .find("lima", "rayx")
            .is_some_and(|g| !g.retired)
    );
}

#[test]
fn yes_replaces_without_asking_and_check_only_prints_the_plan() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);

    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();
    let mut prompter = ScriptedPrompter::always(false);
    let (result, _) = setup_lima(
        &request(true, false),
        &machine,
        &mut runner,
        Some(&mut prompter),
    );
    result.expect("--yes");
    assert!(prompter.asked.is_empty());
    assert!(
        limactl_lines(&runner)
            .iter()
            .any(|c| c.starts_with("rename --tty=false rayx rayx-22.04"))
    );

    let machine = mac_machine(config.path(), OLD_RECORD);
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();
    let (result, out) = setup_lima(
        &request(false, true),
        &machine,
        &mut runner,
        Some(&mut ScriptedPrompter::always(true)),
    );
    result.expect("--check");
    assert!(
        out.contains("would run: limactl stop rayx; limactl rename rayx rayx-22.04"),
        "{out}"
    );
    assert!(
        limactl_lines(&runner)
            .iter()
            .all(|c| c.starts_with("start") || c.starts_with("--")),
        "--check renames and creates nothing: {:?}",
        limactl_lines(&runner)
    );
}

#[test]
fn a_vm_at_the_pin_is_not_migrated() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), &OLD_RECORD.replace("22.04", "24.04"));
    let mut runner = Lab::new(&[("rayx", "Running", "24.04")]).runner();

    let (result, _) = setup_lima(&request(true, false), &machine, &mut runner, None);

    result.expect("normal run");
    assert!(limactl_lines(&runner).is_empty());
}

#[test]
fn the_update_refresh_migrates_an_outdated_guest_instead_of_skipping_it() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let lab = Lab::new(&[("rayx", "Running", "22.04")]);
    let mut runner = lab.runner();
    let options = RefreshOptions {
        version: "1.0.0".into(),
        yes: true,
        check: false,
        lima_binary: None,
        wsl_binary: None,
    };

    let report = refresh_guests(
        &options,
        &host(Os::MacOs),
        &mut runner,
        &machine,
        None,
        &mut Vec::new(),
    );

    assert_eq!(
        report.guests,
        [(
            "rayx".to_string(),
            GuestOutcome::Migrated("rayx".to_string())
        )]
    );
    assert_eq!(report.code(), 0);
    assert!(
        limactl_lines(&runner)
            .iter()
            .any(|c| c.starts_with("rename --tty=false rayx rayx-22.04"))
    );
}

fn windows_machine(config: &Path, record: &str, registered: &[&str]) -> FakeMachine {
    write_record(config, record);
    let distributions = registered
        .iter()
        .map(|name| Distribution {
            id: format!("{{{name}}}"),
            name: (*name).into(),
            base_path: PathBuf::from(format!(r"C:\wsl\{name}")),
            version: Some(2),
            default_uid: Some(1000),
        })
        .collect();
    FakeMachine::new()
        .with_env("LOCALAPPDATA", &config.display().to_string())
        .with_wsl_distributions(distributions)
        .with_dir(r"D:\Work\RayX")
        .with_text(config.join("rayx/guests.json"), record)
}

const WSL_RECORD: &str = r#"{"version":1,"guests":[{"kind":"wsl","name":"Ubuntu-22.04","ubuntu":"22.04","flags":["--web"],
  "clones":[{"source":"D:\\Work\\RayX","branch":"main","target":"$HOME/RayX"}],"retired":false}]}"#;

fn wsl_runner() -> Runner {
    wsl_runner_with("22.04")
}

/// A Windows where the old distribution reports `old_version` and every other one 24.04.
fn wsl_runner_with(old_version: &'static str) -> Runner {
    Runner::record()
        .with_os(Os::Windows)
        .responder(move |spec| {
            let script = spec.args.last().map(String::as_str).unwrap_or_default();
            let distribution = spec.args.get(1).map(String::as_str).unwrap_or_default();
            if script.contains("DIRTY") {
                Some(Outcome::success().with_stdout("DIRTY\t/home/dev/RayX\n"))
            } else if script.contains("os-release") {
                let version = if distribution == "Ubuntu-22.04" {
                    old_version
                } else {
                    "24.04"
                };
                Some(Outcome::success().with_stdout(format!("{version}\n")))
            } else if script.ends_with("--version") {
                Some(Outcome::success().with_stdout("rayx 1.0.0\n"))
            } else {
                None
            }
        })
}

fn wsl_request() -> WslRequest {
    WslRequest {
        flags: vec!["--web".into()],
        clone: None,
        binary_override: None,
        version: "1.0.0".into(),
        windows_gpu: None,
    }
}

fn migrate_wsl(
    machine: &FakeMachine,
    runner: &mut Runner,
    yes: bool,
    prompter: Option<&mut ScriptedPrompter>,
) -> (Result<(), String>, String) {
    let mut out = Vec::new();
    let result = migrate_older_guest(
        &wsl_request(),
        false,
        yes,
        &host(Os::Windows),
        runner,
        machine,
        prompter.map(|p| p as &mut dyn rayx_cli::setup::lima::sizing::Prompter),
        &mut out,
    );
    (result, String::from_utf8(out).expect("UTF-8"))
}

#[test]
fn wsl_installs_the_pinned_distribution_next_to_the_old_one_and_stops_for_the_first_user() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = windows_machine(config.path(), WSL_RECORD, &["Ubuntu-22.04"]);
    let mut runner = wsl_runner();
    let mut yes = ScriptedPrompter::always(true);

    let (result, out) = migrate_wsl(&machine, &mut runner, false, Some(&mut yes));

    assert_eq!(
        yes.asked,
        ["Create Ubuntu-24.04 with Ubuntu 24.04 next to Ubuntu-22.04?"]
    );
    assert!(out.contains("/home/dev/RayX"), "dirty clones listed: {out}");
    let install = runner
        .lines()
        .iter()
        .find(|l| l.contains("--install"))
        .expect("install");
    assert!(
        install.contains("wsl --install -d Ubuntu-24.04"),
        "{install}"
    );
    // The fake registry is static: the new distribution has no user yet, as after a real install.
    let error = result.expect_err("first user");
    assert!(error.contains("Ubuntu-24.04 has no user yet"), "{error}");
    assert!(
        runner.lines().iter().all(|l| !l.contains("unregister")),
        "the old one is never removed"
    );
    assert!(
        !read_record(config.path()).guests[0].retired,
        "retired only once the new one is set up"
    );
}

#[test]
fn wsl_continues_on_a_rerun_restores_the_clones_and_retires_the_old_entry() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = windows_machine(config.path(), WSL_RECORD, &["Ubuntu-22.04", "Ubuntu-24.04"]);
    let mut runner = wsl_runner();
    let mut prompter = ScriptedPrompter::always(false);

    let (result, out) = migrate_wsl(&machine, &mut runner, false, Some(&mut prompter));

    result.expect("continues");
    assert!(
        prompter.asked.is_empty(),
        "the new distribution exists: this is a resume"
    );
    assert!(
        runner.lines().iter().all(|l| !l.contains("--install")),
        "{:?}",
        runner.lines()
    );
    let scripts: Vec<String> = runner
        .specs()
        .iter()
        .filter(|s| s.program == "wsl" && s.args.contains(&"bash".to_string()))
        .filter_map(|s| s.args.last().cloned())
        .collect();
    let clone = scripts
        .iter()
        .find(|s| s.contains(" clone "))
        .expect("clone");
    assert!(
        clone.contains("clone -b 'main' '/mnt/d/Work/RayX' \"$HOME/RayX\""),
        "{clone}"
    );
    assert!(
        scripts.contains(&"cd \"$HOME/RayX\" && \"$HOME/.rayx/bin/rayx\" setup --web".to_string()),
        "{scripts:?}"
    );
    assert!(out.contains("rayx wsl remove Ubuntu-22.04"), "{out}");
    let record = read_record(config.path());
    assert!(
        record
            .find("wsl", "Ubuntu-22.04")
            .is_some_and(|g| g.retired)
    );
    let new = record.find("wsl", &distribution()).expect("new entry");
    assert!(!new.retired);
    assert_eq!(new.flags, ["--web"]);
    assert_eq!(new.clones[0].target, "$HOME/RayX");
}

#[test]
fn wsl_declined_changes_nothing_and_a_current_distribution_starts_no_migration() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = windows_machine(config.path(), WSL_RECORD, &["Ubuntu-22.04"]);
    let mut runner = wsl_runner();
    let mut no = ScriptedPrompter::always(false);

    let (result, _) = migrate_wsl(&machine, &mut runner, false, Some(&mut no));

    let error = result.expect_err("declined");
    assert!(error.contains("nothing was changed"), "{error}");
    assert!(runner.lines().iter().all(|l| !l.contains("--install")));

    let current = WSL_RECORD
        .replace("Ubuntu-22.04", "Ubuntu-24.04")
        .replace("22.04", "24.04");
    let machine = windows_machine(config.path(), &current, &["Ubuntu-24.04"]);
    let mut runner = wsl_runner();
    let (result, _) = migrate_wsl(&machine, &mut runner, false, None);
    result.expect("nothing to migrate");
    assert!(runner.specs().is_empty());
    assert!(outdated_wsl(&read_record(config.path())).is_none());
}

#[test]
fn the_kept_line_names_the_old_guest_and_its_removal_command_before_the_question() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();
    let mut no = ScriptedPrompter::always(false);

    let (_, out) = setup_lima(&request(false, false), &machine, &mut runner, Some(&mut no));

    assert!(
        out.contains("The old VM is kept, retired, as rayx-22.04 until you remove it: rayx lima remove rayx-22.04"),
        "{out}"
    );
}

#[test]
fn check_over_an_interrupted_migration_prints_the_plan_and_records_nothing() {
    let config = tempfile::tempdir().expect("temp dir");
    let retired = r#"{"version":1,"guests":[{"kind":"lima","name":"rayx-22.04","ubuntu":"22.04","flags":["--web"],"clones":[],"retired":true,"migrating":true}]}"#;
    let machine = mac_machine(config.path(), retired);
    let mut runner = Lab::new(&[("rayx-22.04", "Stopped", "22.04")]).runner();

    let (result, out) = setup_lima(&request(false, true), &machine, &mut runner, None);

    result.expect("--check");
    assert!(out.contains("A migration of rayx was interrupted"), "{out}");
    assert!(!out.contains("now runs Ubuntu"), "{out}");
    assert!(
        limactl_lines(&runner)
            .iter()
            .all(|c| !c.starts_with("create") && !c.starts_with("rename")),
        "{:?}",
        limactl_lines(&runner)
    );
    let record = read_record(config.path());
    assert_eq!(
        record.guests.len(),
        1,
        "no current rayx is recorded: {:?}",
        record.guests
    );
    assert!(record.guests[0].migrating);
}

#[test]
fn an_unusable_record_stops_the_migration_before_anything_is_stopped_or_renamed() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    std::fs::write(config.path().join("rayx/guests.json"), "not json").expect("break it");
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();

    let (result, _) = setup_lima(&request(true, false), &machine, &mut runner, None);

    let error = result.expect_err("unusable record").to_string();
    assert!(error.contains("rayx was not changed"), "{error}");
    assert!(
        limactl_lines(&runner).is_empty(),
        "{:?}",
        limactl_lines(&runner)
    );
    assert_eq!(
        std::fs::read_to_string(config.path().join("rayx/guests.json")).expect("file"),
        "not json"
    );
}

#[cfg(unix)]
#[test]
fn a_record_that_cannot_be_written_stops_the_migration_before_the_rename() {
    use std::os::unix::fs::PermissionsExt;

    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let dir = config.path().join("rayx");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).expect("read-only");
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();

    let (result, _) = setup_lima(&request(true, false), &machine, &mut runner, None);

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("restore");
    // Running as root can write anyway; then the migration simply proceeds.
    if result.is_err() {
        assert!(
            limactl_lines(&runner).is_empty(),
            "{:?}",
            limactl_lines(&runner)
        );
    }
}

#[test]
fn a_finished_migration_is_not_a_pending_one_and_a_plain_removal_is_not_a_resume() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let lab = Lab::new(&[("rayx", "Running", "22.04")]);
    let mut runner = lab.runner();
    let (result, _) = setup_lima(&request(true, false), &machine, &mut runner, None);
    result.expect("migrates");

    let record = read_record(config.path());
    assert!(
        !record.find("lima", "rayx-22.04").expect("old").migrating,
        "the migration is finished"
    );

    // The owner removes the current guest: the retired VM stays, but nothing resumes.
    let mut after_removal = record.clone();
    after_removal.remove("lima", "rayx");
    let instances = rayx_cli::setup::lima::parse_instances(
        "{\"name\":\"rayx-22.04\",\"status\":\"Stopped\",\"cpus\":6,\"memory\":12884901888,\"disk\":107374182400,\"dir\":\"/x\"}\n",
    );
    assert!(pending_lima(&after_removal, &instances).is_none());
}

#[test]
fn the_update_refresh_reports_an_interrupted_migration_instead_of_exiting_0() {
    let config = tempfile::tempdir().expect("temp dir");
    let retired = r#"{"version":1,"guests":[{"kind":"lima","name":"rayx-22.04","ubuntu":"22.04","flags":[],"clones":[],"retired":true,"migrating":true}]}"#;
    let machine = mac_machine(config.path(), retired);
    let mut runner = Lab::new(&[("rayx-22.04", "Stopped", "22.04")]).runner();
    let options = RefreshOptions {
        version: "1.0.0".into(),
        yes: true,
        check: false,
        lima_binary: None,
        wsl_binary: None,
    };

    let report = refresh_guests(
        &options,
        &host(Os::MacOs),
        &mut runner,
        &machine,
        None,
        &mut Vec::new(),
    );

    assert!(
        matches!(&report.guests[0].1, GuestOutcome::Failed(why) if why.contains("interrupted") && why.contains("rayx setup --lima")),
        "{:?}",
        report.guests
    );
    assert_eq!(report.code(), 1);
    assert!(limactl_lines(&runner).is_empty());
}

#[test]
fn vm_flags_on_an_older_vm_do_not_skip_the_version_check() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();
    let mut asked = request(true, false);
    asked.vm = VmFlags {
        cpus: Some(8),
        ..VmFlags::default()
    };
    let mut no = ScriptedPrompter::always(false).with_confirms([true, false]);
    asked.yes = false;

    let (result, _) = setup_lima(&asked, &machine, &mut runner, Some(&mut no));

    let error = result.expect_err("replacement declined");
    assert!(matches!(error, LimaError::Declined(_)), "{error}");
    let changes = limactl_lines(&runner);
    assert!(
        changes
            .iter()
            .any(|c| c.starts_with("edit --tty=false rayx --cpus=8")),
        "{changes:?}"
    );
    assert!(
        changes.iter().all(|c| !c.starts_with("rename")),
        "{changes:?}"
    );
}

#[test]
fn a_vm_newer_than_the_pin_is_reported_by_setup_lima_and_left_alone() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), &OLD_RECORD.replace("22.04", "26.04"));
    let mut runner = Lab::new(&[("rayx", "Running", "26.04")]).runner();

    let (result, out) = setup_lima(&request(true, false), &machine, &mut runner, None);

    result.expect("normal run");
    assert!(out.contains("newer than the pinned 24.04"), "{out}");
    assert!(limactl_lines(&runner).is_empty());
}

#[test]
fn wsl_trusts_the_guests_own_version_over_a_stale_record_and_reports_a_newer_one() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = windows_machine(config.path(), WSL_RECORD, &["Ubuntu-22.04"]);
    // The distribution says it is already at the pin although the record is old.
    let mut runner = wsl_runner_with("24.04");
    let (result, _) = migrate_wsl(&machine, &mut runner, true, None);
    result.expect("nothing to migrate");
    assert!(
        runner.lines().iter().all(|l| !l.contains("--install")),
        "{:?}",
        runner.lines()
    );

    let newer = WSL_RECORD.replace("\"ubuntu\":\"22.04\"", "\"ubuntu\":\"26.04\"");
    let machine = windows_machine(config.path(), &newer, &["Ubuntu-22.04"]);
    let mut runner = wsl_runner_with("26.04");
    let (result, out) = migrate_wsl(&machine, &mut runner, true, None);
    result.expect("nothing to migrate");
    assert!(out.contains("newer than the pinned 24.04"), "{out}");
    assert!(runner.specs().is_empty());
}
