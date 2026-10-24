//! The Ubuntu pin and the guest layer WSL and Lima share: the command that runs a script inside a
//! guest, the Linux `rayx` install, the checkout clone and the inner `rayx setup`.

use std::path::{Path, PathBuf};

use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::pins::{DEFAULTS, PinSource, Pins};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::guest::{
    CLONES_FILE, CloneSource, Guest, LEGACY_CLONES_FILE, LINUX_BINARY, LinuxBinary, checkout_name,
    clone_checkout, clone_script, dirty_clones, ensure_linux_binary, lima_template, linux_dir,
    os_version, pinned_ubuntu, release_asset, run_setup_inside, wsl_distribution,
};
use rayx_cli::setup::wsl::{Checkout, WslRequest, distribution, run_with};
use rayx_cli::wsl::{Distribution, status_script};

const INSTALLER: &str =
    "https://github.com/rayx-ui/RayX.CLI/releases/download/v1.0.0/rayx-cli-installer.sh";

fn binary() -> LinuxBinary {
    LinuxBinary {
        version: "1.0.0".into(),
        arch: Arch::X64,
        override_path: None,
    }
}

fn guests() -> [Guest; 2] {
    [Guest::wsl("Ubuntu-24.04"), Guest::lima("rayx")]
}

/// The scripts each guest command received, in order (the last argument of `bash -lc <script>`).
fn scripts(runner: &Runner) -> Vec<String> {
    runner
        .specs()
        .iter()
        .filter(|spec| spec.args.iter().any(|a| a == "bash"))
        .filter_map(|spec| spec.args.last().cloned())
        .collect()
}

#[test]
fn the_ubuntu_pin_is_a_built_in_default_a_project_cannot_override() {
    let pins = Pins::resolve(None);
    assert_eq!(pins.ubuntu.value, "24.04");
    assert_eq!(pins.ubuntu.source, PinSource::Default);
    assert_eq!(DEFAULTS.ubuntu, "24.04");
    assert_eq!(pinned_ubuntu(), "24.04");

    // `[workspace.metadata.rayx] ubuntu = "22.04"` is not a key: a guest serves every project.
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = []\n[workspace.metadata.rayx]\nubuntu = \"22.04\"\n",
    )
    .expect("manifest");
    let project =
        rayx_cli::project::Project::discover(&mut Runner::record(), dir.path(), None).ok();
    let resolved = Pins::resolve(project.as_ref());
    assert_eq!(resolved.ubuntu.value, "24.04");
    assert_eq!(resolved.ubuntu.source, PinSource::Default);
}

#[test]
fn both_guest_names_derive_from_one_pin_value() {
    assert_eq!(wsl_distribution(pinned_ubuntu()), "Ubuntu-24.04");
    assert_eq!(lima_template(pinned_ubuntu()), "template:ubuntu-24.04");
    assert_eq!(distribution(), "Ubuntu-24.04");

    let mut pins = Pins::resolve(None);
    pins.ubuntu.value = "26.04".into();
    assert_eq!(wsl_distribution(&pins.ubuntu.value), "Ubuntu-26.04");
    assert_eq!(lima_template(&pins.ubuntu.value), "template:ubuntu-26.04");
}

#[test]
fn no_source_file_but_the_pins_table_names_an_ubuntu_version() {
    fn walk(dir: &Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, hits);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).expect("source");
                for (number, line) in text.lines().enumerate() {
                    let code = line.trim_start();
                    if !code.starts_with("//")
                        && (code.contains("\"24.04\"")
                            || code.contains("buntu-24")
                            || code.contains("template:ubuntu\"")
                            || code.contains("ubuntu-lts"))
                    {
                        hits.push(format!("{}:{}: {line}", path.display(), number + 1));
                    }
                }
            }
        }
    }
    let mut hits = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut hits,
    );
    let hits: Vec<&String> = hits
        .iter()
        // Windows prints `project\pins.rs`.
        .filter(|hit| !hit.replace('\\', "/").contains("project/pins.rs"))
        .collect();
    assert!(
        hits.is_empty(),
        "one literal Ubuntu version, in the pins table: {hits:?}"
    );
}

#[test]
fn a_script_runs_inside_each_guest_kind_as_one_bash_argument() {
    let script = "echo \"it's $HOME\"; ls | wc -l";

    let wsl = Guest::wsl("Ubuntu-24.04").script(script);
    assert_eq!(wsl.program, "wsl");
    assert_eq!(
        wsl.args,
        ["-d", "Ubuntu-24.04", "--exec", "bash", "-lc", script]
    );

    let lima = Guest::lima("rayx").script(script);
    assert_eq!(lima.program, "limactl");
    assert_eq!(
        lima.args,
        [
            "shell",
            "--workdir",
            "/",
            "rayx",
            "--",
            "bash",
            "-lc",
            script
        ]
    );
}

#[test]
fn the_release_installer_installs_the_matching_linux_rayx_in_each_guest() {
    for guest in guests() {
        let mut runner = Runner::record().with_os(Os::Windows);
        let mut out = Vec::new();

        ensure_linux_binary(&guest, &binary(), &mut runner, &mut out).expect("installs");

        let all = scripts(&runner);
        assert_eq!(all.len(), 2, "a version probe, then the installer: {all:?}");
        assert_eq!(all[0], format!("\"{LINUX_BINARY}\" --version"));
        assert!(
            all[1].starts_with("set -o pipefail; curl --proto '=https' --tlsv1.2 -LsSf"),
            "{}",
            all[1]
        );
        assert!(all[1].contains(&format!("'{INSTALLER}' | env -u RAYX_HOME sh")));
        assert!(all[1].ends_with(&format!("test -x \"{LINUX_BINARY}\"")));
        let heading = String::from_utf8(out).expect("UTF-8");
        assert!(
            heading.contains(&format!(
                "==> {}: installing the Linux rayx 1.0.0",
                guest.kind.label()
            )),
            "{heading}"
        );
    }
}

#[test]
fn a_matching_linux_rayx_is_left_alone_and_an_old_one_is_replaced() {
    for guest in guests() {
        let current = Runner::record().respond(
            |spec| spec.args.last().is_some_and(|a| a.ends_with("--version")),
            Outcome::success().with_stdout("rayx 1.0.0\n"),
        );
        let mut runner = current;
        ensure_linux_binary(&guest, &binary(), &mut runner, &mut Vec::new()).expect("current");
        assert_eq!(scripts(&runner).len(), 1, "only the probe ran");

        let mut old = Runner::record().respond(
            |spec| spec.args.last().is_some_and(|a| a.ends_with("--version")),
            Outcome::success().with_stdout("rayx 0.9.0\n"),
        );
        ensure_linux_binary(&guest, &binary(), &mut old, &mut Vec::new()).expect("replaces");
        assert_eq!(scripts(&old).len(), 2, "the installer ran");
    }
}

#[test]
fn an_override_binary_is_copied_instead_of_downloading() {
    for guest in guests() {
        let mut runner = Runner::record();
        let mut request = binary();
        request.override_path = Some("/Users/dev/rayx-linux/rayx".into());

        ensure_linux_binary(&guest, &request, &mut runner, &mut Vec::new()).expect("copies");

        let all = scripts(&runner);
        assert_eq!(all.len(), 1, "{all:?}");
        assert!(
            all[0].contains("cp \"/Users/dev/rayx-linux/rayx\" \"$HOME/.rayx/bin/rayx\""),
            "{}",
            all[0]
        );
        assert!(!all[0].contains("curl"), "{}", all[0]);
    }
}

#[test]
fn an_architecture_without_a_linux_release_fails_before_any_download() {
    assert_eq!(release_asset(Arch::X64), Some("x86_64-unknown-linux-musl"));
    assert_eq!(release_asset(Arch::Other), None);
    for guest in guests() {
        let mut runner = Runner::record();
        let mut request = binary();
        request.arch = Arch::Other;

        let error = ensure_linux_binary(&guest, &request, &mut runner, &mut Vec::new())
            .expect_err("no release");

        assert!(error.contains("no Linux rayx release"), "{error}");
        assert!(
            scripts(&runner).iter().all(|s| !s.contains("curl")),
            "nothing downloaded"
        );
    }
}

#[test]
fn check_mode_prints_the_install_and_changes_nothing() {
    let shared = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut runner = Runner::print_to(Sink(shared.clone()));
    let guest = Guest::lima("rayx-guest-layer-test-none");
    let source = CloneSource {
        name: "app".into(),
        path: "/Users/dev/app".into(),
        branch: Some("main".into()),
    };

    ensure_linux_binary(&guest, &binary(), &mut runner, &mut Vec::new()).expect("prints");
    clone_checkout(&guest, "", &source, &mut runner, &mut Vec::new()).expect("prints");
    run_setup_inside(
        &guest,
        &["--web".to_string()],
        Some("$HOME/app"),
        &mut runner,
        &mut Vec::new(),
    )
    .expect("prints");

    let printed = String::from_utf8(shared.lock().expect("lock").clone()).expect("UTF-8");
    assert_eq!(printed.lines().count(), 3, "{printed}");
    assert!(printed.lines().all(|l| l.starts_with("limactl shell")));
    assert!(
        runner.specs().is_empty(),
        "print mode records and runs nothing"
    );
}

#[test]
fn a_checkout_is_cloned_and_recorded_in_the_guest_neutral_file() {
    for guest in guests() {
        let mut runner = Runner::record();
        let source = CloneSource {
            name: "RayX".into(),
            path: "/mnt/d/Work/RayX".into(),
            branch: Some("feat/x".into()),
        };

        let target =
            clone_checkout(&guest, "", &source, &mut runner, &mut Vec::new()).expect("clones");

        assert_eq!(target, "$HOME/RayX");
        let script = &scripts(&runner)[0];
        assert!(
            script.contains(
                "git -c safe.directory='*' clone -b 'feat/x' '/mnt/d/Work/RayX' \"$HOME/RayX\""
            ),
            "{script}"
        );
        assert!(
            script.contains("git -C \"$HOME/RayX\" fetch --all --prune"),
            "an existing clone is fetched: {script}"
        );
        assert!(
            script.contains(&format!("echo \"$HOME/RayX\" >> \"{CLONES_FILE}\"")),
            "{script}"
        );
        assert_eq!(CLONES_FILE, "$HOME/.config/rayx/clones");

        let explicit = clone_checkout(
            &guest,
            "~/work/app",
            &source,
            &mut Runner::record(),
            &mut Vec::new(),
        )
        .expect("explicit target");
        assert_eq!(explicit, "$HOME/work/app");
    }
}

#[test]
fn unsafe_clone_directories_and_quoted_sources_are_refused() {
    for bad in ["relative/dir", "~/a\"b", "/srv/$x", "/srv/`id`", "/srv/a'b"] {
        assert!(linux_dir(bad).is_err(), "{bad}");
    }
    assert_eq!(linux_dir("~").as_deref(), Ok("$HOME"));
    assert!(clone_script("/Users/o'brien/x", None, "$HOME/x").is_err());
    assert!(clone_script("/Users/dev/x", Some("a'b"), "$HOME/x").is_err());
    for guest in guests() {
        let mut runner = Runner::record();
        let source = CloneSource {
            name: "x".into(),
            path: "/Users/dev/x".into(),
            branch: None,
        };
        let error = clone_checkout(&guest, "/srv/a\"b", &source, &mut runner, &mut Vec::new())
            .expect_err("unsafe");
        assert!(error.contains("not a usable Linux directory"), "{error}");
        assert!(runner.specs().is_empty(), "nothing ran");
    }
}

#[test]
fn checkout_names_split_on_both_separators() {
    assert_eq!(
        checkout_name(Path::new(r"D:\Work\RayX\")).as_deref(),
        Some("RayX")
    );
    assert_eq!(
        checkout_name(Path::new("/Users/dev/app")).as_deref(),
        Some("app")
    );
    assert_eq!(checkout_name(&PathBuf::from("/")), None);
}

#[test]
fn setup_runs_inside_the_guest_in_the_clone_with_the_same_flags() {
    for guest in guests() {
        let flags = vec!["--web".to_string(), "--yes".to_string()];
        let mut runner = Runner::record();
        run_setup_inside(
            &guest,
            &flags,
            Some("$HOME/RayX"),
            &mut runner,
            &mut Vec::new(),
        )
        .expect("runs");
        run_setup_inside(&guest, &[], None, &mut runner, &mut Vec::new()).expect("runs");

        let all = scripts(&runner);
        assert_eq!(
            all[0],
            format!("cd \"$HOME/RayX\" && \"{LINUX_BINARY}\" setup --web --yes")
        );
        assert_eq!(
            all[1],
            format!("\"{LINUX_BINARY}\" setup"),
            "no trailing space"
        );
        assert!(
            runner.specs().iter().all(|spec| spec.interactive),
            "stdio is inherited"
        );
    }
}

#[test]
fn the_guest_version_and_dirty_clones_are_read_from_the_guest() {
    for guest in guests() {
        let mut runner = Runner::record().respond(
            |spec| spec.args.last().is_some_and(|a| a.contains("os-release")),
            Outcome::success().with_stdout("24.04\n"),
        );
        assert_eq!(os_version(&guest, &mut runner).as_deref(), Some("24.04"));
        assert_eq!(scripts(&runner)[0], ". /etc/os-release; echo $VERSION_ID");

        let mut dirty = Runner::record().respond(
            |spec| spec.args.last().is_some_and(|a| a.contains("DIRTY")),
            Outcome::success().with_stdout("DIRTY\t/home/dev/RayX\nnoise\nDIRTY\trelative\n"),
        );
        assert_eq!(dirty_clones(&guest, &mut dirty), ["/home/dev/RayX"]);
        let script = &scripts(&dirty)[0];
        assert!(script.contains(CLONES_FILE) && script.contains(LEGACY_CLONES_FILE));
    }
}

#[test]
fn status_and_compaction_read_the_neutral_and_the_legacy_clones_file() {
    let script = status_script();
    assert!(script.contains(CLONES_FILE), "{script}");
    assert!(script.contains("$HOME/.config/rayx/wsl-clones"), "{script}");
    assert_eq!(LEGACY_CLONES_FILE, "$HOME/.config/rayx/wsl-clones");
}

fn windows() -> HostFacts {
    HostFacts {
        os: Os::Windows,
        arch: Arch::X64,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

#[test]
fn setup_wsl_runs_the_same_sequence_on_the_guest_layer() {
    let machine = FakeMachine::new().with_wsl_distributions(vec![Distribution {
        id: "{abc}".into(),
        name: distribution(),
        base_path: PathBuf::from(r"C:\Users\dev\AppData\Local\wsl\ubuntu"),
        version: Some(2),
        default_uid: Some(1000),
    }]);
    let mut runner = Runner::record().with_os(Os::Windows).respond(
        |spec| spec.program == "wsl" && spec.args == ["--version"],
        Outcome::success().with_stdout("WSL version: 2.6.1"),
    );
    let request = WslRequest {
        flags: vec!["--web".to_string()],
        clone: Some(String::new()),
        binary_override: None,
        version: "1.0.0".into(),
        release: None,
    };
    let checkout = Checkout {
        root: PathBuf::from(r"D:\Work\RayX"),
        branch: Some("main".into()),
    };

    run_with(
        &request,
        &windows(),
        &mut runner,
        &machine,
        Some(&checkout),
        &mut Vec::new(),
    )
    .expect("sets up");

    let wsl: Vec<&rayx_cli::host::CommandSpec> = runner
        .specs()
        .iter()
        .filter(|spec| spec.args.iter().any(|a| a == "bash"))
        .collect();
    assert_eq!(wsl.len(), 4, "probe, installer, clone, setup");
    for spec in &wsl {
        assert_eq!(spec.program, "wsl");
        assert_eq!(
            spec.args[..5],
            ["-d", "Ubuntu-24.04", "--exec", "bash", "-lc"]
        );
    }
    let last = wsl[3].args.last().expect("script");
    assert_eq!(
        last,
        &format!("cd \"$HOME/RayX\" && \"{LINUX_BINARY}\" setup --web")
    );
    assert!(
        wsl[2]
            .args
            .last()
            .expect("script")
            .contains("'/mnt/d/Work/RayX'")
    );
}

#[test]
fn doctor_lists_the_ubuntu_pin_with_the_other_pins() {
    use rayx_cli::host::path_env::{MemoryPathStore, UserPath};
    use rayx_cli::setup::PlanEnv;

    let env = PlanEnv {
        host: windows(),
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes: false,
    };
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(None)));
    let report = rayx_cli::doctor::report_for(
        &env,
        &mut Runner::record().with_os(Os::Windows),
        &FakeMachine::new(),
        &mut path,
        None,
    );

    let json = report.to_json();
    assert_eq!(json["pins"]["ubuntu"]["value"], "24.04");
    assert_eq!(json["pins"]["ubuntu"]["source"], "default");
}
