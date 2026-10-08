//! The base set on Windows x64 and ARM64, through winget: Visual Studio Build Tools, the Windows
//! SDK's `fxc.exe`, LLVM, the VC++ redistributables and a rustup whose host triple is the native
//! one, never the emulated x64 one.

use std::path::{Path, PathBuf};

use crate::host::{Arch, CommandSpec, Privilege};

use super::{Action, Cx, PlanEnv, Probed, Set, SetupError, Step, rust};

const BUILD_TOOLS_ID: &str = "Microsoft.VisualStudio.2022.BuildTools";
const VC_WORKLOAD: &str = "Microsoft.VisualStudio.Workload.VCTools";
/// The C++ compilers and libraries themselves: present in the Build Tools workload and in every
/// full Visual Studio with the C++ desktop workload, so the probe asks for the component.
const VC_TOOLS_COMPONENT: &str = "Microsoft.VisualStudio.Component.VC.Tools.x86.x64";
const VC_ARM64_COMPONENT: &str = "Microsoft.VisualStudio.Component.VC.Tools.ARM64";
const LLVM_ID: &str = "LLVM.LLVM";
const WINGET_HELP: &str = "winget is not available. Install \"App Installer\" from the Microsoft \
    Store (https://aka.ms/getwinget) or update Windows, then run `rayx setup` again.";

/// The native Rust host triple for an architecture.
pub fn native_triple(arch: Arch) -> &'static str {
    match arch {
        Arch::Arm64 => "aarch64-pc-windows-msvc",
        _ => "x86_64-pc-windows-msvc",
    }
}

fn program_files(cx: &Cx) -> PathBuf {
    PathBuf::from(
        cx.machine
            .env("ProgramFiles")
            .unwrap_or_else(|| r"C:\Program Files".to_string()),
    )
}

fn program_files_x86(cx: &Cx) -> PathBuf {
    PathBuf::from(
        cx.machine
            .env("ProgramFiles(x86)")
            .unwrap_or_else(|| r"C:\Program Files (x86)".to_string()),
    )
}

/// `winget install --id <id> --exact`, accepting agreements only with `--yes`.
fn winget_install(cx: &Cx, id: &str) -> CommandSpec {
    let mut spec = CommandSpec::new("winget").args(["install", "--id", id, "--exact"]);
    if cx.env.yes {
        spec = spec.args(["--accept-package-agreements", "--accept-source-agreements"]);
    }
    spec.interactive()
}

fn winget_step() -> Step {
    Step::new(
        "winget",
        Set::Base,
        "winget (App Installer)",
        Privilege::None,
        |cx| {
            let found = cx
                .query(&CommandSpec::new("winget").arg("--version"))
                .is_some_and(|outcome| outcome.is_success());
            if found {
                Probed::ok()
            } else {
                Probed::missing("winget not found")
            }
        },
        |_, _| Err(SetupError::Prerequisite(WINGET_HELP.to_string())),
    )
    .with_fix_hint(WINGET_HELP)
}

fn vswhere(cx: &Cx) -> PathBuf {
    program_files_x86(cx)
        .join("Microsoft Visual Studio")
        .join("Installer")
        .join("vswhere.exe")
}

/// The install path of a Visual Studio with all `components`, if any.
fn find_vs(cx: &mut Cx, components: &[&str]) -> Option<String> {
    let program = vswhere(cx);
    if !cx.machine.exists(&program) {
        return None;
    }
    // Without a component filter the newest instance is wanted (the one to modify); with one,
    // any instance that has the components will do, even when a newer one (Visual Studio 2026
    // next to 2022 on a hosted runner) lacks them.
    let mut spec = CommandSpec::new(program.display().to_string());
    if components.is_empty() {
        spec = spec.args(["-latest"]);
    }
    spec = spec
        .args(["-products", "*"])
        .args(["-property", "installationPath"]);
    if !components.is_empty() {
        spec = spec.arg("-requires").args(components.iter().copied());
    }
    let outcome = cx.query(&spec)?;
    let path = outcome.stdout.lines().next()?.trim().to_string();
    (outcome.is_success() && !path.is_empty()).then_some(path)
}

/// What the probe asks vswhere for.
fn probed_components(arch: Arch) -> Vec<&'static str> {
    let mut components = vec![VC_TOOLS_COMPONENT];
    if arch == Arch::Arm64 {
        components.push(VC_ARM64_COMPONENT);
    }
    components
}

/// What an install or modify adds.
fn required_components(arch: Arch) -> Vec<&'static str> {
    let mut components = vec![VC_WORKLOAD];
    if arch == Arch::Arm64 {
        components.push(VC_ARM64_COMPONENT);
    }
    components
}

fn build_tools_step(env: &PlanEnv) -> Step {
    let arch = env.host.arch;
    Step::new(
        "build-tools",
        Set::Base,
        if arch == Arch::Arm64 {
            "Visual Studio 2022 Build Tools (C++ workload and the ARM64 tools)"
        } else {
            "Visual Studio 2022 Build Tools (C++ workload)"
        },
        Privilege::None,
        move |cx| {
            if find_vs(cx, &probed_components(arch)).is_some() {
                Probed::ok()
            } else {
                Probed::missing("the C++ build tools are not installed")
            }
        },
        move |cx, _| {
            let components = required_components(arch);
            // An existing Visual Studio without the components is modified in place; winget
            // would report the package as already installed.
            if let Some(install_path) = find_vs(cx, &[]) {
                let installer = program_files_x86(cx)
                    .join("Microsoft Visual Studio")
                    .join("Installer")
                    .join("setup.exe");
                let mut modify = CommandSpec::new(installer.display().to_string())
                    .args(["modify", "--installPath", install_path.as_str()])
                    .args(["--passive", "--norestart", "--includeRecommended"]);
                for component in &components {
                    modify = modify.args(["--add", component]);
                }
                return Ok(vec![Action::Run(modify.interactive())]);
            }
            let mut override_args = String::from("--wait --passive --includeRecommended");
            for component in &components {
                override_args.push_str(&format!(" --add {component}"));
            }
            Ok(vec![Action::Run(
                winget_install(cx, BUILD_TOOLS_ID)
                    .arg("--override")
                    .arg(override_args),
            )])
        },
    )
}

fn redist_ids(arch: Arch) -> Vec<&'static str> {
    match arch {
        Arch::Arm64 => vec![
            "Microsoft.VCRedist.2015+.arm64",
            "Microsoft.VCRedist.2015+.x64",
        ],
        _ => vec!["Microsoft.VCRedist.2015+.x64"],
    }
}

fn redist_step(env: &PlanEnv) -> Step {
    let ids = redist_ids(env.host.arch);
    let probe_ids = ids.clone();
    Step::new(
        "vc-redist",
        Set::Base,
        "Microsoft Visual C++ redistributables",
        Privilege::None,
        move |cx| {
            let missing: Vec<String> = probe_ids
                .iter()
                .filter(|id| {
                    !cx.query(&CommandSpec::new("winget").args(["list", "--id", id, "--exact"]))
                        .is_some_and(|outcome| outcome.is_success())
                })
                .map(|id| id.to_string())
                .collect();
            Probed::missing_items(missing)
        },
        move |cx, probed| {
            Ok(probed
                .missing
                .iter()
                .map(|id| Action::Run(winget_install(cx, id)))
                .collect())
        },
    )
}

fn llvm_dir(cx: &Cx) -> PathBuf {
    program_files(cx).join("LLVM").join("bin")
}

fn llvm_step() -> Step {
    Step::new(
        "llvm",
        Set::Base,
        "LLVM and Clang",
        Privilege::None,
        |cx| {
            if cx.machine.exists(&llvm_dir(cx).join("clang.exe"))
                || cx.machine.which("clang").is_some()
            {
                Probed::ok()
            } else {
                Probed::missing("clang not found")
            }
        },
        |cx, _| Ok(vec![Action::Run(winget_install(cx, LLVM_ID))]),
    )
}

fn llvm_path_step() -> Step {
    Step::new(
        "llvm-path",
        Set::Base,
        "LLVM bin on the user PATH",
        Privilege::None,
        |cx| {
            if cx.machine.which("clang").is_some() {
                return Probed::ok();
            }
            match cx.user_path.add(&llvm_dir(cx), true) {
                Ok(crate::host::path_env::PathChange::AlreadyPresent) => Probed::ok(),
                _ => Probed::missing("not on the user PATH"),
            }
        },
        |cx, _| Ok(vec![Action::AddToPath(llvm_dir(cx))]),
    )
}

/// Compares dotted numeric versions such as `10.0.22621.0`.
fn version_key(name: &str) -> Vec<u64> {
    name.split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

/// The directory holding the newest Windows SDK `fxc.exe` for `arch`, falling back to the other
/// architecture's when the preferred one has none.
pub fn find_fxc_dir(cx: &Cx, arch: Arch) -> Option<PathBuf> {
    let root = program_files_x86(cx)
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let mut versions: Vec<String> = cx
        .machine
        .list_dir(&root)
        .into_iter()
        .filter(|name| name.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .collect();
    versions.sort_by_key(|name| std::cmp::Reverse(version_key(name)));
    let (preferred, fallback) = if arch == Arch::Arm64 {
        ("arm64", "x64")
    } else {
        ("x64", "arm64")
    };
    [preferred, fallback].iter().find_map(|folder| {
        versions.iter().find_map(|version| {
            let dir = root.join(version).join(folder);
            cx.machine.exists(&dir.join("fxc.exe")).then_some(dir)
        })
    })
}

fn fxc_step(env: &PlanEnv) -> Step {
    let arch = env.host.arch;
    Step::new(
        "fxc-path",
        Set::Base,
        "Windows SDK fxc.exe on the user PATH",
        Privilege::None,
        move |cx| {
            let Some(dir) = find_fxc_dir(cx, arch) else {
                return Probed::missing("fxc.exe was not found under Windows Kits");
            };
            match cx.user_path.add(&dir, true) {
                Ok(crate::host::path_env::PathChange::AlreadyPresent) => Probed::ok(),
                _ => Probed::missing(format!("{} is not on the user PATH", dir.display())),
            }
        },
        move |cx, _| {
            let dir = find_fxc_dir(cx, arch).ok_or_else(|| {
                SetupError::Prerequisite(
                    "Could not find fxc.exe under Windows Kits\\10\\bin. Install the Visual Studio \
                     Build Tools with the Windows SDK first."
                        .to_string(),
                )
            })?;
            Ok(vec![Action::AddToPath(dir)])
        },
    )
}

fn temp_dir(cx: &Cx) -> PathBuf {
    cx.machine
        .env("TEMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(r"C:\Windows\Temp").to_path_buf())
}

/// rustup, installed with `rustup-init.exe` for the native triple.
fn rustup_step(env: &PlanEnv) -> Step {
    let triple = native_triple(env.host.arch);
    Step::new(
        "rustup",
        Set::Base,
        format!("rustup ({triple})"),
        Privilege::None,
        |cx| {
            if cx.machine.which("rustup").is_some() || rust::rustup_program(cx) != "rustup" {
                Probed::ok()
            } else {
                Probed::missing("rustup not found")
            }
        },
        move |cx, _| {
            let installer = temp_dir(cx).join("rayx-rustup-init.exe");
            let url = format!("https://static.rust-lang.org/rustup/dist/{triple}/rustup-init.exe");
            Ok(vec![
                Action::Run(
                    CommandSpec::new("powershell")
                        .args(["-NoProfile", "-NonInteractive", "-Command"])
                        .arg(format!(
                            "Invoke-WebRequest -UseBasicParsing -Uri {url} -OutFile '{}'",
                            // Inside a single-quoted PowerShell string an apostrophe is doubled.
                            installer.display().to_string().replace('\'', "''")
                        ))
                        .interactive(),
                ),
                Action::Run(
                    CommandSpec::new(installer.display().to_string())
                        .args(["-y", "--default-host", triple])
                        .args(["--default-toolchain", "none", "--no-modify-path"])
                        .interactive(),
                ),
                Action::RemoveFile(installer),
            ])
        },
    )
}

/// A rustup installed earlier from the emulated x64 installer keeps `x86_64` as its host on ARM64.
fn rustup_host_step(env: &PlanEnv) -> Step {
    let triple = native_triple(env.host.arch);
    Step::new(
        "rustup-host",
        Set::Base,
        format!("rustup default host {triple}"),
        Privilege::None,
        move |cx| {
            let program = rust::rustup_program(cx);
            // `rustup show` would install the active toolchain of a project directory.
            let show = CommandSpec::new(program)
                .arg("show")
                .env("RUSTUP_AUTO_INSTALL", "0");
            let Some(shown) = cx.query(&show) else {
                return Probed::missing("rustup is not installed");
            };
            let host = shown
                .stdout
                .lines()
                .find_map(|line| line.trim().strip_prefix("Default host:"))
                .map(str::trim);
            match host {
                Some(host) if host == triple => Probed::ok(),
                Some(host) => Probed::missing(format!("the default host is {host}")),
                None => Probed::missing("the default host is unknown"),
            }
        },
        move |cx, _| {
            let program = rust::rustup_program(cx);
            Ok(vec![Action::Run(
                CommandSpec::new(program)
                    .args(["set", "default-host", triple])
                    .interactive(),
            )])
        },
    )
}

pub fn steps(env: &PlanEnv) -> Result<Vec<Step>, SetupError> {
    if env.host.arch == Arch::Other {
        return Err(SetupError::Unsupported(
            "`rayx setup` supports Windows on x64 and ARM64 only".to_string(),
        ));
    }
    Ok(vec![
        winget_step(),
        build_tools_step(env),
        redist_step(env),
        llvm_step(),
        llvm_path_step(),
        fxc_step(env),
        rustup_step(env),
        rustup_host_step(env),
        rust::toolchain_step(env),
        rust::cargo_path_step(),
    ])
}
