//! The nsh store (design §4.2, D10, D11): one entry per pinned NSH commit (or upstream release)
//! under `$RAYX_HOME/nsh/<id>/`, else `~/.rayx/nsh/<id>/`.
//!
//! An entry is complete once its `receipt.json`, written last, exists; an entry without one is
//! unfinished and the next install resumes it. An install holds `<id>.lock`, so a second rayx
//! waits instead of building the same commit twice.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::setup::Machine;

/// The version of `receipt.json`.
pub const RECEIPT_VERSION: u32 = 1;

/// The file that makes an entry complete.
pub const RECEIPT: &str = "receipt.json";

/// How an entry holds its CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A release's prebuilt distribution: `nsh/bin/nsh` and `nsh/runtime/…`.
    Distribution,
    /// A checkout at the commit with the CLI built inside it: `source/target/release/nsh`.
    Source,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Distribution => "distribution",
            Kind::Source => "source",
        }
    }
}

/// What a complete entry is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub version: u32,
    pub id: String,
    pub kind: Kind,
    /// Where it came from: the archive's URL, or the git URL of a source checkout.
    pub url: String,
    /// The tag, branch or rev the pin named, if any.
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    /// The full commit, when the pin names one.
    pub commit: Option<String>,
    /// The NSH version the CLI reports.
    pub nsh: String,
    /// The CLI, relative to the entry.
    pub cli: String,
    /// When the entry was completed (UTC, RFC 3339).
    pub installed: String,
}

/// One entry of the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    /// `None` while the entry is unfinished.
    pub receipt: Option<Receipt>,
    pub bytes: u64,
}

/// The store directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    /// `$RAYX_HOME/nsh`, else `~/.rayx/nsh`; `None` without a home directory.
    pub fn locate(machine: &dyn Machine) -> Option<Store> {
        let home = machine
            .env("RAYX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| machine.home().map(|home| home.join(".rayx")))?;
        Some(Store::at(home.join("nsh")))
    }

    pub fn at(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    /// The directory of an entry.
    pub fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// The receipt of a complete entry; `None` for an unfinished or missing one (or one whose
    /// receipt cannot be read, which the next install redoes).
    pub fn receipt(&self, id: &str) -> Option<Receipt> {
        let text = std::fs::read_to_string(self.dir(id).join(RECEIPT)).ok()?;
        serde_json::from_str(&text)
            .ok()
            .filter(|receipt: &Receipt| receipt.version == RECEIPT_VERSION && receipt.id == id)
    }

    /// The CLI of a complete entry.
    pub fn cli(&self, receipt: &Receipt) -> PathBuf {
        receipt
            .cli
            .split('/')
            .fold(self.dir(&receipt.id), |path, part| path.join(part))
    }

    /// Writes the receipt that completes an entry: a temporary file renamed into place, so a
    /// reader sees a whole receipt or none.
    pub fn complete(&self, receipt: &Receipt) -> Result<()> {
        let dir = self.dir(&receipt.id);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(RECEIPT);
        let temporary = dir.join(format!("{RECEIPT}.{}.tmp", std::process::id()));
        let mut text = serde_json::to_string_pretty(receipt)?;
        text.push('\n');
        let written = File::create(&temporary)
            .and_then(|mut file| {
                file.write_all(text.as_bytes())
                    .and_then(|()| file.sync_all())
            })
            .and_then(|()| std::fs::rename(&temporary, &path));
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written.with_context(|| format!("writing {}", path.display()))
    }

    /// Makes a complete entry unfinished again: its receipt goes, the rest stays for the next
    /// install to reuse or replace.
    pub fn unfinish(&self, id: &str) -> Result<()> {
        let path = self.dir(id).join(RECEIPT);
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(error).with_context(|| format!("removing {}", path.display()))
            }
            _ => Ok(()),
        }
    }

    fn lock_file(&self, id: &str) -> Result<(File, PathBuf)> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("creating {}", self.root.display()))?;
        let path = self.root.join(format!("{id}.lock"));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        Ok((file, path))
    }

    /// Takes `<id>.lock` for an install, waiting (and saying so on `out`) while another rayx
    /// holds it. The lock is released when the guard drops.
    pub fn lock(&self, id: &str, out: &mut dyn Write) -> Result<StoreLock> {
        let (file, path) = self.lock_file(id)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let _ = writeln!(
                    out,
                    "rayx: another rayx is installing nsh {id}; waiting for it to finish"
                );
                file.lock()
                    .with_context(|| format!("locking {}", path.display()))?;
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| format!("locking {}", path.display()));
            }
        }
        Ok(StoreLock { _file: file })
    }

    /// Takes `<id>.lock` if no other rayx holds it; `None` while one is installing that entry.
    pub fn try_lock(&self, id: &str) -> Result<Option<StoreLock>> {
        let (file, path) = self.lock_file(id)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(StoreLock { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => {
                Err(error).with_context(|| format!("locking {}", path.display()))
            }
        }
    }

    /// Every entry, complete or not, sorted by id. Lock files and stray files are not entries.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .map(|id| Entry {
                receipt: self.receipt(&id),
                bytes: size_of(&self.dir(&id)),
                id,
            })
            .collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries
    }

    /// Deletes an entry; the caller holds its lock. The lock file stays: an install waiting on
    /// it must find the same file, or two installs could hold two locks of one id.
    pub fn remove(&self, id: &str) -> Result<()> {
        let dir = self.dir(id);
        std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))
    }
}

/// An install's hold on `<id>.lock`.
#[derive(Debug)]
pub struct StoreLock {
    _file: File,
}

/// The size of a directory tree in bytes; symbolic links count as themselves.
pub fn size_of(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| size_of(&entry.path()))
        .sum()
}

/// `2026-10-24T03:30:00Z` for `time`.
pub fn rfc3339(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let (days, rest) = ((seconds / 86_400) as i64, seconds % 86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// `3.1 GB` style sizes, as `rayx nsh status`, prune and doctor print them.
pub fn human(bytes: u64) -> String {
    crate::doctor::format_bytes(bytes)
}

/// What prune would remove: the entries no kept id names.
pub fn prune_candidates(store: &Store, keep: &[String]) -> Vec<Entry> {
    store
        .entries()
        .into_iter()
        .filter(|entry| !keep.contains(&entry.id))
        .collect()
}

/// Lists what prune would remove on `out` and removes it after `confirm` (or `yes`); returns the
/// exit code. Without an answer (`confirm` is `None`: no terminal) and without `yes`, nothing is
/// removed and the code is 1.
pub fn prune(
    store: &Store,
    keep: &[String],
    yes: bool,
    confirm: Option<&mut dyn FnMut(&str) -> bool>,
    out: &mut dyn Write,
) -> u8 {
    // Each candidate's lock is held until it is removed, so no install starts on it meanwhile;
    // an entry another rayx is installing is kept.
    let mut candidates = Vec::new();
    for entry in prune_candidates(store, keep) {
        match store.try_lock(&entry.id) {
            Ok(Some(lock)) => candidates.push((entry, lock)),
            Ok(None) => {
                let _ = writeln!(
                    out,
                    "rayx: another rayx is installing nsh {}; it is kept",
                    entry.id
                );
            }
            Err(error) => {
                let _ = writeln!(out, "rayx: {error:#}; nsh {} is kept", entry.id);
            }
        }
    }
    if candidates.is_empty() {
        let _ = writeln!(out, "Nothing to prune in {}.", store.root.display());
        return 0;
    }
    let _ = writeln!(
        out,
        "nsh builds no recorded project and no default pin uses, in {}:",
        store.root.display()
    );
    for (entry, _) in &candidates {
        let what = match &entry.receipt {
            Some(receipt) => format!("{} {}", receipt.kind.as_str(), receipt.nsh),
            None => "unfinished".to_string(),
        };
        let _ = writeln!(out, "  {}  {}  {what}", entry.id, human(entry.bytes));
    }
    let total: u64 = candidates.iter().map(|(entry, _)| entry.bytes).sum();
    let question = format!("Remove these ({})?", human(total));
    let agreed = if yes {
        true
    } else if let Some(confirm) = confirm {
        confirm(&question)
    } else {
        let _ = writeln!(
            out,
            "rayx: nothing removed: confirm in a terminal, or run `rayx nsh prune --yes`"
        );
        return 1;
    };
    if !agreed {
        let _ = writeln!(out, "Nothing removed.");
        return 0;
    }
    let mut code = 0;
    for (entry, _lock) in &candidates {
        match store.remove(&entry.id) {
            Ok(()) => {
                let _ = writeln!(out, "Removed {}.", entry.id);
            }
            Err(error) => {
                let _ = writeln!(out, "rayx: {error:#}");
                code = 1;
            }
        }
    }
    code
}
