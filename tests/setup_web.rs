//! The web set: the pinned nightly, the wasm32 targets, `wasm-bindgen-cli` and LLVM per OS.

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

const NIGHTLY: &str = "nightly-2026-06-23";
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

#[test]
fn a_bare_machine_gets_the_nightly_the_targets_and_wasm_bindgen() {
    let tools = Rc::new(RefCell::new(Tools::default()));
    let (code, text) = check(
        &env(Os::Linux, Arch::X64),
        &machine(),
        &mut runner(&tools, Os::Linux),
    );
    assert_eq!(code, 1, "{text}");

    assert!(
        text.contains(&format!(
            "rustup toolchain install {NIGHTLY} --profile minimal --component rust-src \
             --target wasm32-unknown-unknown"
        )),
        "{text}"
    );
    assert!(
        text.contains("rustup target add wasm32-unknown-unknown --toolchain 1.95.0"),
        "the project's toolchain gets the wasm32 target:\n{text}"
    );
    assert!(
        text.contains("cargo install wasm-bindgen-cli --version 0.2.129 --force --locked"),
        "{text}"
    );
}

#[test]
fn an_unpinned_project_uses_stable_and_the_latest_wasm_bindgen() {
    let mut env = env(Os::Linux, Arch::X64);
    env.pins.rust_toolchain = None;
    env.pins.wasm_bindgen = None;
    let tools = Rc::new(RefCell::new(Tools::default()));
    let (_, text) = check(&env, &machine(), &mut runner(&tools, Os::Linux));
    assert!(
        text.contains("rustup target add wasm32-unknown-unknown --toolchain stable"),
        "{text}"
    );
    assert!(
        text.contains("cargo install wasm-bindgen-cli --locked"),
        "{text}"
    );
    assert!(!text.contains("--version"), "{text}");
}

#[test]
fn a_provisioned_web_set_has_nothing_to_do() {
    let tools = Rc::new(RefCell::new(Tools {
        toolchains: vec![
            format!("{NIGHTLY}-x86_64-unknown-linux-gnu"),
            "1.95.0-x86_64-unknown-linux-gnu (default)".into(),
        ],
        components: BTreeMap::from([(
            NIGHTLY.into(),
            vec!["rust-src".into(), "cargo-x86_64".into()],
        )]),
        targets: BTreeMap::from([
            (NIGHTLY.into(), vec!["wasm32-unknown-unknown".into()]),
            ("1.95.0".into(), vec!["wasm32-unknown-unknown".into()]),
        ]),
        wasm_bindgen: Some("0.2.129".into()),
    }));
    let (_, text) = check(
        &env(Os::Linux, Arch::X64),
        &machine(),
        &mut runner(&tools, Os::Linux),
    );
    // The base steps are not part of this assertion: only the web steps must be satisfied.
    for step in ["web: ", "wasm-bindgen"] {
        assert!(
            text.lines()
                .filter(|l| l.contains(step))
                .all(|l| l.starts_with("[ok     ]")),
            "{text}"
        );
    }
    assert!(
        text.contains("[ok     ] web: nightly-2026-06-23 with rust-src"),
        "{text}"
    );
}

#[test]
fn a_nightly_without_rust_src_or_the_target_is_not_enough() {
    let installed = |components: Vec<String>, targets: Vec<String>| {
        Rc::new(RefCell::new(Tools {
            toolchains: vec![format!("{NIGHTLY}-x86_64-unknown-linux-gnu")],
            components: BTreeMap::from([(NIGHTLY.into(), components)]),
            targets: BTreeMap::from([(NIGHTLY.into(), targets)]),
            wasm_bindgen: Some("0.2.129".into()),
        }))
    };
    let env = env(Os::Linux, Arch::X64);
    let no_src = installed(vec![], vec!["wasm32-unknown-unknown".into()]);
    let (_, text) = check(&env, &machine(), &mut runner(&no_src, Os::Linux));
    assert!(text.contains("[missing] web: nightly-2026-06-23"), "{text}");
    assert!(text.contains("rust-src component is missing"), "{text}");

    let no_target = installed(vec!["rust-src".into()], vec![]);
    let (_, text) = check(&env, &machine(), &mut runner(&no_target, Os::Linux));
    assert!(
        text.contains("the wasm32-unknown-unknown target is missing"),
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
    // Linux: the base apt packages carry clang, llvm and lld.
    for package in ["clang", "llvm", "lld", "libclang-dev"] {
        assert!(BASE_PACKAGES.contains(&package), "{package}");
    }
    let tools = Rc::new(RefCell::new(Tools::default()));
    let (_, linux) = check(
        &env(Os::Linux, Arch::X64),
        &machine(),
        &mut runner(&tools, Os::Linux),
    );
    assert!(linux.contains("clang llvm lld"), "{linux}");
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
