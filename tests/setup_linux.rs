//! The base set on Ubuntu and Debian: apt packages, rustup, the project toolchain and PATH.

use std::path::PathBuf;

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::UserPath;
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::{Pin, PinSource, Pins};
use rayx_cli::setup::linux::BASE_PACKAGES;
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, SetupError, plan, run_plan};

const UBUNTU_2404: &str = "ID=ubuntu\nVERSION_ID=\"24.04\"\nVERSION_CODENAME=noble\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\nID_LIKE=debian\n";
const DEBIAN_12: &str =
    "ID=debian\nVERSION_ID=\"12\"\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\n";
const FEDORA_41: &str = "ID=fedora\nVERSION_ID=41\nPRETTY_NAME=\"Fedora Linux 41\"\n";

fn env(os_release: &str, wsl: bool, toolchain: Option<&str>, yes: bool) -> PlanEnv {
    let mut pins = Pins::resolve(None);
    pins.rust_toolchain = toolchain.map(|channel| Pin {
        value: channel.to_string(),
        source: PinSource::Project,
    });
    PlanEnv {
        host: HostFacts {
            os: Os::Linux,
            arch: Arch::X64,
            emulated: false,
            wsl,
            distro: Some(Distro::parse(os_release)),
        },
        pins,
        project_root: toolchain.map(|_| PathBuf::from("/home/dev/rayx")),
        yes,
    }
}

fn machine() -> FakeMachine {
    FakeMachine::new().with_home("/home/dev")
}

/// A bare machine: nothing is installed.
fn bare_runner() -> Runner {
    Runner::record().with_os(Os::Linux).with_root(false)
}

fn check(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> (u8, String) {
    let plan = plan(env, &[Set::Base]).expect("plan");
    let mut path = UserPath::Profiles {
        files: vec![PathBuf::from("/home/dev/.profile")],
        home: PathBuf::from("/home/dev"),
    };
    let mut out = Vec::new();
    let mut cx = Cx::new(env, runner, machine, &mut path);
    let args = SetupArgs {
        check: true,
        ..SetupArgs::default()
    };
    let code = run_plan(&args, &plan, &mut cx, &mut out);
    (code, String::from_utf8(out).expect("UTF-8"))
}

#[test]
fn ubuntu_base_plan_is_one_batched_apt_install_then_rustup_toolchain_and_path() {
    let env = env(UBUNTU_2404, false, None, true);
    let (code, text) = check(&env, &machine(), &mut bare_runner());
    assert_eq!(code, 1, "{text}");

    assert!(text.contains("sudo apt-get update"), "{text}");
    let install = format!("sudo apt-get install -y {}", BASE_PACKAGES.join(" "));
    assert!(
        text.contains(&install),
        "one install with the full list:\n{text}"
    );
    assert_eq!(text.matches("apt-get install").count(), 1, "{text}");

    let rustup = text
        .lines()
        .find(|l| l.contains("sh.rustup.rs"))
        .unwrap_or_else(|| panic!("rustup installer line:\n{text}"));
    assert!(rustup.contains("--default-toolchain none"), "{rustup}");
    assert!(rustup.contains("--no-modify-path"), "{rustup}");
    assert!(text.contains("rustup toolchain install stable"), "{text}");
    let cargo_bin = PathBuf::from("/home/dev").join(".cargo").join("bin");
    assert!(
        text.contains(&format!("add {} to the user PATH", cargo_bin.display())),
        "{text}"
    );

    let order: Vec<usize> = ["system packages", "rustup (", "Rust stable", "~/.cargo/bin"]
        .iter()
        .map(|needle| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}:\n{text}"))
        })
        .collect();
    assert!(
        order.windows(2).all(|w| w[0] < w[1]),
        "steps are ordered:\n{text}"
    );
}

#[test]
fn the_package_list_is_the_documented_one() {
    let expected = "build-essential pkg-config git curl jq cmake clang llvm lld libclang-dev \
        libssl-dev libzstd-dev libsqlite3-dev libglib2.0-dev libxcb1-dev libxkbcommon-dev \
        libxkbcommon-x11-dev libwayland-dev libfontconfig1-dev libfreetype-dev libasound2-dev \
        libvulkan1 mesa-vulkan-drivers xdg-utils";
    assert_eq!(BASE_PACKAGES.join(" "), expected);
}

#[test]
fn the_same_plan_applies_inside_wsl_and_on_debian() {
    let ubuntu = env(UBUNTU_2404, false, None, false);
    let wsl = env(UBUNTU_2404, true, None, false);
    let debian = env(DEBIAN_12, false, None, false);
    let ids = |env: &PlanEnv| -> Vec<&'static str> {
        plan(env, &[Set::Base])
            .expect("plan")
            .steps
            .iter()
            .map(|s| s.id)
            .collect()
    };
    assert_eq!(
        ids(&ubuntu),
        ["apt-packages", "rustup", "rust-toolchain", "cargo-path"]
    );
    assert_eq!(ids(&wsl), ids(&ubuntu));
    assert_eq!(ids(&debian), ids(&ubuntu));

    let (code, text) = check(&wsl, &machine(), &mut bare_runner());
    assert_eq!(code, 1);
    assert!(text.contains("sudo apt-get install"), "{text}");
}

#[test]
fn an_unsupported_distribution_fails_with_an_actionable_message() {
    let fedora = env(FEDORA_41, false, None, false);
    let Err(error) = plan(&fedora, &[Set::Base]) else {
        panic!("Fedora must not plan");
    };
    assert!(matches!(error, SetupError::Unsupported(_)));
    let message = error.to_string();
    assert!(
        message.contains("Ubuntu") && message.contains("Debian"),
        "{message}"
    );
    assert!(
        message.contains("Fedora Linux 41"),
        "names what it found: {message}"
    );
    assert!(
        message.contains("libvulkan1"),
        "lists the packages to install: {message}"
    );
}

#[test]
fn inside_a_project_the_toolchain_step_installs_from_rust_toolchain_toml() {
    let env = env(UBUNTU_2404, false, Some("1.95.0"), false);
    let (_, text) = check(&env, &machine(), &mut bare_runner());
    assert!(text.contains("Rust toolchain 1.95.0"), "{text}");
    assert!(
        text.contains("cd /home/dev/rayx && rustup toolchain install"),
        "installs the project's pinned toolchain from its directory:\n{text}"
    );

    // An installed 1.95.0 satisfies the step; another toolchain does not.
    let with_toolchain = |listing: &'static str| {
        let mut runner = bare_runner().responder(move |spec| {
            (spec.program.ends_with("rustup")).then(|| Outcome::success().with_stdout(listing))
        });
        let machine = machine().with_program("rustup", "/home/dev/.cargo/bin/rustup");
        check(&env, &machine, &mut runner).1
    };
    let met = with_toolchain("1.95.0-x86_64-unknown-linux-gnu\n");
    assert!(
        met.contains("[ok     ] base: Rust toolchain 1.95.0"),
        "{met}"
    );
    let unmet = with_toolchain("stable-x86_64-unknown-linux-gnu (default)\n");
    assert!(
        unmet.contains("[missing] base: Rust toolchain 1.95.0"),
        "{unmet}"
    );
}

#[test]
fn a_fully_provisioned_host_has_nothing_to_do() {
    let env = env(UBUNTU_2404, false, None, false);
    let installed: String = BASE_PACKAGES
        .iter()
        .map(|p| format!("{p}\tii \n"))
        .collect();
    let mut runner = bare_runner().responder(move |spec| match spec.program.as_str() {
        "dpkg-query" => Some(Outcome::success().with_stdout(installed.clone())),
        program if program.ends_with("rustup") => {
            Some(Outcome::success().with_stdout("stable-x86_64-unknown-linux-gnu (default)\n"))
        }
        _ => None,
    });
    let machine = machine()
        .with_program("rustup", "/home/dev/.cargo/bin/rustup")
        .with_program("cargo", "/home/dev/.cargo/bin/cargo");
    let (code, text) = check(&env, &machine, &mut runner);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("Everything is installed."), "{text}");
    assert!(!text.contains("[missing]"), "{text}");
}

#[test]
fn the_cargo_bin_directory_counts_once_it_is_on_the_user_path() {
    let env = env(UBUNTU_2404, false, None, false);
    let dir = tempfile::tempdir().expect("temp dir");
    let profile = dir.path().join(".profile");
    std::fs::write(
        &profile,
        "# >>> rayx >>>\nexport PATH=\"$HOME/.cargo/bin:$PATH\"\n# <<< rayx <<<\n",
    )
    .expect("profile");
    let mut path = UserPath::Profiles {
        files: vec![profile],
        home: dir.path().to_path_buf(),
    };
    let machine = FakeMachine::new().with_home(dir.path());
    let plan = plan(&env, &[Set::Base]).expect("plan");
    let mut runner = bare_runner();
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    let checked = rayx_cli::setup::check(&plan, &mut cx);
    let cargo_path = plan
        .steps
        .iter()
        .position(|s| s.id == "cargo-path")
        .expect("step");
    assert!(checked.probed[cargo_path].satisfied);
}

#[test]
fn only_dpkg_status_i_counts_as_installed() {
    let env = env(UBUNTU_2404, false, None, false);
    let machine = machine();
    let mut path = UserPath::Profiles {
        files: vec![PathBuf::from("/home/dev/.profile")],
        home: PathBuf::from("/home/dev"),
    };
    // want, status, error: `ii` installed, `hi` held but installed, `iU` unpacked only,
    // `iF` half-configured, `rc` removed with config left.
    let mut runner = bare_runner().responder(|spec| {
        (spec.program == "dpkg-query").then(|| {
            Outcome::success().with_stdout("a\tii \nb\thi \nc\tiU \nd\tiF \ne\trc \n".to_string())
        })
    });
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    let packages: Vec<String> = ["a", "b", "c", "d", "e", "f"].map(String::from).to_vec();
    let probed = rayx_cli::setup::linux::probe_packages(&mut cx, &packages);
    assert!(!probed.satisfied);
    assert_eq!(probed.missing, ["c", "d", "e", "f"]);
}

#[test]
fn outside_a_project_the_stable_toolchain_is_installed_and_made_the_default() {
    let env = env(UBUNTU_2404, false, None, false);
    let (_, text) = check(&env, &machine(), &mut bare_runner());
    assert!(text.contains("rustup toolchain install stable"), "{text}");
    assert!(
        text.contains("rustup default stable"),
        "rustup is installed without a default toolchain:\n{text}"
    );

    // An installed toolchain that is not the default does not satisfy the step.
    let mut runner = bare_runner().responder(|spec| {
        spec.program
            .ends_with("rustup")
            .then(|| Outcome::success().with_stdout("nightly-x86_64-unknown-linux-gnu\n"))
    });
    let machine = machine().with_program("rustup", "/home/dev/.cargo/bin/rustup");
    let (_, text) = check(&env, &machine, &mut runner);
    assert!(
        text.contains("[missing] base: Rust stable toolchain"),
        "{text}"
    );
}
