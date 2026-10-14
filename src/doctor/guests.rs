//! The guests `rayx` manages, as `rayx doctor` reports them: the Lima VMs on macOS and the WSL
//! distributions on Windows, joined from `guests.json` and the host's own view of them. Nothing
//! here starts, stops or changes a guest.

use serde_json::{Value, json};

use crate::host::{CommandSpec, Os};
use crate::setup::Cx;
use crate::setup::lima::{self, edit::home_is_writable, parse_instances, parse_lima_version};
use crate::setup::records::{self, GuestRecord};

use super::format_bytes;

/// Lima on this Mac.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimaInfo {
    /// `None` when `limactl` is missing or does not run.
    pub version: Option<String>,
}

/// One recorded guest joined with what the host reports about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestReport {
    /// `lima` or `wsl`.
    pub kind: String,
    pub name: String,
    /// `running`, `stopped` or `missing`.
    pub status: String,
    pub cpus: Option<u32>,
    pub memory_gib: Option<u64>,
    /// A Lima disk in GiB.
    pub disk_gib: Option<u64>,
    /// A WSL virtual disk file in bytes.
    pub disk_bytes: Option<u64>,
    /// `read-only` or `writable` (Lima).
    pub home: Option<&'static str>,
    pub ubuntu: String,
    /// `current`, `outdated` (older than the pin, not replaced yet) or `retired`.
    pub state: &'static str,
    /// The command that removes a retired or missing guest.
    pub remove: Option<String>,
}

/// The guests section of the report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuestsReport {
    /// On macOS only.
    pub lima: Option<LimaInfo>,
    pub guests: Vec<GuestReport>,
    /// Why `guests.json` could not be used.
    pub problem: Option<String>,
}

impl GuestsReport {
    pub fn is_empty(&self) -> bool {
        self.lima.is_none() && self.guests.is_empty() && self.problem.is_none()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "lima": self.lima.as_ref().map(|lima| json!({
                "installed": lima.version.is_some(),
                "version": lima.version,
            })),
            "guests": self.guests.iter().map(|guest| json!({
                "kind": guest.kind,
                "name": guest.name,
                "status": guest.status,
                "cpus": guest.cpus,
                "memoryGib": guest.memory_gib,
                "diskGib": guest.disk_gib,
                "diskBytes": guest.disk_bytes,
                "home": guest.home,
                "ubuntu": guest.ubuntu,
                "state": guest.state,
                "current": guest.state == "current",
                "retired": guest.state == "retired",
                "remove": guest.remove,
            })).collect::<Vec<_>>(),
            "problem": self.problem,
        })
    }

    /// The section for a terminal; empty off macOS and Windows.
    pub fn to_text(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut out = String::from("\nGuests:\n");
        if let Some(lima) = &self.lima {
            match &lima.version {
                Some(version) => out.push_str(&format!("  Lima: limactl {version}\n")),
                None => out.push_str("  Lima: missing (rayx setup --lima installs it)\n"),
            }
        }
        if let Some(problem) = &self.problem {
            out.push_str(&format!("  {problem}\n"));
        }
        for guest in &self.guests {
            let mut parts = vec![guest.status.clone()];
            if let Some(cpus) = guest.cpus {
                parts.push(format!("{cpus} CPUs"));
            }
            if let Some(memory) = guest.memory_gib {
                parts.push(format!("{memory} GiB memory"));
            }
            if let Some(disk) = guest.disk_gib {
                parts.push(format!("{disk} GiB disk"));
            }
            if let Some(bytes) = guest.disk_bytes {
                parts.push(format!("disk {}", format_bytes(bytes)));
            }
            if let Some(home) = guest.home {
                parts.push(format!("home {home}"));
            }
            parts.push(format!("Ubuntu {}", guest.ubuntu));
            parts.push(guest.state.to_string());
            out.push_str(&format!(
                "  {} {}: {}\n",
                guest.kind,
                guest.name,
                parts.join(", ")
            ));
            if let Some(remove) = &guest.remove {
                out.push_str(&format!("      remove: {remove}\n"));
            }
        }
        if self.guests.is_empty() && self.lima.is_some() {
            out.push_str("  no guests recorded\n");
        }
        out
    }
}

/// Reads the guests `rayx` manages on this host. A host that drives no guests has an empty report.
pub fn inspect(cx: &mut Cx) -> GuestsReport {
    let os = cx.env.host.os;
    if !matches!(os, Os::MacOs | Os::Windows) || cx.env.host.wsl {
        return GuestsReport::default();
    }
    let mut report = GuestsReport::default();
    let recorded = match records::read_guests(cx.machine, os) {
        Ok(file) => file.guests,
        Err(error) => {
            report.problem = Some(error.to_string());
            Vec::new()
        }
    };
    let pin = cx.env.pins.ubuntu.value.clone();
    match os {
        Os::MacOs => inspect_lima(cx, &recorded, &pin, &mut report),
        _ => inspect_wsl(cx, &recorded, &pin, &mut report),
    }
    report
}

fn state(record: &GuestRecord, pin: &str) -> &'static str {
    if record.retired {
        "retired"
    } else if record.ubuntu == pin {
        "current"
    } else {
        "outdated"
    }
}

fn remove_hint(record: &GuestRecord, status: &str, command: &str) -> Option<String> {
    (record.retired || status == "missing").then(|| format!("{command} {}", record.name))
}

fn inspect_lima(cx: &mut Cx, recorded: &[GuestRecord], pin: &str, report: &mut GuestsReport) {
    let path_dir = lima::lima_path_dir(cx.machine);
    let limactl = |args: &[&str]| {
        let spec = CommandSpec::new("limactl").args(args.iter().copied());
        match &path_dir {
            Some(dir) => spec.env("PATH", crate::setup::guest::path_with(dir)),
            None => spec,
        }
    };
    let version = cx
        .query(&limactl(&["--version"]))
        .filter(|outcome| outcome.is_success())
        .and_then(|outcome| parse_lima_version(&outcome.stdout));
    let installed = version.is_some();
    report.lima = Some(LimaInfo { version });
    // A `limactl list` that fails says nothing about the guests: they are `unknown`, not missing.
    let mut list_failed = false;
    let instances = if installed {
        match cx
            .query(&limactl(&["list", "--json"]))
            .filter(|outcome| outcome.is_success())
        {
            Some(outcome) => parse_instances(&outcome.stdout),
            None => {
                list_failed = true;
                let why = "cannot read Lima: `limactl list --json` failed".to_string();
                report.problem = Some(match report.problem.take() {
                    Some(problem) => format!("{problem}; {why}"),
                    None => why,
                });
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let home = cx.machine.home();
    for record in recorded.iter().filter(|g| g.kind == records::LIMA) {
        let instance = instances.iter().find(|i| i.name == record.name);
        let status = match instance {
            Some(i) if i.is_running() => "running",
            Some(_) => "stopped",
            None if list_failed => "unknown",
            None => "missing",
        };
        let writable = match (instance, &home) {
            (Some(i), Some(home)) => cx
                .machine
                .read_to_string(&std::path::Path::new(&i.dir).join("lima.yaml"))
                .map(|yaml| home_is_writable(&yaml, home)),
            _ => None,
        };
        report.guests.push(GuestReport {
            kind: records::LIMA.to_string(),
            name: record.name.clone(),
            status: status.to_string(),
            cpus: instance.map(|i| i.cpus),
            memory_gib: instance.map(|i| i.memory_gib()),
            disk_gib: instance.map(|i| i.disk_gib()),
            disk_bytes: None,
            home: writable.map(|w| if w { "writable" } else { "read-only" }),
            ubuntu: record.ubuntu.clone(),
            state: state(record, pin),
            remove: remove_hint(record, status, "rayx lima remove"),
        });
    }
}

fn inspect_wsl(cx: &mut Cx, recorded: &[GuestRecord], pin: &str, report: &mut GuestsReport) {
    let wsl: Vec<&GuestRecord> = recorded.iter().filter(|g| g.kind == records::WSL).collect();
    if wsl.is_empty() {
        return;
    }
    let distributions = cx.machine.wsl_distributions();
    // `wsl.exe` prints UTF-16, which arrives with NUL bytes between letters.
    let running: Vec<String> = cx
        .query(&CommandSpec::new("wsl").args(["--list", "--running", "--quiet"]))
        .filter(|outcome| outcome.is_success())
        .map(|outcome| {
            outcome
                .stdout
                .replace('\0', "")
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .unwrap_or_default();
    for record in wsl {
        let distribution = distributions.iter().find(|d| d.name == record.name);
        let status = match distribution {
            Some(_) if running.contains(&record.name) => "running",
            Some(_) => "stopped",
            None => "missing",
        };
        report.guests.push(GuestReport {
            kind: records::WSL.to_string(),
            name: record.name.clone(),
            status: status.to_string(),
            cpus: None,
            memory_gib: None,
            disk_gib: None,
            disk_bytes: distribution.and_then(|d| cx.machine.file_size(&d.vhdx())),
            home: None,
            ubuntu: record.ubuntu.clone(),
            state: state(record, pin),
            remove: remove_hint(record, status, "rayx wsl remove"),
        });
    }
}
