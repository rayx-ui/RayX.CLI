//! The web set for NSH: the wasm32 target on the project's toolchain, `wasm-bindgen-cli` only when
//! the project's lock has `wasm-bindgen`, and LLVM per OS. No nightly and no `rust-src` (D3).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::UserPath;
use rayx_cli::host::{Arch, Distro, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::{Pin, PinSource, Pins};
use rayx_cli::setup::linux::BASE_PACKAGES;
use rayx_cli::setup::{Cx, FakeMachine, PlanEnv, Set, plan, run_plan};

const UBUNTU: &str = "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n";

fn pin(value: &str) -> Pin {
    Pin {
        value: value.to_string(),
        source: PinSource::Project,
    }
}

fn env(os: Os, arch: Arch) -> PlanEnv {
    let mut pins = Pins::resolve(None);
    pins.wasm_bindgen = Some(pin("0.2.129"));
    pins.rust_toolchain = Some(pin("1.95.0"));
    PlanEnv {
        host: HostFacts {
            os,
            arch,
            emulated: false,
            wsl: false,
            distro: (os == Os::Linux).then(|| Distro::parse(UBUNTU)),
        },
        pins,
        project_root: Some(PathBuf::from("/work/rayx")),
        tools_node: None,
        nsh: None,
        yes: false,
    }
}

/// What rustup and wasm-bindgen report on the fake machine.
#[derive(Default)]
struct Tools {
    toolchains: Vec<String>,
    components: BTreeMap<String, Vec<String>>,
    targets: BTreeMap<String, Vec<String>>,
    wasm_bindgen: Option<String>,
}

fn runner(tools: &Rc<RefCell<Tools>>, os: Os) -> Runner {
    let state = tools.clone();
    Runner::record()
        .with_os(os)
        .with_root(false)
        .responder(move |spec| {
            let tools = state.borrow();
            let program = spec.program.rsplit(['/', '\\']).next().unwrap_or("");
            if program.starts_with("rustup") {
                let listing = |map: &BTreeMap<String, Vec<String>>| {
                    let toolchain = spec
                        .args
                        .iter()
                        .position(|a| a == "--toolchain")
                        .and_then(|i| spec.args.get(i + 1))?;
                    Some(
                        Outcome::success().with_stdout(
                            map.get(toolchain)
                                .map(|items| items.join("\n"))
                                .unwrap_or_default(),
                        ),
                    )
                };
                return match spec.args.first().map(String::as_str) {
                    Some("toolchain") => Some(
                        Outcome::success().with_stdout(
                            tools
                                .toolchains
                                .iter()
                                .map(|t| format!("{t}\n"))
                                .collect::<String>(),
                        ),
                    ),
                    Some("component") => listing(&tools.components),
                    Some("target") => listing(&tools.targets),
                    _ => None,
                };
            }
            if program.starts_with("wasm-bindgen") {
                return Some(match &tools.wasm_bindgen {
                    Some(version) => {
                        Outcome::success().with_stdout(format!("wasm-bindgen {version}\n"))
                    }
                    None => Outcome::failure(127),
                });
            }
            None
        })
}

fn check(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> (u8, String) {
    let plan = plan(env, &[Set::Web]).expect("plan");
    let mut path = UserPath::Profiles {
        files: vec![PathBuf::from("/home/dev/.profile")],
        home: PathBuf::from("/home/dev"),
    };
    let mut out = Vec::new();
    let mut cx = Cx::new(env, runner, machine, &mut path);
    let args = SetupArgs {
        check: true,
        web: true,
        ..SetupArgs::default()
    };
    let code = run_plan(&args, &plan, &mut cx, &mut out);
    (code, String::from_utf8(out).expect("UTF-8"))
}

fn machine() -> FakeMachine {
    FakeMachine::new()
        .with_home("/home/dev")
        .with_program("rustup", "/home/dev/.cargo/bin/rustup")
        .with_program("cargo", "/home/dev/.cargo/bin/cargo")
}

fn step_ids(env: &PlanEnv) -> Vec<&'static str> {
    plan(env, &[Set::Web])
        .expect("plan")
        .steps
        .iter()
        .filter(|step| step.set == Set::Web)
        .map(|step| step.id)
        .collect()
}

#[test]
fn a_bare_machine_gets_the_wasm32_target_and_the_locked_wasm_bindgen() {
    let tools = Rc::new(RefCell::new(Tools::default()));
    let (code, text) = check(
        &env(Os::Linux, Arch::X64),
        &machine(),
        &mut runner(&tools, Os::Linux),
    );
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("rustup target add wasm32-unknown-unknown --toolchain 1.95.0"),
        "the project's toolchain gets the wasm32 target:\n{text}"
    );
    assert!(
        text.contains("cargo install wasm-bindgen-cli --version 0.2.129 --force --locked"),
        "{text}"
    );
    assert!(
        text.contains("[missing] web: wasm-bindgen-cli 0.2.129 (the project's Cargo.lock)"),
        "{text}"
    );
}

#[test]
fn the_web_set_installs_no_nightly_and_no_rust_src() {
    for os in [Os::Linux, Os::MacOs, Os::Windows] {
        let tools = Rc::new(RefCell::new(Tools::default()));
        let mac = machine().with_file("/opt/homebrew/bin/brew");
        let (_, text) = check(&env(os, Arch::Arm64), &mac, &mut runner(&tools, os));
        let web: Vec<&str> = text
            .lines()
            .skip_while(|line| !line.contains("] web: "))
            .collect();
        assert!(!web.is_empty(), "{text}");
        for line in web {
            assert!(
                !line.contains("nightly") && !line.contains("rust-src"),
                "{os:?}: {line}"
            );
        }
        assert!(!text.contains("toolchain install nightly"), "{text}");
    }
}

#[test]
fn without_wasm_bindgen_in_the_lock_there_is_no_wasm_bindgen_step() {
    let mut env = env(Os::Linux, Arch::X64);
    env.pins.wasm_bindgen = None;
    assert_eq!(step_ids(&env), ["web-wasm-target"]);
    let tools = Rc::new(RefCell::new(Tools::default()));
    let (_, text) = check(&env, &machine(), &mut runner(&tools, Os::Linux));
    assert!(!text.contains("wasm-bindgen"), "{text}");

    // Outside a project: the stable toolchain gets the target.
    env.pins.rust_toolchain = None;
    let (_, text) = check(&env, &machine(), &mut runner(&tools, Os::Linux));
    assert!(
        text.contains("rustup target add wasm32-unknown-unknown --toolchain stable"),
        "{text}"
    );

    let mut mac = self::env(Os::MacOs, Arch::Arm64);
    mac.pins.wasm_bindgen = None;
    assert_eq!(step_ids(&mac), ["web-wasm-target", "web-llvm"]);
    assert_eq!(
        step_ids(&self::env(Os::MacOs, Arch::Arm64)),
        ["web-wasm-target", "wasm-bindgen", "web-llvm"]
    );
}

#[test]
fn a_provisioned_web_set_has_nothing_to_do() {
    let tools = Rc::new(RefCell::new(Tools {
        toolchains: vec!["1.95.0-x86_64-unknown-linux-gnu (default)".into()],
        components: BTreeMap::new(),
        targets: BTreeMap::from([("1.95.0".into(), vec!["wasm32-unknown-unknown".into()])]),
        wasm_bindgen: Some("0.2.129".into()),
    }));
    let (_, text) = check(
        &env(Os::Linux, Arch::X64),
        &machine(),
        &mut runner(&tools, Os::Linux),
    );
    // The base steps are not part of this assertion: only the web steps must be satisfied.
    let web: Vec<&str> = text.lines().filter(|l| l.contains("] web: ")).collect();
    assert_eq!(web.len(), 2, "{text}");
    assert!(web.iter().all(|l| l.starts_with("[ok     ]")), "{text}");
    assert!(
        text.contains(
            "[ok     ] web: wasm-bindgen-cli 0.2.129 (the project's Cargo.lock) (found 0.2.129)"
        ),
        "{text}"
    );
}

#[test]
fn another_wasm_bindgen_version_is_replaced_with_the_pinned_one() {
    let tools = Rc::new(RefCell::new(Tools {
        wasm_bindgen: Some("0.2.100".into()),
        ..Tools::default()
    }));
    let (_, text) = check(
        &env(Os::Linux, Arch::X64),
        &machine(),
        &mut runner(&tools, Os::Linux),
    );
    assert!(
        text.contains("version 0.2.100 is installed, the project pins 0.2.129"),
        "{text}"
    );
    assert!(
        text.contains("--version 0.2.129 --force --locked"),
        "{text}"
    );
}

#[test]
fn clang_and_llvm_come_with_the_base_set_on_linux_and_windows_and_from_homebrew_on_macos() {
    // Linux: the base apt packages carry clang, llvm (with llvm-ar) and libclang.
    for package in ["clang", "llvm", "libclang-dev"] {
        assert!(BASE_PACKAGES.contains(&package), "{package}");
    }
    let tools = Rc::new(RefCell::new(Tools::default()));
    let (_, linux) = check(
        &env(Os::Linux, Arch::X64),
        &machine(),
        &mut runner(&tools, Os::Linux),
    );
    assert!(linux.contains("clang llvm libclang-dev"), "{linux}");
    assert!(!linux.contains("brew"), "{linux}");

    // Windows: LLVM.LLVM is a base step.
    let (_, windows) = check(
        &env(Os::Windows, Arch::Arm64),
        &machine(),
        &mut runner(&tools, Os::Windows),
    );
    assert!(
        windows.contains("winget install --id LLVM.LLVM"),
        "{windows}"
    );

    // macOS: Apple's clang cannot target wasm32, so Homebrew's LLVM is a web step.
    let mac = machine().with_file("/opt/homebrew/bin/brew");
    let (_, text) = check(
        &env(Os::MacOs, Arch::Arm64),
        &mac,
        &mut runner(&tools, Os::MacOs),
    );
    assert!(
        text.contains("[missing] web: LLVM with the wasm32 backend (Homebrew)"),
        "{text}"
    );
    assert!(
        text.contains("/opt/homebrew/bin/brew install llvm"),
        "{text}"
    );
}
