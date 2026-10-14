//! Corrective flows found by the advisory read of the integrated change: `--vm-*` flags with a
//! migration, guests recorded even when the first run fails, and the refresh after a migration.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rayx_cli::host::{Arch, CommandSpec, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::guest::GuestKind;
use rayx_cli::setup::lima::sizing::ScriptedPrompter;
use rayx_cli::setup::lima::{
    LimaError, LimaRequest, VmFlags, record_failed_first_run as lima_failed, run_with,
};
use rayx_cli::setup::records::{GuestsFile, read_guests_at};
use rayx_cli::setup::refresh::{GuestOutcome, RefreshOptions, refresh_guests};
use rayx_cli::setup::remove::remove_guest;
use rayx_cli::setup::wsl::record_failed_first_run as wsl_failed;
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

fn flags_request(vm: VmFlags) -> LimaRequest {
    let mut asked = request(true, false);
    asked.vm = vm;
    asked
}

#[test]
fn flags_with_a_migration_size_the_new_vm_and_the_old_one_is_never_edited() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();
    let vm = VmFlags {
        cpus: Some(5),
        memory_gib: Some(8),
        disk_gib: Some(50),
        home_writable: false,
    };

    let (result, _) = setup_lima(&flags_request(vm), &machine, &mut runner, None);

    result.expect("migrates");
    let changes = limactl_lines(&runner);
    assert!(
        changes.iter().all(|c| !c.starts_with("edit")),
        "no edit of the old VM: {changes:?}"
    );
    assert_eq!(changes[0], "stop rayx");
    assert!(changes[1].starts_with("rename --tty=false rayx rayx-22.04"));
    assert_eq!(
        changes[2],
        "create --tty=false --name=rayx --vm-type=vz --cpus=5 --memory=8 --disk=50 \
         --mount-only=/Users/dev:w --containerd=none template:ubuntu-24.04",
        "the flags laid over the old VM's settings (the home stays as it was)"
    );
    let record = read_record(config.path());
    let new = record.find("lima", "rayx").expect("new entry");
    let vm = new.vm.expect("settings");
    assert_eq!(
        (vm.cpus, vm.memory_gib, vm.disk_gib),
        (5, 8, 50),
        "the record holds what was used"
    );
}

#[test]
fn flags_that_are_not_given_keep_the_old_vms_size_and_a_writable_home_flag_is_added() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD)
        .with_text("/x/rayx/lima.yaml", "mounts:\n- location: \"/Users/dev\"\n");
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();
    let vm = VmFlags {
        home_writable: true,
        ..VmFlags::default()
    };

    let (result, _) = setup_lima(&flags_request(vm), &machine, &mut runner, None);

    result.expect("migrates");
    let create = limactl_lines(&runner)
        .into_iter()
        .find(|c| c.starts_with("create"))
        .expect("create");
    assert!(
        create.contains("--cpus=6 --memory=12 --disk=100 --mount-only=/Users/dev:w"),
        "{create}"
    );
}

#[test]
fn a_stopped_older_vm_with_flags_is_started_to_be_inspected_not_edited() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let mut runner = Lab::new(&[("rayx", "Stopped", "22.04")]).runner();
    let vm = VmFlags {
        cpus: Some(4),
        ..VmFlags::default()
    };

    let (result, _) = setup_lima(&flags_request(vm), &machine, &mut runner, None);

    result.expect("migrates");
    let changes = limactl_lines(&runner);
    assert_eq!(changes[0], "start --tty=false rayx", "{changes:?}");
    assert!(
        changes.iter().all(|c| !c.starts_with("edit")),
        "{changes:?}"
    );
    assert!(
        changes.iter().any(|c| c.contains("--cpus=4")),
        "{changes:?}"
    );
}

#[test]
fn a_resumed_migration_applies_the_flags_to_the_vm_it_creates() {
    let config = tempfile::tempdir().expect("temp dir");
    let retired = r#"{"version":1,"guests":[{"kind":"lima","name":"rayx-22.04","ubuntu":"22.04","flags":["--web"],"clones":[],"retired":true,"migrating":true}]}"#;
    let machine = mac_machine(config.path(), retired);
    let mut runner = Lab::new(&[("rayx-22.04", "Stopped", "22.04")]).runner();
    let vm = VmFlags {
        memory_gib: Some(6),
        ..VmFlags::default()
    };

    let (result, _) = setup_lima(&flags_request(vm), &machine, &mut runner, None);

    result.expect("resumes");
    let create = limactl_lines(&runner)
        .into_iter()
        .find(|c| c.starts_with("create"))
        .expect("create");
    assert!(
        create.contains("--cpus=6 --memory=6 --disk=100"),
        "{create}"
    );
}

fn mac_host() -> HostFacts {
    host(Os::MacOs)
}

#[test]
fn a_first_run_that_made_the_vm_and_then_failed_leaves_it_recorded_and_removable() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), "{\"version\":1,\"guests\":[]}");
    // The VM exists now (the failed run created it); nothing was recorded.
    let lab = Lab::new(&[("rayx", "Running", "24.04")]);

    lima_failed(
        &request(false, false),
        &mac_host(),
        &machine,
        &mut lab.runner(),
        false,
    );

    let record = read_record(config.path());
    let guest = record
        .find("lima", "rayx")
        .expect("recorded after the failure");
    assert_eq!(guest.ubuntu, "24.04");
    assert!(!guest.retired);
    let vm = guest.vm.expect("settings");
    assert_eq!((vm.cpus, vm.memory_gib, vm.disk_gib), (6, 12, 100));

    // ... so `rayx lima remove rayx` knows it.
    let mut runner = lab.runner();
    let mut out = Vec::new();
    let recorded = std::fs::read_to_string(config.path().join("rayx/guests.json")).expect("record");
    let machine = mac_machine(config.path(), &recorded);
    remove_guest(
        GuestKind::Lima,
        "rayx",
        true,
        &mac_host(),
        &mut runner,
        &machine,
        None,
        &mut out,
    )
    .expect("removable");
}

#[test]
fn nothing_is_recorded_when_the_vm_existed_before_is_absent_or_the_run_was_a_check() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), "{\"version\":1,\"guests\":[]}");
    let lab = Lab::new(&[("rayx", "Running", "24.04")]);

    lima_failed(
        &request(false, false),
        &mac_host(),
        &machine,
        &mut lab.runner(),
        true,
    );
    lima_failed(
        &request(false, true),
        &mac_host(),
        &machine,
        &mut lab.runner(),
        false,
    );
    lima_failed(
        &request(false, false),
        &mac_host(),
        &machine,
        &mut Lab::new(&[("gpux", "Running", "24.04")]).runner(),
        false,
    );

    assert!(read_record(config.path()).guests.is_empty());
}

fn windows_with(config: &Path, registered: &[&str]) -> FakeMachine {
    let dir = config.join("rayx");
    std::fs::create_dir_all(&dir).expect("dir");
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
}

#[test]
fn a_distribution_a_failed_first_run_registered_is_recorded() {
    let config = tempfile::tempdir().expect("temp dir");
    let windows = host(Os::Windows);

    wsl_failed(
        false,
        &windows,
        &windows_with(config.path(), &["Ubuntu-24.04"]),
        false,
    );

    let record = read_record(config.path());
    let guest = record.find("wsl", "Ubuntu-24.04").expect("recorded");
    assert_eq!(guest.ubuntu, "24.04");
    assert!(guest.vm.is_none());

    let other = tempfile::tempdir().expect("temp dir");
    wsl_failed(
        false,
        &windows,
        &windows_with(other.path(), &["Ubuntu-24.04"]),
        true,
    );
    wsl_failed(
        true,
        &windows,
        &windows_with(other.path(), &["Ubuntu-24.04"]),
        false,
    );
    wsl_failed(
        false,
        &windows,
        &windows_with(other.path(), &["Debian"]),
        false,
    );
    assert!(
        !other.path().join("rayx/guests.json").exists(),
        "existing, check and unregistered record nothing"
    );
}

#[test]
fn after_a_migration_the_refresh_stops_the_vm_it_started_and_says_why() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), OLD_RECORD);
    let lab = Lab::new(&[("rayx", "Stopped", "22.04")]);
    let mut runner = lab.runner();
    let options = RefreshOptions {
        version: "1.0.0".into(),
        yes: true,
        check: false,
        lima_binary: None,
        wsl_binary: None,
    };
    let mut out = Vec::new();

    let report = refresh_guests(
        &options,
        &host(Os::MacOs),
        &mut runner,
        &machine,
        None,
        &mut out,
    );

    assert_eq!(
        report.guests,
        [(
            "rayx".to_string(),
            GuestOutcome::Migrated("rayx".to_string())
        )]
    );
    let changes = limactl_lines(&runner);
    assert_eq!(
        changes.first().map(String::as_str),
        Some("start --tty=false rayx"),
        "{changes:?}"
    );
    assert_eq!(
        changes.last().map(String::as_str),
        Some("stop rayx"),
        "the started VM is stopped again: {changes:?}"
    );
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .contains("stopping the VM rayx again (it was stopped before the refresh)")
    );

    let mut running = Lab::new(&[("rayx", "Running", "22.04")]).runner();
    refresh_guests(
        &options,
        &host(Os::MacOs),
        &mut running,
        &machine,
        None,
        &mut Vec::new(),
    );
    assert_ne!(
        limactl_lines(&running).last().map(String::as_str),
        Some("stop rayx"),
        "a VM that was running is left running"
    );
}

#[test]
fn a_running_old_vm_with_no_record_is_replaced_not_edited() {
    let config = tempfile::tempdir().expect("temp dir");
    // Nothing is recorded: the VM's own Ubuntu decides.
    let machine = mac_machine(config.path(), "{\"version\":1,\"guests\":[]}");
    let mut runner = Lab::new(&[("rayx", "Running", "22.04")]).runner();
    let vm = VmFlags {
        cpus: Some(4),
        ..VmFlags::default()
    };

    let (result, _) = setup_lima(&flags_request(vm), &machine, &mut runner, None);

    result.expect("migrates");
    let changes = limactl_lines(&runner);
    assert!(
        changes.iter().all(|c| !c.starts_with("edit")),
        "{changes:?}"
    );
    assert!(
        changes
            .iter()
            .any(|c| c.starts_with("create") && c.contains("--cpus=4")),
        "{changes:?}"
    );
}

#[test]
fn a_refused_flag_on_a_stopped_current_vm_is_refused_before_anything_starts() {
    let config = tempfile::tempdir().expect("temp dir");
    let machine = mac_machine(config.path(), &OLD_RECORD.replace("22.04", "24.04"));
    let mut runner = Lab::new(&[("rayx", "Stopped", "24.04")]).runner();
    let vm = VmFlags {
        disk_gib: Some(50),
        ..VmFlags::default()
    };

    let (result, _) = setup_lima(&flags_request(vm), &machine, &mut runner, None);

    let error = result.expect_err("a disk cannot shrink").to_string();
    assert!(error.contains("Lima cannot shrink a disk"), "{error}");
    assert!(
        limactl_lines(&runner).is_empty(),
        "nothing started: {:?}",
        limactl_lines(&runner)
    );
}
