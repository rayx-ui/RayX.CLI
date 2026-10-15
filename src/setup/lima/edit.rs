//! Explicit `--vm-*` flags on an existing VM `rayx`: the differences become one update, asked
//! about like any outdated requirement and applied as stop (when running), `limactl edit`, start.
//! A re-run without flags never reaches this module.

use std::path::Path;

use crate::host::CommandSpec;
use crate::setup::Machine;

use super::{Instance, LimaError, VmFlags, VmName};

/// What explicit flags change in an existing VM.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VmEdit {
    pub cpus: Option<(u32, u32)>,
    pub memory_gib: Option<(u64, u32)>,
    pub disk_gib: Option<(u64, u32)>,
    /// The home becomes writable.
    pub home_writable: bool,
}

impl VmEdit {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The changes as the owner reads them: `4 -> 6 CPUs, 12 -> 8 GiB memory`.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some((from, to)) = self.cpus {
            parts.push(format!("{from} -> {to} CPUs"));
        }
        if let Some((from, to)) = self.memory_gib {
            parts.push(format!("{from} -> {to} GiB memory"));
        }
        if let Some((from, to)) = self.disk_gib {
            parts.push(format!("{from} -> {to} GiB disk"));
        }
        if self.home_writable {
            parts.push("home read-only -> writable".to_string());
        }
        parts.join(", ")
    }

    /// The `limactl edit` command with only the changed flags.
    pub fn command(&self, name: &VmName, home: &Path) -> CommandSpec {
        let mut spec = CommandSpec::new("limactl").args(["edit", "--tty=false", name.as_str()]);
        if let Some((_, to)) = self.cpus {
            spec = spec.arg(format!("--cpus={to}"));
        }
        if let Some((_, to)) = self.memory_gib {
            spec = spec.arg(format!("--memory={to}"));
        }
        if let Some((_, to)) = self.disk_gib {
            spec = spec.arg(format!("--disk={to}"));
        }
        if self.home_writable {
            spec = spec.arg(format!("--mount-only={}:w", home.display()));
        }
        spec.interactive()
    }
}

/// Whether the macOS home is mounted writable according to an instance's `lima.yaml`: the mount
/// whose location is `~` or the home path; `writable` absent means read-only.
pub fn home_is_writable(lima_yaml: &str, home: &Path) -> bool {
    let Ok(document) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(lima_yaml) else {
        return false;
    };
    let home = home.to_string_lossy();
    let home = home.trim_end_matches('/');
    document
        .get("mounts")
        .and_then(serde_yaml_ng::Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|mount| {
            mount
                .get("location")
                .and_then(serde_yaml_ng::Value::as_str)
                .is_some_and(|location| {
                    let location = location.trim_end_matches('/');
                    location == "~" || location == home
                })
        })
        .any(|mount| {
            mount
                .get("writable")
                .and_then(serde_yaml_ng::Value::as_bool)
                .unwrap_or(false)
        })
}

/// What the explicit `flags` change in `instance`; a smaller disk is refused because Lima cannot
/// shrink one. Flags equal to the VM change nothing.
pub fn plan_edit(
    name: &VmName,
    instance: &Instance,
    flags: &VmFlags,
    machine: &dyn Machine,
    home: &Path,
) -> Result<VmEdit, LimaError> {
    let mut edit = VmEdit::default();
    if let Some(cpus) = flags.cpus.filter(|cpus| *cpus != instance.cpus) {
        edit.cpus = Some((instance.cpus, cpus));
    }
    if let Some(memory) = flags
        .memory_gib
        .filter(|memory| u64::from(*memory) != instance.memory_gib())
    {
        edit.memory_gib = Some((instance.memory_gib(), memory));
    }
    if let Some(disk) = flags.disk_gib {
        match u64::from(disk).cmp(&instance.disk_gib()) {
            std::cmp::Ordering::Less => {
                return Err(LimaError::Failed(format!(
                    "--vm-disk {disk} is smaller than the {} GiB disk of {name}: Lima cannot \
                     shrink a disk",
                    instance.disk_gib()
                )));
            }
            std::cmp::Ordering::Greater => edit.disk_gib = Some((instance.disk_gib(), disk)),
            std::cmp::Ordering::Equal => {}
        }
    }
    if flags.home_writable {
        let yaml = machine
            .read_to_string(&Path::new(&instance.dir).join("lima.yaml"))
            .unwrap_or_default();
        edit.home_writable = !home_is_writable(&yaml, home);
    }
    Ok(edit)
}
