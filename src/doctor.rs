//! `rayx doctor`: every requirement of every set that applies to this host, each with its status
//! and the exact command that fixes it, plus environment findings. It reads the same step table
//! as `setup` and only probes: it installs nothing and exits 0 whenever it could inspect the host.

use std::io::Write;

use serde_json::{Value, json};

use crate::cli::DoctorArgs;
use crate::host::path_env::UserPath;
use crate::host::{HostFacts, Os, Runner};
use crate::project::Pin;
use crate::setup::gpu::{self, GpuAdapter};
use crate::setup::{
    Cx, Machine, PlanEnv, Set, SystemMachine, check, describe_actions, environment, plan,
};

/// The state of one requirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Present,
    Missing,
    /// Installed, but not at the version the project needs.
    WrongVersion,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Present => "present",
            Status::Missing => "missing",
            Status::WrongVersion => "wrong-version",
        }
    }
}

/// One requirement of a set.
#[derive(Clone, Debug)]
pub struct Requirement {
    pub id: &'static str,
    pub title: String,
    pub status: Status,
    /// The version found, when the probe reads one.
    pub found: Option<String>,
    pub detail: String,
    /// The exact commands that fix it; empty when present.
    pub fix: Vec<String>,
}

/// Environment findings outside the requirement table.
#[derive(Clone, Debug)]
pub struct Findings {
    pub adapters: Vec<GpuAdapter>,
    pub hardware_adapter: bool,
    /// Whether this session can open headed browsers and windows.
    pub interactive_desktop: bool,
    /// WSL findings: PATH order, Vulkan driver files and lavapipe. Empty outside WSL.
    pub wsl: Vec<String>,
    /// On Windows: each WSL distribution with the size of its virtual disk.
    pub wsl_disks: Vec<WslDisk>,
}

/// A WSL distribution's virtual disk, as seen from Windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WslDisk {
    pub distribution: String,
    pub vhdx: std::path::PathBuf,
    /// `None` when the file cannot be read.
    pub bytes: Option<u64>,
}

/// The whole report.
#[derive(Clone, Debug)]
pub struct Report {
    pub host: HostFacts,
    pub project_root: Option<String>,
    pub pins: Vec<(&'static str, Pin)>,
    pub sets: Vec<(Set, Vec<Requirement>)>,
    /// Why requirements could not be listed (an unsupported distribution), if so.
    pub unsupported: Option<String>,
    pub findings: Findings,
}

impl Report {
    pub fn requirement_count(&self) -> usize {
        self.sets.iter().map(|(_, items)| items.len()).sum()
    }

    pub fn missing_count(&self) -> usize {
        self.sets
            .iter()
            .flat_map(|(_, items)| items)
            .filter(|r| r.status != Status::Present)
            .count()
    }

    /// The report as one JSON document.
    pub fn to_json(&self) -> Value {
        let host = &self.host;
        json!({
            "host": {
                "os": host.os.as_str(),
                "arch": host.arch.as_str(),
                "emulated": host.emulated,
                "wsl": host.wsl,
                "distribution": host.distro.as_ref().map(|d| d.name.clone()),
            },
            "project": self.project_root,
            "pins": self.pins.iter().map(|(name, pin)| {
                (name.to_string(), json!({"value": pin.value, "source": pin.source.as_str()}))
            }).collect::<serde_json::Map<String, Value>>(),
            "sets": self.sets.iter().map(|(set, items)| json!({
                "name": set.name(),
                "requirements": items.iter().map(|r| json!({
                    "id": r.id,
                    "title": r.title,
                    "status": r.status.as_str(),
                    "found": r.found,
                    "detail": r.detail,
                    "fix": r.fix,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "unsupported": self.unsupported,
            "requirementCount": self.requirement_count(),
            "missingCount": self.missing_count(),
            "gpuAdapters": self.findings.adapters.iter().map(|a| json!({
                "vendor": a.vendor.name(),
                "name": a.name,
                "driverVersion": a.driver_version,
                "driverDate": a.driver_date,
                "software": a.software,
            })).collect::<Vec<_>>(),
            "hardwareAdapter": self.findings.hardware_adapter,
            "interactiveDesktop": self.findings.interactive_desktop,
            "wsl": self.findings.wsl,
            "wslDisks": self.findings.wsl_disks.iter().map(|disk| json!({
                "distribution": disk.distribution,
                "vhdx": disk.vhdx.display().to_string(),
                "bytes": disk.bytes,
            })).collect::<Vec<_>>(),
        })
    }

    /// The report as text for a terminal.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let host = &self.host;
        out.push_str(&format!(
            "rayx doctor: {} {}{}{}\n",
            host.os.as_str(),
            host.arch.as_str(),
            if host.emulated {
                " (this rayx is emulated)"
            } else {
                ""
            },
            if host.wsl { ", inside WSL" } else { "" },
        ));
        out.push_str(&format!(
            "Project: {}\n",
            self.project_root
                .as_deref()
                .unwrap_or("none (built-in defaults apply)")
        ));
        out.push_str("\nPins:\n");
        for (name, pin) in &self.pins {
            out.push_str(&format!("  {name}: {} ({})\n", pin.value, pin.source));
        }
        if let Some(reason) = &self.unsupported {
            out.push_str(&format!("\nRequirements are not listed: {reason}\n"));
        }
        for (set, items) in &self.sets {
            out.push_str(&format!("\n[{set}]\n"));
            for item in items {
                let label = match item.status {
                    Status::Present => "present      ",
                    Status::Missing => "missing      ",
                    Status::WrongVersion => "wrong version",
                };
                let mut line = format!("  {label} {}", item.title);
                if let Some(found) = &item.found {
                    line.push_str(&format!(" (found {found})"));
                }
                if !item.detail.is_empty() {
                    line.push_str(&format!(" ({})", item.detail));
                }
                out.push_str(&line);
                out.push('\n');
                for command in &item.fix {
                    out.push_str(&format!("      fix: {command}\n"));
                }
            }
        }
        out.push_str("\nEnvironment:\n");
        if self.findings.adapters.is_empty() {
            out.push_str("  GPU adapters: none found\n");
        }
        for adapter in &self.findings.adapters {
            out.push_str(&format!(
                "  GPU: {} {}{}{}\n",
                adapter.vendor.name(),
                adapter.name,
                adapter
                    .driver_version
                    .as_deref()
                    .map_or(String::new(), |v| format!(", driver {v}")),
                if adapter.software { " (software)" } else { "" },
            ));
        }
        out.push_str(&format!(
            "  Hardware adapter usable: {}\n",
            yes_no(self.findings.hardware_adapter)
        ));
        out.push_str(&format!(
            "  Interactive desktop session: {}\n",
            yes_no(self.findings.interactive_desktop)
        ));
        for finding in &self.findings.wsl {
            out.push_str(&format!("  WSL: {finding}\n"));
        }
        for disk in &self.findings.wsl_disks {
            out.push_str(&format!(
                "  WSL disk: {} {} ({})\n",
                disk.distribution,
                disk.bytes.map_or("size unknown".to_string(), format_bytes),
                disk.vhdx.display()
            ));
        }
        out.push_str(&format!(
            "\n{} requirement(s), {} missing or at the wrong version.\n",
            self.requirement_count(),
            self.missing_count()
        ));
        out
    }
}

/// `12.3 GiB` style sizes.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// The pins the report shows, in a fixed order.
fn pin_list(env: &PlanEnv) -> Vec<(&'static str, Pin)> {
    let p = &env.pins;
    let mut pins: Vec<(&'static str, Pin)> = Vec::new();
    let optional = |name: &'static str, pin: &Option<Pin>, pins: &mut Vec<(&'static str, Pin)>| {
        if let Some(pin) = pin {
            pins.push((name, pin.clone()));
        }
    };
    optional("rust-toolchain", &p.rust_toolchain, &mut pins);
    optional("wasm-bindgen", &p.wasm_bindgen, &mut pins);
    pins.push(("web-toolchain", p.web_toolchain.clone()));
    pins.push(("node", p.node.clone()));
    pins.push(("jdk", p.jdk.clone()));
    pins.push(("playwright", p.playwright.clone()));
    pins.push(("android-ndk", p.android_ndk.clone()));
    pins.push(("android-platform", p.android_platform.clone()));
    optional("android-build-tools", &p.android_build_tools, &mut pins);
    pins.push(("android-ndk-api", p.android_ndk_api.clone()));
    pins.push(("android-system-image", p.android_system_image.clone()));
    optional("xcode", &p.xcode, &mut pins);
    pins
}

/// Whether this session can open headed browsers and windows.
fn interactive_desktop(cx: &mut Cx) -> bool {
    match cx.env.host.os {
        Os::Windows => cx.machine.console_session().unwrap_or(false),
        Os::Linux => ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|name| cx.machine.env(name).is_some_and(|v| !v.is_empty())),
        Os::MacOs => cx
            .query(&crate::host::CommandSpec::new("launchctl").arg("managername"))
            .is_some_and(|outcome| outcome.is_success() && outcome.stdout.trim() == "Aqua"),
    }
}

/// WSL-specific findings: Windows binaries ahead of Linux ones, and the Vulkan driver setup.
fn wsl_findings(cx: &mut Cx, hardware_adapter: bool) -> Vec<String> {
    let mut findings = Vec::new();
    for tool in ["pnpm", "wasm-bindgen", "cargo", "node"] {
        if let Some(found) = cx
            .machine
            .which(tool)
            .filter(|path| path.starts_with("/mnt"))
        {
            findings.push(format!(
                "`{tool}` resolves to {} (a Windows program); put ~/.local/bin and ~/.cargo/bin ahead of /mnt/c in PATH",
                found.display()
            ));
        }
    }
    if let Some(path) = cx.machine.env("PATH") {
        let entries: Vec<&str> = path.split(':').collect();
        let first_windows = entries.iter().position(|e| e.starts_with("/mnt/"));
        let user_bin = entries.iter().position(|e| e.ends_with("/.local/bin"));
        if let Some(windows) = first_windows
            && user_bin.is_none_or(|user| user > windows)
        {
            findings.push(
                "PATH lists /mnt/c entries before ~/.local/bin, so a Windows pnpm can win"
                    .to_string(),
            );
        }
    }
    let driver_files = cx
        .machine
        .env("VK_DRIVER_FILES")
        .or_else(|| cx.machine.env("VK_ICD_FILENAMES"));
    findings.push(match &driver_files {
        Some(files) => format!("VK_DRIVER_FILES = {files}"),
        None => "VK_DRIVER_FILES is not set".to_string(),
    });
    let icd_dir = std::path::Path::new("/usr/share/vulkan/icd.d");
    let lavapipe = cx
        .machine
        .list_dir(icd_dir)
        .iter()
        .any(|name| name.starts_with("lvp_icd"));
    findings.push(format!(
        "lavapipe (software Vulkan fallback): {}{}",
        if lavapipe {
            "installed"
        } else {
            "not installed"
        },
        if hardware_adapter {
            ""
        } else {
            "; no hardware adapter is visible from WSL"
        },
    ));
    findings
}

/// Probes every requirement of every set that applies to `env`'s host. Nothing is installed.
pub fn inspect(env: &PlanEnv, project_root: Option<String>, cx: &mut Cx) -> Report {
    let sets: Vec<Set> = Set::ALL
        .into_iter()
        .filter(|set| set.applies_to(&env.host))
        .collect();
    let mut report_sets: Vec<(Set, Vec<Requirement>)> =
        sets.iter().map(|s| (*s, Vec::new())).collect();
    let mut unsupported = None;

    match plan(env, &sets) {
        Ok(plan) => {
            let checked = check(&plan, cx);
            for (step, probed) in plan.steps.iter().zip(&checked.probed) {
                let status = if probed.satisfied {
                    Status::Present
                } else if probed.found.is_some() {
                    Status::WrongVersion
                } else {
                    Status::Missing
                };
                let fix = if probed.satisfied {
                    Vec::new()
                } else {
                    describe_actions(step, probed, &probed.missing, cx)
                };
                let requirement = Requirement {
                    id: step.id,
                    title: step.title.clone(),
                    status,
                    found: probed.found.clone(),
                    detail: probed.detail.clone(),
                    fix,
                };
                if let Some((_, items)) = report_sets.iter_mut().find(|(set, _)| *set == step.set) {
                    items.push(requirement);
                }
            }
        }
        // An unsupported distribution still gets the environment report.
        Err(error) => unsupported = Some(error.to_string()),
    }

    let adapters = gpu::detect_adapters(cx);
    let hardware_adapter = gpu::hardware_adapter_usable(cx, &adapters);
    let interactive = interactive_desktop(cx);
    let wsl = if env.host.wsl {
        wsl_findings(cx, hardware_adapter)
    } else {
        Vec::new()
    };
    // From Windows, the WSL virtual disks: the size WSL keeps even after files are deleted.
    let wsl_disks = if env.host.os == Os::Windows {
        cx.machine
            .wsl_distributions()
            .into_iter()
            .map(|distribution| {
                let vhdx = distribution.vhdx();
                WslDisk {
                    distribution: distribution.name,
                    bytes: cx.machine.file_size(&vhdx),
                    vhdx,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Report {
        host: env.host.clone(),
        project_root,
        pins: pin_list(env),
        sets: report_sets,
        unsupported,
        findings: Findings {
            adapters,
            hardware_adapter,
            interactive_desktop: interactive,
            wsl,
            wsl_disks,
        },
    }
}

/// Runs `rayx doctor` against the real machine and returns the exit code: 0 whenever the host
/// could be inspected.
pub fn run(args: &DoctorArgs) -> u8 {
    let host = crate::host::facts();
    let (env, project) = environment(host.clone(), false);
    let mut runner = Runner::print();
    let machine = SystemMachine;
    let mut user_path = match UserPath::system(host.os) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("rayx: cannot read the user PATH: {error}");
            return 1;
        }
    };
    let report = report_for(
        &env,
        &mut runner,
        &machine,
        &mut user_path,
        project.map(|p| p.workspace_root.display().to_string()),
    );
    let stdout = &mut std::io::stdout();
    let text = if args.json {
        serde_json::to_string_pretty(&report.to_json()).unwrap_or_default() + "\n"
    } else {
        report.to_text()
    };
    // A closed pipe leaves nothing more to report.
    let _ = stdout.write_all(text.as_bytes());
    0
}

/// Builds the report over injectable collaborators.
pub fn report_for(
    env: &PlanEnv,
    runner: &mut Runner,
    machine: &dyn Machine,
    user_path: &mut UserPath,
    project_root: Option<String>,
) -> Report {
    let mut cx = Cx::new(env, runner, machine, user_path);
    inspect(env, project_root, &mut cx)
}
