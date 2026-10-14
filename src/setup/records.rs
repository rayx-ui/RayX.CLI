//! What `rayx` remembers on a machine: `guests.json` on a host that drives guests (the guests it
//! manages, their clones and settings). Records hold names, versions, flags, paths and sizes
//! only, never a secret. A missing or unreadable file is an empty record; an unparsable one is
//! reported and left untouched.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::host::Os;

use super::Machine;

/// The version of the record documents.
pub const VERSION: u32 = 1;

/// The file name of the guest record.
pub const GUESTS_FILE: &str = "guests.json";

/// The directory records live in: `$XDG_CONFIG_HOME/rayx` or `~/.config/rayx` on Unix,
/// `%LOCALAPPDATA%\rayx` on Windows.
pub fn config_dir(machine: &dyn Machine, os: Os) -> Option<PathBuf> {
    let base = if os == Os::Windows {
        machine
            .env("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)?
    } else {
        machine
            .env("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| machine.home().map(|home| home.join(".config")))?
    };
    Some(base.join("rayx"))
}

/// Where `guests.json` is.
pub fn guests_path(machine: &dyn Machine, os: Os) -> Option<PathBuf> {
    config_dir(machine, os).map(|dir| dir.join(GUESTS_FILE))
}

/// The file name of the setup record, kept on every machine `rayx setup` runs on.
pub const SETUP_FILE: &str = "setup.json";

/// A guest kind as the record spells it.
pub const WSL: &str = "wsl";
pub const LIMA: &str = "lima";

/// A clone `--clone` made in a guest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloneRecord {
    /// The host checkout it was copied from.
    pub source: String,
    pub branch: Option<String>,
    /// The Linux directory in the guest.
    pub target: String,
}

/// The settings a Lima VM was created with (or last changed to).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmRecord {
    pub cpus: u32,
    pub memory_gib: u32,
    pub disk_gib: u32,
    pub home_writable: bool,
}

/// One guest `rayx` manages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestRecord {
    /// `wsl` or `lima`.
    pub kind: String,
    pub name: String,
    /// The guest's `VERSION_ID`.
    pub ubuntu: String,
    /// The set flags it was set up with, over every run.
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub clones: Vec<CloneRecord>,
    /// Lima only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vm: Option<VmRecord>,
    /// Replaced by a newer guest and kept until the owner removes it.
    #[serde(default)]
    pub retired: bool,
    /// A Lima VM renamed for a migration whose new VM is not recorded yet: the next run resumes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub migrating: bool,
}

/// The `guests.json` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestsFile {
    pub version: u32,
    #[serde(default)]
    pub guests: Vec<GuestRecord>,
}

impl Default for GuestsFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            guests: Vec::new(),
        }
    }
}

/// What a successful `setup --wsl` or `--lima` run tells the record.
#[derive(Clone, Debug)]
pub struct GuestUpdate {
    pub kind: &'static str,
    pub name: String,
    pub ubuntu: String,
    /// The set flags of this run; `--check` and `--yes` are not part of what a guest was set up
    /// with.
    pub flags: Vec<String>,
    pub clone: Option<CloneRecord>,
    pub vm: Option<VmRecord>,
}

impl GuestsFile {
    pub fn find(&self, kind: &str, name: &str) -> Option<&GuestRecord> {
        self.guests
            .iter()
            .find(|guest| guest.kind == kind && guest.name == name)
    }

    /// Updates the entry of the guest, or adds it: flags are a union, a clone is added once per
    /// target, the VM settings change only when the run says so.
    pub fn upsert(&mut self, update: &GuestUpdate) {
        let index = self
            .guests
            .iter()
            .position(|guest| guest.kind == update.kind && guest.name == update.name);
        let guest = match index {
            Some(index) => &mut self.guests[index],
            None => {
                self.guests.push(GuestRecord {
                    kind: update.kind.to_string(),
                    name: update.name.clone(),
                    ubuntu: update.ubuntu.clone(),
                    flags: Vec::new(),
                    clones: Vec::new(),
                    vm: None,
                    retired: false,
                    migrating: false,
                });
                self.guests.last_mut().expect("just pushed")
            }
        };
        guest.ubuntu = update.ubuntu.clone();
        for flag in &update.flags {
            if matches!(flag.as_str(), "--check" | "--yes") || guest.flags.contains(flag) {
                continue;
            }
            guest.flags.push(flag.clone());
        }
        if let Some(clone) = &update.clone {
            match guest.clones.iter_mut().find(|c| c.target == clone.target) {
                Some(existing) => *existing = clone.clone(),
                None => guest.clones.push(clone.clone()),
            }
        }
        if update.vm.is_some() {
            guest.vm = update.vm;
        }
    }
}

impl GuestsFile {
    /// Renames a guest's entry and retires it (a Lima VM renamed to `rayx-<version>`); adds a
    /// retired entry when the guest was never recorded.
    pub fn rename_and_retire(&mut self, kind: &str, from: &str, to: &str, ubuntu: &str) {
        match self
            .guests
            .iter_mut()
            .find(|guest| guest.kind == kind && guest.name == from)
        {
            Some(guest) => {
                guest.name = to.to_string();
                guest.retired = true;
                guest.migrating = true;
            }
            None => self.guests.push(GuestRecord {
                kind: kind.to_string(),
                name: to.to_string(),
                ubuntu: ubuntu.to_string(),
                flags: Vec::new(),
                clones: Vec::new(),
                vm: None,
                retired: true,
                migrating: true,
            }),
        }
    }

    /// Drops a guest's entry; the entry it was.
    pub fn remove(&mut self, kind: &str, name: &str) -> Option<GuestRecord> {
        let index = self
            .guests
            .iter()
            .position(|guest| guest.kind == kind && guest.name == name)?;
        Some(self.guests.remove(index))
    }

    /// The migration of a renamed VM is finished: its new guest is recorded.
    pub fn finish_migration(&mut self, kind: &str, name: &str) {
        if let Some(guest) = self
            .guests
            .iter_mut()
            .find(|guest| guest.kind == kind && guest.name == name)
        {
            guest.migrating = false;
        }
    }

    /// Marks a guest retired.
    pub fn retire(&mut self, kind: &str, name: &str) {
        if let Some(guest) = self
            .guests
            .iter_mut()
            .find(|guest| guest.kind == kind && guest.name == name)
        {
            guest.retired = true;
        }
    }
}

/// Reads `guests.json` from disk, lets `change` edit it and writes it back; an unparsable file
/// is reported and left untouched.
pub fn update_guests(
    machine: &dyn Machine,
    os: Os,
    change: impl FnOnce(&mut GuestsFile),
) -> Result<(), RecordError> {
    let path = guests_path(machine, os).ok_or(RecordError::NoConfigDir)?;
    let mut guests = read_guests_at(&path)?;
    change(&mut guests);
    write_guests_at(&path, &guests)
}

/// Why a record could not be used.
#[derive(Debug, PartialEq, Eq)]
pub enum RecordError {
    /// The file exists but is not a valid record; it is left as it is.
    Unparsable { path: PathBuf, message: String },
    /// The directory or the file could not be written.
    Write { path: PathBuf, message: String },
    /// There is no home or config directory to put the record in.
    NoConfigDir,
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordError::Unparsable { path, message } => write!(
                f,
                "{} is not a valid guest record ({message}); it was left untouched",
                path.display()
            ),
            RecordError::Write { path, message } => {
                write!(f, "cannot write {}: {message}", path.display())
            }
            RecordError::NoConfigDir => {
                f.write_str("there is no config directory for the guest record")
            }
        }
    }
}

impl std::error::Error for RecordError {}

/// Reads a record from its text; an empty or missing file is an empty record.
pub fn parse_guests(path: &Path, text: Option<&str>) -> Result<GuestsFile, RecordError> {
    let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
        return Ok(GuestsFile::default());
    };
    serde_json::from_str(text).map_err(|error| RecordError::Unparsable {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

/// Reads `guests.json` from disk.
pub fn read_guests_at(path: &Path) -> Result<GuestsFile, RecordError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_guests(path, Some(&text)),
        // Not text at all: as unusable as broken JSON, and as much to be left alone.
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            Err(RecordError::Unparsable {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        }
        Err(_) => Ok(GuestsFile::default()),
    }
}

/// Reads `guests.json` through a machine: what doctor and the guest commands see.
pub fn read_guests(machine: &dyn Machine, os: Os) -> Result<GuestsFile, RecordError> {
    let path = guests_path(machine, os).ok_or(RecordError::NoConfigDir)?;
    parse_guests(&path, machine.read_to_string(&path).as_deref())
}

/// Writes the record with a temporary file and a rename, so a reader never sees half a document.
pub fn write_guests_at(path: &Path, guests: &GuestsFile) -> Result<(), RecordError> {
    write_json_atomic(path, guests)
}

fn write_json_atomic(path: &Path, document: &impl Serialize) -> Result<(), RecordError> {
    let fail = |error: std::io::Error| RecordError::Write {
        path: path.to_path_buf(),
        message: error.to_string(),
    };
    let dir = path.parent().ok_or(RecordError::NoConfigDir)?;
    std::fs::create_dir_all(dir).map_err(fail)?;
    let temporary = path.with_extension("json.tmp");
    let mut text = serde_json::to_string_pretty(document).map_err(|error| RecordError::Write {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    text.push('\n');
    let written = std::fs::File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
        })
        .and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written.map_err(fail)
}

/// Records a guest after a successful setup. Nothing is written in a dry run, and an unparsable
/// record is reported and left untouched.
pub fn record_guest(
    machine: &dyn Machine,
    os: Os,
    update: &GuestUpdate,
    dry_run: bool,
) -> Result<(), RecordError> {
    if dry_run {
        return Ok(());
    }
    let path = guests_path(machine, os).ok_or(RecordError::NoConfigDir)?;
    let mut guests = read_guests_at(&path)?;
    guests.upsert(update);
    write_guests_at(&path, &guests)
}

/// Where `setup.json` is.
pub fn setup_path(machine: &dyn Machine, os: Os) -> Option<PathBuf> {
    config_dir(machine, os).map(|dir| dir.join(SETUP_FILE))
}

/// The `setup.json` document: what `rayx setup` was last asked to keep installed on this machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupFile {
    pub version: u32,
    /// The union of the sets every run installed or checked (`base`, `web`, ...).
    #[serde(default)]
    pub sets: Vec<String>,
    /// The project workspace roots the runs happened in.
    #[serde(default)]
    pub roots: Vec<String>,
}

impl Default for SetupFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            sets: Vec::new(),
            roots: Vec::new(),
        }
    }
}

impl SetupFile {
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// Adds sets and a root once each.
    pub fn merge(&mut self, sets: &[String], root: Option<&str>) {
        for set in sets {
            if !self.sets.contains(set) {
                self.sets.push(set.clone());
            }
        }
        if let Some(root) = root
            && !self.roots.iter().any(|known| known == root)
        {
            self.roots.push(root.to_string());
        }
    }

    pub fn forget_root(&mut self, root: &str) {
        self.roots.retain(|known| known != root);
    }
}

/// Reads `setup.json` through a machine; a missing file is an empty record.
pub fn read_setup(machine: &dyn Machine, os: Os) -> Result<SetupFile, RecordError> {
    let path = setup_path(machine, os).ok_or(RecordError::NoConfigDir)?;
    match machine.read_to_string(&path) {
        Some(text) if !text.trim().is_empty() => {
            serde_json::from_str(&text).map_err(|error| RecordError::Unparsable {
                path,
                message: error.to_string(),
            })
        }
        _ => Ok(SetupFile::default()),
    }
}

fn read_setup_at(path: &Path) -> Result<SetupFile, RecordError> {
    match std::fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => {
            serde_json::from_str(&text).map_err(|error| RecordError::Unparsable {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            Err(RecordError::Unparsable {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        }
        _ => Ok(SetupFile::default()),
    }
}

/// Writes `setup.json` with a temporary file and a rename.
pub fn write_setup(machine: &dyn Machine, os: Os, setup: &SetupFile) -> Result<(), RecordError> {
    let path = setup_path(machine, os).ok_or(RecordError::NoConfigDir)?;
    write_json_atomic(&path, setup)
}

/// Records what a successful `rayx setup` ran: the sets and the project root. Nothing is written
/// in a dry run, and an unparsable record is reported and left untouched.
pub fn record_setup(
    machine: &dyn Machine,
    os: Os,
    sets: &[String],
    root: Option<&str>,
    dry_run: bool,
) -> Result<(), RecordError> {
    if dry_run {
        return Ok(());
    }
    let path = setup_path(machine, os).ok_or(RecordError::NoConfigDir)?;
    let mut setup = read_setup_at(&path)?;
    setup.merge(sets, root);
    write_json_atomic(&path, &setup)
}
