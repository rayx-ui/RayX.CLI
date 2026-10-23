//! The base set on Windows x64 and ARM64: Build Tools components, LLVM, VC++ redistributables,
//! the WebView2 Runtime, the native rustup host and the missing-winget error. There is no `fxc.exe`
//! step: NSH compiles no shaders with the Windows SDK's compiler.
//!
//! Paths are built with `join` so the same tests pass when this suite runs on Linux or macOS.

use std::path::PathBuf;

use rayx_cli::cli::SetupArgs;
use rayx_cli::host::path_env::{MemoryPathStore, PathValue, UserPath};
use rayx_cli::host::{Arch, HostFacts, Os, Outcome, Runner};
use rayx_cli::project::Pins;
use rayx_cli::setup::windows::native_triple;
use rayx_cli::setup::{Cx, FakeMachine, Plan, PlanEnv, Set, StepOutcome, execute, plan, run_plan};

const PF: &str = r"C:\Program Files";
const PF86: &str = r"C:\Program Files (x86)";

fn env(arch: Arch, emulated: bool, yes: bool) -> PlanEnv {
    PlanEnv {
        host: HostFacts {
            os: Os::Windows,
            arch,
            emulated,
            wsl: false,
            distro: None,
        },
        pins: Pins::resolve(None),
        project_root: None,
        tools_node: None,
        yes,
    }
}

fn path(parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(PathBuf::new(), |acc, part| acc.join(part))
}

fn vswhere() -> PathBuf {
    path(&[PF86, "Microsoft Visual Studio", "Installer", "vswhere.exe"])
}

/// A Windows machine with nothing of the base set installed.
fn sdk_machine() -> FakeMachine {
    FakeMachine::new()
        .with_home(path(&[r"C:\Users", "dev"]))
        .with_env("ProgramFiles", PF)
        .with_env("ProgramFiles(x86)", PF86)
        .with_env("TEMP", r"C:\Temp")
}

/// Where the Evergreen WebView2 Runtime puts one of its versions.
fn webview2(version: &str) -> PathBuf {
    path(&[
        PF86,
        "Microsoft",
        "EdgeWebView",
        "Application",
        version,
        "msedgewebview2.exe",
    ])
}

fn runner() -> Runner {
    Runner::record().with_os(Os::Windows)
}

fn user_path() -> UserPath {
    UserPath::Windows(Box::new(MemoryPathStore::new(Some(PathValue::Expandable(
        r"%USERPROFILE%\AppData\Local\Microsoft\WindowsApps".into(),
    )))))
}

fn check(env: &PlanEnv, machine: &FakeMachine, runner: &mut Runner) -> (u8, String) {
    let plan = plan(env, &[Set::Base]).expect("plan");
    let mut path = user_path();
    let mut out = Vec::new();
    let mut cx = Cx::new(env, runner, machine, &mut path);
    let args = SetupArgs {
        check: true,
        ..SetupArgs::default()
    };
    let code = run_plan(&args, &plan, &mut cx, &mut out);
    (code, String::from_utf8(out).expect("UTF-8"))
}

/// A machine with winget but nothing else installed.
fn winget_only(runner: Runner) -> Runner {
    runner.responder(|spec| match spec.program.as_str() {
        "winget" if spec.args.first().is_some_and(|a| a == "list") => Some(Outcome::failure(1)),
        _ => None,
    })
}

#[test]
fn x64_plan_installs_build_tools_llvm_redistributable_and_the_x64_rustup() {
    let env = env(Arch::X64, false, true);
    let (code, text) = check(&env, &sdk_machine(), &mut winget_only(runner()));
    assert_eq!(code, 1, "{text}");

    assert!(
        text.contains(
            "winget install --id Microsoft.VisualStudio.2022.BuildTools --exact \
             --accept-package-agreements --accept-source-agreements --override \
             \"--wait --passive --includeRecommended --add Microsoft.VisualStudio.Workload.VCTools\""
        ),
        "{text}"
    );
    assert!(
        !text.contains("VC.Tools.ARM64"),
        "no ARM64 component on x64:\n{text}"
    );
    assert!(
        text.contains("winget install --id Microsoft.VCRedist.2015+.x64 --exact"),
        "{text}"
    );
    assert!(!text.contains("VCRedist.2015+.arm64"), "{text}");
    assert!(
        text.contains("winget install --id LLVM.LLVM --exact"),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "add {} to the user PATH",
            path(&[PF, "LLVM", "bin"]).display()
        )),
        "{text}"
    );
    assert!(
        text.contains("x86_64-pc-windows-msvc/rustup-init.exe"),
        "{text}"
    );
    assert!(
        text.contains("--default-host x86_64-pc-windows-msvc --default-toolchain none"),
        "{text}"
    );
    assert!(!text.contains("aarch64"), "{text}");
}

#[test]
fn arm64_plan_adds_the_arm64_component_redistributables_and_the_native_rustup() {
    // An x64 `rayx` emulated on ARM64 still sets up the native tools.
    let env = env(Arch::Arm64, true, false);
    let (code, text) = check(&env, &sdk_machine(), &mut winget_only(runner()));
    assert_eq!(code, 1, "{text}");

    assert!(
        text.contains(
            "--override \"--wait --passive --includeRecommended --add \
             Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.VC.Tools.ARM64\""
        ),
        "{text}"
    );
    assert!(text.contains("Microsoft.VCRedist.2015+.arm64"), "{text}");
    assert!(text.contains("Microsoft.VCRedist.2015+.x64"), "{text}");
    assert!(
        !text.contains("--accept-package-agreements"),
        "agreements are accepted only with --yes:\n{text}"
    );
    assert!(
        text.contains("aarch64-pc-windows-msvc/rustup-init.exe"),
        "{text}"
    );
    assert!(
        text.contains("--default-host aarch64-pc-windows-msvc --default-toolchain none"),
        "{text}"
    );
    assert_eq!(native_triple(Arch::Arm64), "aarch64-pc-windows-msvc");
    assert_eq!(native_triple(Arch::X64), "x86_64-pc-windows-msvc");
}

#[test]
fn the_plan_has_no_fxc_step_and_a_webview2_step_after_llvm() {
    for arch in [Arch::X64, Arch::Arm64] {
        let ids: Vec<&str> = plan(&env(arch, false, false), &[Set::Base])
            .expect("plan")
            .steps
            .iter()
            .map(|step| step.id)
            .collect();
        assert_eq!(
            ids,
            [
                "winget",
                "build-tools",
                "vc-redist",
                "llvm",
                "llvm-path",
                "webview2",
                "rustup",
                "rustup-host",
                "rust-toolchain",
                "cargo-path"
            ],
            "{arch:?}"
        );
    }
    let (_, text) = check(
        &env(Arch::X64, false, false),
        &sdk_machine(),
        &mut winget_only(runner()),
    );
    assert!(!text.contains("fxc"), "{text}");
}

#[test]
fn a_missing_webview2_runtime_installs_through_winget() {
    let (code, text) = check(
        &env(Arch::X64, false, true),
        &sdk_machine(),
        &mut winget_only(runner()),
    );
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("[missing] base: the WebView2 Runtime (web views)"),
        "{text}"
    );
    assert!(
        text.contains(
            "winget install --id Microsoft.EdgeWebView2Runtime --exact \
             --accept-package-agreements --accept-source-agreements"
        ),
        "{text}"
    );
}

#[test]
fn an_installed_webview2_runtime_is_found_in_its_directory_or_by_winget() {
    // Windows 11 ships it: the Evergreen directory has a version with the executable.
    let machine = sdk_machine().with_file(webview2("130.0.2849.80"));
    let (_, text) = check(
        &env(Arch::X64, false, false),
        &machine,
        &mut winget_only(runner()),
    );
    assert!(
        text.contains("[ok     ] base: the WebView2 Runtime (web views) (found 130.0.2849.80)"),
        "{text}"
    );
    assert!(!text.contains("EdgeWebView2Runtime"), "{text}");

    // Elsewhere winget knows it.
    let mut runner = runner().responder(|spec| match spec.program.as_str() {
        "winget"
            if spec
                .args
                .iter()
                .any(|a| a == "Microsoft.EdgeWebView2Runtime") =>
        {
            Some(Outcome::success())
        }
        "winget" if spec.args.first().is_some_and(|a| a == "list") => Some(Outcome::failure(1)),
        _ => None,
    });
    let (_, text) = check(&env(Arch::X64, false, false), &sdk_machine(), &mut runner);
    assert!(
        text.contains("[ok     ] base: the WebView2 Runtime (web views)\n"),
        "{text}"
    );
}

#[test]
fn existing_build_tools_without_the_arm64_component_are_modified_in_place() {
    let env = env(Arch::Arm64, false, false);
    let vs = path(&[PF86, "Microsoft Visual Studio", "2022", "BuildTools"]);
    let machine = sdk_machine().with_file(vswhere());
    let vs_path = vs.display().to_string();
    let mut runner = winget_only(runner()).responder({
        let vs_path = vs_path.clone();
        move |spec| {
            if spec.program.ends_with("vswhere.exe") {
                // VS exists, but not with the ARM64 component.
                return Some(if spec.args.iter().any(|a| a.contains("ARM64")) {
                    Outcome::success()
                } else {
                    Outcome::success().with_stdout(format!("{vs_path}\n"))
                });
            }
            if spec.program == "winget" && spec.args.first().is_some_and(|a| a == "list") {
                return Some(Outcome::failure(1));
            }
            None
        }
    });
    let (_, text) = check(&env, &machine, &mut runner);
    let setup = path(&[PF86, "Microsoft Visual Studio", "Installer", "setup.exe"]);
    let expected = format!("modify --installPath \"{vs_path}\"");
    assert!(text.contains(&expected), "{text}");
    assert!(text.contains(&setup.display().to_string()), "{text}");
    assert!(
        text.contains("--add Microsoft.VisualStudio.Component.VC.Tools.ARM64"),
        "{text}"
    );
    assert!(
        !text.contains("winget install --id Microsoft.VisualStudio.2022.BuildTools"),
        "an installed Visual Studio is modified, not reinstalled:\n{text}"
    );
}

#[test]
fn a_missing_winget_stops_setup_with_the_app_installer_instruction() {
    let env = env(Arch::X64, false, false);
    let plan: Plan = plan(&env, &[Set::Base]).expect("plan");
    let machine = sdk_machine();
    let mut path = user_path();
    // Every program is missing: `winget --version` cannot even start.
    let mut real = Runner::record()
        .with_os(Os::Windows)
        .responder(|spec| (spec.program == "winget").then(|| Outcome::failure(1)));
    let mut cx = Cx::new(&env, &mut real, &machine, &mut path);
    let report = execute(&plan, &mut cx, &mut Vec::new());
    let message = report.failure().expect("setup stops");
    assert!(message.contains("App Installer"), "{message}");
    assert!(message.contains("aka.ms/getwinget"), "{message}");
    assert_eq!(report.steps[0].1, StepOutcome::Failed(message.to_string()));
    assert!(
        report.steps[1..]
            .iter()
            .all(|(_, o)| *o == StepOutcome::NotRun),
        "nothing runs after the missing winget"
    );
}

#[test]
fn a_rustup_with_the_emulated_host_on_arm64_is_switched_to_the_native_one() {
    let env = env(Arch::Arm64, true, false);
    let machine = sdk_machine().with_program(
        "rustup",
        path(&[r"C:\Users\dev", ".cargo", "bin", "rustup.exe"]),
    );
    let mut runner = winget_only(runner()).responder(|spec| {
        (spec.program.ends_with("rustup") || spec.program.ends_with("rustup.exe")).then(|| {
            if spec.args.first().is_some_and(|a| a == "show") {
                Outcome::success().with_stdout("Default host: x86_64-pc-windows-msvc\n")
            } else {
                Outcome::success().with_stdout("stable-x86_64-pc-windows-msvc (default)\n")
            }
        })
    });
    let (_, text) = check(&env, &machine, &mut runner);
    assert!(
        text.contains("[missing] base: rustup default host aarch64-pc-windows-msvc"),
        "{text}"
    );
    assert!(
        text.contains("set default-host aarch64-pc-windows-msvc"),
        "{text}"
    );
    // The x64 toolchain already installed is not the native one, so it does not count.
    assert!(
        text.contains("[missing] base: Rust stable toolchain"),
        "an emulated x64 toolchain must not satisfy an ARM64 host:\n{text}"
    );
    assert!(text.contains("rustup toolchain install stable"), "{text}");
    assert!(text.contains("rustup default stable"), "{text}");
}

#[test]
fn the_native_toolchain_satisfies_the_toolchain_step_on_each_architecture() {
    let toolchain_line = |arch: Arch, listing: &'static str| -> bool {
        let env = env(arch, false, false);
        let machine = sdk_machine().with_program(
            "rustup",
            path(&[r"C:\Users\dev", ".cargo", "bin", "rustup.exe"]),
        );
        let mut runner = winget_only(runner()).responder(move |spec| {
            (spec.program.ends_with("rustup") || spec.program.ends_with("rustup.exe"))
                .then(|| Outcome::success().with_stdout(listing))
        });
        check(&env, &machine, &mut runner)
            .1
            .contains("[ok     ] base: Rust stable toolchain")
    };
    let x64 = "stable-x86_64-pc-windows-msvc (default)\n";
    let arm = "stable-aarch64-pc-windows-msvc (default)\n";
    assert!(toolchain_line(Arch::X64, x64));
    assert!(!toolchain_line(Arch::X64, arm));
    assert!(toolchain_line(Arch::Arm64, arm));
    assert!(!toolchain_line(Arch::Arm64, x64));
}

#[test]
fn an_apostrophe_in_the_temp_path_is_escaped_in_the_powershell_download() {
    let env = env(Arch::X64, false, false);
    let machine = sdk_machine().with_env("TEMP", r"C:\Users\O'Brien\AppData\Local\Temp");
    let (_, text) = check(&env, &machine, &mut winget_only(runner()));
    let line = text
        .lines()
        .find(|l| l.contains("Invoke-WebRequest"))
        .unwrap_or_else(|| panic!("download line:\n{text}"));
    assert!(
        line.contains("O''Brien"),
        "the apostrophe is doubled: {line}"
    );
    assert!(!line.contains("O'Brien"), "{line}");
}

#[test]
fn a_provisioned_windows_host_has_nothing_to_do() {
    let env = env(Arch::X64, false, false);
    let llvm = path(&[PF, "LLVM", "bin"]);
    let machine = sdk_machine()
        .with_file(vswhere())
        .with_file(llvm.join("clang.exe"))
        .with_program("winget", "winget.exe")
        .with_program(
            "rustup",
            path(&[r"C:\Users\dev", ".cargo", "bin", "rustup.exe"]),
        )
        .with_program(
            "cargo",
            path(&[r"C:\Users\dev", ".cargo", "bin", "cargo.exe"]),
        );
    let mut runner = runner().responder(|spec| match spec.program.as_str() {
        "winget" => Some(Outcome::success()),
        p if p.ends_with("vswhere.exe") => Some(Outcome::success().with_stdout("C:\\VS\n")),
        p if p.ends_with("rustup") || p.ends_with("rustup.exe") => {
            Some(if spec.args.first().is_some_and(|a| a == "show") {
                Outcome::success().with_stdout("Default host: x86_64-pc-windows-msvc\n")
            } else {
                Outcome::success().with_stdout("stable-x86_64-pc-windows-msvc (default)\n")
            })
        }
        _ => None,
    });
    // The LLVM directory is already on the user PATH.
    let mut path = UserPath::Windows(Box::new(MemoryPathStore::new(Some(PathValue::Plain(
        llvm.display().to_string(),
    )))));
    let plan = plan(&env, &[Set::Base]).expect("plan");
    let mut out = Vec::new();
    let mut cx = Cx::new(&env, &mut runner, &machine, &mut path);
    let args = SetupArgs {
        check: true,
        ..SetupArgs::default()
    };
    let code = run_plan(&args, &plan, &mut cx, &mut out);
    let text = String::from_utf8(out).expect("UTF-8");
    assert_eq!(code, 0, "{text}");
}

#[test]
fn a_full_visual_studio_with_the_cpp_tools_counts_even_without_the_build_tools_workload() {
    // A hosted runner has Visual Studio 2022 Enterprise with the C++ desktop workload and a newer
    // instance without it: vswhere must be asked for the compiler component, of any instance.
    let env = env(Arch::X64, false, false);
    let machine = sdk_machine().with_file(vswhere());
    let mut runner = runner().responder(|spec| {
        if !spec.program.ends_with("vswhere.exe") {
            return None;
        }
        let asked = spec.args.join(" ");
        let has_compilers = asked.contains("Microsoft.VisualStudio.Component.VC.Tools.x86.x64");
        let wants_build_tools_workload = asked.contains("Workload.VCTools");
        Some(
            if has_compilers && !wants_build_tools_workload && !asked.contains("-latest") {
                Outcome::success().with_stdout("C:\\VS2022-Enterprise\n")
            } else {
                Outcome::success()
            },
        )
    });

    let (_, text) = check(&env, &machine, &mut runner);

    assert!(
        text.contains("[ok     ] base: Visual Studio 2022 Build Tools (C++ workload)"),
        "{text}"
    );
}
