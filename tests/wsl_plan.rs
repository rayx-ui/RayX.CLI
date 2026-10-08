//! `rayx setup --wsl [--clone]`: the plan from Windows, tested with a fake runner, fake registry
//! and fixture checkouts.

use std::path::{Path, PathBuf};

use clap::Parser;
use rayx_cli::cli::{Cli, Command, SetupArgs};
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::setup::FakeMachine;
use rayx_cli::setup::wsl::{
    Checkout, DISTRIBUTION, WslError, WslRequest, clone_script, linux_dir, release_asset, run_with,
    setup_flags, to_wsl_path,
};
use rayx_cli::wsl::Distribution;

fn host(os: Os, arch: Arch) -> HostFacts {
    HostFacts {
        os,
        arch,
        emulated: false,
        wsl: false,
        distro: None,
    }
}

fn windows() -> HostFacts {
    host(Os::Windows, Arch::X64)
}

fn ubuntu(uid: u32) -> Distribution {
    Distribution {
        id: "{abc}".into(),
        name: DISTRIBUTION.into(),
        base_path: PathBuf::from(r"C:\Users\dev\AppData\Local\wsl\ubuntu"),
        version: Some(2),
        default_uid: Some(uid),
    }
}

fn request(flags: &[&str]) -> WslRequest {
    WslRequest {
        flags: flags.iter().map(ToString::to_string).collect(),
        clone: None,
        binary_override: None,
        version: "0.3.0".into(),
    }
}

/// A runner that answers `wsl --version` like WSL 2 (UTF-16 output arrives with NUL bytes).
fn wsl2_runner() -> Runner {
    Runner::record().with_os(Os::Windows).respond(
        |spec| spec.program == "wsl" && spec.args == ["--version"],
        Outcome::success().with_stdout(
            "WSL version: 2.6.1"
                .chars()
                .flat_map(|c| [c, '\0'])
                .collect::<String>(),
        ),
    )
}

fn lines(runner: &Runner) -> Vec<String> {
    runner.lines().to_vec()
}

fn run(
    request: &WslRequest,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &FakeMachine,
    checkout: Option<&Checkout>,
) -> Result<String, WslError> {
    let mut out = Vec::new();
    run_with(request, host, runner, machine, checkout, &mut out)
        .map(|()| String::from_utf8(out).expect("UTF-8"))
}

#[test]
fn other_hosts_are_told_where_wsl_setup_runs() {
    for os in [Os::Linux, Os::MacOs] {
        let mut runner = Runner::record();
        let error = run(
            &request(&[]),
            &host(os, Arch::X64),
            &mut runner,
            &FakeMachine::new(),
            None,
        )
        .expect_err("not Windows");
        assert_eq!(error, WslError::NotWindows);
        assert!(error.to_string().contains("from Windows"));
        assert!(runner.lines().is_empty(), "nothing runs off Windows");
    }
}

#[test]
fn wsl_without_version_support_is_not_wsl_2() {
    let mut runner = Runner::record()
        .with_os(Os::Windows)
        .respond(|spec| spec.args == ["--version"], Outcome::failure(1));

    let error = run(
        &request(&[]),
        &windows(),
        &mut runner,
        &FakeMachine::new(),
        None,
    )
    .expect_err("needs WSL 2");

    assert!(matches!(error, WslError::Wsl(_)), "{error}");
    assert!(error.to_string().contains("wsl --install"));
    assert_eq!(lines(&runner), ["wsl --version"], "nothing else ran");
}

#[test]
fn a_missing_distribution_is_installed_and_then_needs_its_first_user() {
    let mut runner = wsl2_runner();

    let error = run(
        &request(&[]),
        &windows(),
        &mut runner,
        &FakeMachine::new(),
        None,
    )
    .expect_err("the distribution has no user yet");

    assert!(
        lines(&runner)
            .iter()
            .any(|line| line == "wsl --install -d Ubuntu-24.04"),
        "{:?}",
        lines(&runner)
    );
    assert!(matches!(error, WslError::NeedsUser(_)));
    let text = error.to_string();
    assert!(
        text.contains("open Ubuntu once") && text.contains("rayx setup --wsl"),
        "{text}"
    );
    assert!(
        !lines(&runner).iter().any(|line| line.contains("bash")),
        "nothing runs inside before the user exists"
    );
}

#[test]
fn a_distribution_whose_default_user_is_root_needs_its_first_user() {
    let machine = FakeMachine::new().with_wsl_distributions(vec![ubuntu(0)]);
    let mut runner = wsl2_runner();

    let error = run(&request(&[]), &windows(), &mut runner, &machine, None).expect_err("root");

    assert!(matches!(error, WslError::NeedsUser(_)));
    assert!(
        !lines(&runner).iter().any(|line| line.contains("--install")),
        "an existing distribution is not installed again"
    );
}

#[test]
fn the_release_asset_of_the_same_version_is_installed_and_setup_runs_with_the_same_flags() {
    let machine = FakeMachine::new().with_wsl_distributions(vec![ubuntu(1000)]);
    let mut runner = wsl2_runner();

    run(
        &request(&["--web", "--test", "--yes"]),
        &windows(),
        &mut runner,
        &machine,
        None,
    )
    .expect("sets up");

    let all = lines(&runner);
    let download = all
        .iter()
        .find(|line| line.contains("curl"))
        .unwrap_or_else(|| panic!("a download in {all:?}"));
    assert!(
        download.contains(
            "https://github.com/rayx-ui/RayX.CLI/releases/download/v0.3.0/rayx-cli-x86_64-unknown-linux-musl.tar.xz"
        ),
        "{download}"
    );
    assert!(download.contains("-d Ubuntu-24.04"), "{download}");
    let setup = all.last().expect("setup runs last");
    assert!(
        setup.contains("$HOME/.local/bin/rayx") && setup.contains("setup --web --test --yes"),
        "{setup}"
    );
    let setup_spec = runner.specs().last().expect("a spec");
    assert!(
        setup_spec.interactive,
        "setup runs in this console so sudo prompts reach the user"
    );
}

#[test]
fn windows_arm64_installs_the_arm64_linux_binary() {
    assert_eq!(release_asset(Arch::X64), Some("x86_64-unknown-linux-musl"));
    assert_eq!(
        release_asset(Arch::Arm64),
        Some("aarch64-unknown-linux-musl")
    );
    assert_eq!(release_asset(Arch::Other), None);

    let machine = FakeMachine::new().with_wsl_distributions(vec![ubuntu(1000)]);
    let mut runner = wsl2_runner();
    run(
        &request(&[]),
        &host(Os::Windows, Arch::Arm64),
        &mut runner,
        &machine,
        None,
    )
    .expect("sets up");
    assert!(
        lines(&runner)
            .iter()
            .any(|line| line.contains("aarch64-unknown-linux-musl")),
        "{:?}",
        lines(&runner)
    );
}

#[test]
fn a_linux_rayx_of_the_same_version_is_not_downloaded_again() {
    let machine = FakeMachine::new().with_wsl_distributions(vec![ubuntu(1000)]);
    let mut runner = wsl2_runner().respond(
        |spec| spec.args.last().is_some_and(|a| a.contains("--version")),
        Outcome::success().with_stdout("rayx 0.3.0\n"),
    );

    run(
        &request(&["--web"]),
        &windows(),
        &mut runner,
        &machine,
        None,
    )
    .expect("sets up");

    assert!(
        !lines(&runner).iter().any(|line| line.contains("curl")),
        "{:?}",
        lines(&runner)
    );
    assert!(
        lines(&runner)
            .last()
            .expect("a command")
            .contains("setup --web")
    );
}

#[test]
fn a_development_binary_is_copied_from_the_windows_drive() {
    let machine = FakeMachine::new().with_wsl_distributions(vec![ubuntu(1000)]);
    let mut runner = wsl2_runner();
    let mut asked = request(&[]);
    asked.binary_override = Some(PathBuf::from(r"C:\Work\rayx build\rayx"));

    run(&asked, &windows(), &mut runner, &machine, None).expect("sets up");

    let all = lines(&runner);
    let copy = all
        .iter()
        .find(|line| line.contains("cp "))
        .unwrap_or_else(|| panic!("a copy in {all:?}"));
    assert!(copy.contains("/mnt/c/Work/rayx build/rayx"), "{copy}");
    assert!(
        !all.iter().any(|line| line.contains("curl")),
        "the override replaces the download"
    );
}

#[test]
fn clone_copies_only_the_current_checkout_and_runs_setup_inside_it() {
    let machine = FakeMachine::new().with_wsl_distributions(vec![ubuntu(1000)]);
    let mut runner = wsl2_runner();
    let mut asked = request(&["--web"]);
    asked.clone = Some(String::new());
    let checkout = Checkout {
        root: PathBuf::from(r"D:\Work\projects\RayX\RayX"),
        branch: Some("feature/x".into()),
    };

    run(&asked, &windows(), &mut runner, &machine, Some(&checkout)).expect("sets up");

    // The scripts, as the arguments `bash -lc` receives (the printed command line escapes quotes).
    let scripts: Vec<String> = runner
        .specs()
        .iter()
        .filter_map(|spec| spec.args.last().cloned())
        .collect();
    let clones: Vec<&String> = scripts.iter().filter(|s| s.contains("clone")).collect();
    assert_eq!(
        clones.len(),
        1,
        "one clone, of this checkout only: {scripts:?}"
    );
    assert!(
        clones[0].contains(
            "git -c safe.directory='*' clone -b 'feature/x' '/mnt/d/Work/projects/RayX/RayX' \"$HOME/RayX\""
        ),
        "{}",
        clones[0]
    );
    let setup = scripts.last().expect("setup runs last");
    assert!(setup.starts_with("cd \"$HOME/RayX\" &&"), "{setup}");
    assert!(setup.ends_with("setup --web"), "{setup}");
}

#[test]
fn clone_needs_a_checkout() {
    let machine = FakeMachine::new().with_wsl_distributions(vec![ubuntu(1000)]);
    let mut runner = wsl2_runner();
    let mut asked = request(&[]);
    asked.clone = Some("~/work".into());

    let error = run(&asked, &windows(), &mut runner, &machine, None).expect_err("no checkout");

    assert!(error.to_string().contains("git checkout"), "{error}");
}

#[test]
fn windows_paths_translate_to_mounts() {
    for (windows, wsl) in [
        (r"C:\Users\dev\app", "/mnt/c/Users/dev/app"),
        (r"D:\Work\projects\RayX", "/mnt/d/Work/projects/RayX"),
        (r"\\?\E:\Repos\x", "/mnt/e/Repos/x"),
        ("c:/Users/dev/app", "/mnt/c/Users/dev/app"),
        (r"C:\", "/mnt/c"),
    ] {
        assert_eq!(
            to_wsl_path(Path::new(windows)).as_deref(),
            Ok(wsl),
            "{windows}"
        );
    }
    assert!(to_wsl_path(Path::new(r"\\server\share\x")).is_err());
    assert!(to_wsl_path(Path::new("relative/dir")).is_err());
}

#[test]
fn the_clone_directory_defaults_to_the_home_directory_and_refuses_shell_metacharacters() {
    assert_eq!(linux_dir("~/RayX").as_deref(), Ok("$HOME/RayX"));
    assert_eq!(linux_dir("~").as_deref(), Ok("$HOME"));
    assert_eq!(linux_dir("/srv/code").as_deref(), Ok("/srv/code"));
    for bad in ["~/a\"b", "~/$(rm -rf x)", "/tmp/a`b`"] {
        assert!(linux_dir(bad).is_err(), "{bad}");
    }
}

#[test]
fn an_existing_clone_is_fetched_not_replaced() {
    let script = clone_script("/mnt/d/x", Some("main"), "$HOME/x").expect("script");
    assert!(script.starts_with("if [ -d \"$HOME/x/.git\" ]; then git -C \"$HOME/x\" fetch"));
    assert!(
        script.contains("else git -c safe.directory='*' clone -b 'main' '/mnt/d/x' \"$HOME/x\"")
    );
    assert!(!script.contains("rm "), "never deletes a clone");
    assert!(
        script.contains("echo \"$HOME/x\" >> \"$HOME/.config/rayx/wsl-clones\""),
        "the clone is recorded for wsl status: {script}"
    );

    let detached = clone_script("/mnt/d/x", None, "$HOME/x").expect("script");
    assert!(!detached.contains(" -b "));
    assert!(clone_script("/mnt/d/o'brien", None, "$HOME/x").is_err());
}

#[test]
fn the_set_flags_reach_the_inner_setup_unchanged() {
    let cli = Cli::try_parse_from([
        "rayx",
        "setup",
        "--wsl",
        "--web",
        "--android",
        "--gpu",
        "--yes",
        "--clone",
        "~/work",
    ])
    .expect("parses");
    let Command::Setup(args) = cli.command else {
        panic!("setup");
    };
    assert_eq!(setup_flags(&args), ["--web", "--android", "--gpu", "--yes"]);
    assert_eq!(args.clone.as_deref(), Some("~/work"));

    let bare = Cli::try_parse_from(["rayx", "setup", "--wsl", "--clone"]).expect("parses");
    let Command::Setup(args) = bare.command else {
        panic!("setup");
    };
    assert_eq!(
        args.clone.as_deref(),
        Some(""),
        "--clone alone means the default directory"
    );

    let none = SetupArgs::default();
    assert!(setup_flags(&none).is_empty());
}

#[test]
fn path_dependencies_outside_the_checkout_are_never_cloned() {
    // The manifest points at siblings; `--clone` copies the checkout only. Cargo resolves the
    // project's dependencies inside WSL.
    let checkout = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        checkout.path().join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n[dependencies]\ngpux = { path = \"../Gpux/crates/gpui\" }\nrayx_reactive = { path = \"../RayX.Reactive/crates\" }\n",
    )
    .expect("manifest");
    let machine = FakeMachine::new().with_wsl_distributions(vec![ubuntu(1000)]);
    let mut runner = wsl2_runner();
    let mut asked = request(&[]);
    asked.clone = Some(String::new());
    let root = std::fs::canonicalize(checkout.path()).expect("canonical");
    let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\?\"));
    let checkout = Checkout {
        root: PathBuf::from(r"C:\Work\app"),
        branch: None,
    };
    assert!(root.join("Cargo.toml").is_file());

    run(&asked, &windows(), &mut runner, &machine, Some(&checkout)).expect("sets up");

    let scripts: Vec<String> = runner
        .specs()
        .iter()
        .filter_map(|spec| spec.args.last().cloned())
        .collect();
    let clones: Vec<&String> = scripts.iter().filter(|s| s.contains(" clone")).collect();
    assert_eq!(clones.len(), 1, "{scripts:?}");
    assert!(clones[0].contains("'/mnt/c/Work/app'"), "{}", clones[0]);
    for script in &scripts {
        for sibling in ["Gpux", "RayX.Reactive", "gpui"] {
            assert!(!script.contains(sibling), "{sibling} in {script}");
        }
    }
}
