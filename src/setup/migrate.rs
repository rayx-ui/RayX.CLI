//! Side-by-side Ubuntu migration. A guest older than the pin is never upgraded in place: a new
//! guest is made from the pinned image next to it (Lima: the old VM is renamed `rayx-<old
//! version>`; WSL: `Ubuntu-<pin>` next to the old distribution), its clones and flags are restored
//! from `guests.json`, and the old guest is kept, retired, until the owner removes it. Every step
//! checks the record and the live state first, so a rerun resumes.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::host::{CommandSpec, HostFacts, Runner};

use super::guest::{self, CloneSource, Guest, LinuxBinary, pinned_ubuntu};
use super::lima::edit::home_is_writable;
use super::lima::sizing::Prompter;
use super::lima::{self, INSTANCE, Instance, Tool, VmSettings};
use super::records::{self, CloneRecord, GuestRecord, GuestUpdate, GuestsFile, VmRecord};
use super::refresh::RefreshOptions;
use super::{Machine, wsl};

/// How a migration ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Migration {
    /// The new guest is set up; the old one is kept, retired.
    Done { new: String, old: String },
    /// `--check`: what would happen was printed.
    Planned,
    /// Declined, or nobody could be asked: nothing changed.
    Declined(String),
}

/// Whether an Ubuntu `VERSION_ID` is older than the pin (numeric on `major.minor`).
pub fn older_than_pin(version: &str) -> bool {
    fn key(version: &str) -> Vec<u32> {
        version
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    }
    key(version) < key(pinned_ubuntu())
}

/// Whether an Ubuntu `VERSION_ID` is newer than the pin: reported, never changed.
pub fn newer_than_pin(version: &str) -> bool {
    !older_than_pin(version) && version != pinned_ubuntu()
}

/// Whether the owner agrees to a change: `--yes` does, a missing prompter does (tests), a prompter
/// without a terminal does not.
fn consent(
    options: &RefreshOptions,
    prompter: &mut Option<&mut dyn Prompter>,
    question: &str,
) -> bool {
    if options.yes {
        return true;
    }
    match prompter.as_mut() {
        None => true,
        Some(prompter) => prompter.interactive() && prompter.confirm(question),
    }
}

fn report_dirty(out: &mut dyn Write, old: &str, dirty: &[String]) {
    if dirty.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "These clones in {old} hold changes that exist nowhere else (uncommitted work or commits \
         on no remote); the old guest is kept, so nothing is lost:"
    );
    for dir in dirty {
        let _ = writeln!(out, "  {dir}");
    }
}

fn record_error(error: &records::RecordError) {
    eprintln!("rayx: {error}");
}

/// The flags a migrated guest is set up with: the recorded ones, and `--yes` when it was given.
fn setup_flags(options: &RefreshOptions, entry: Option<&GuestRecord>) -> Vec<String> {
    let mut flags = entry.map(|e| e.flags.clone()).unwrap_or_default();
    if options.yes && !flags.iter().any(|f| f == "--yes") {
        flags.push("--yes".to_string());
    }
    flags
}

/// Re-runs `rayx setup <flags>` inside the new guest, once per restored clone (in it), or once.
fn setup_in_clones(
    guest: &Guest,
    flags: &[String],
    clones: &[String],
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<(), String> {
    if clones.is_empty() {
        return guest::run_setup_inside(guest, flags, None, runner, out);
    }
    for target in clones {
        guest::run_setup_inside(guest, flags, Some(target), runner, out)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Lima

/// The old Lima VM a migration replaces.
#[derive(Clone, Debug)]
pub struct LimaOld {
    /// `rayx` before the rename, `rayx-<old version>` once renamed (a resumed migration).
    pub current_name: String,
    /// The old guest's Ubuntu `VERSION_ID`.
    pub version: String,
    pub entry: Option<GuestRecord>,
    pub instance: Instance,
}

/// The migration an interrupted run left behind: a retired `rayx-<version>` VM in the record and
/// in Lima, and no current `rayx` in the record.
pub fn pending_lima(file: &GuestsFile, instances: &[Instance]) -> Option<LimaOld> {
    let current = file
        .guests
        .iter()
        .any(|g| g.kind == records::LIMA && g.name == INSTANCE && !g.retired);
    if current {
        return None;
    }
    file.guests
        .iter()
        .filter(|g| {
            g.kind == records::LIMA && g.retired && g.migrating && g.name.starts_with("rayx-")
        })
        .find_map(|entry| {
            instances
                .iter()
                .find(|i| i.name == entry.name)
                .map(|instance| LimaOld {
                    current_name: entry.name.clone(),
                    version: entry.ubuntu.clone(),
                    entry: Some(entry.clone()),
                    instance: instance.clone(),
                })
        })
}

/// What a Lima migration needs from the run around it.
pub struct LimaMigration<'a> {
    pub options: &'a RefreshOptions,
    pub host: &'a HostFacts,
    pub machine: &'a dyn Machine,
    pub home: &'a Path,
    pub path_dir: Option<PathBuf>,
}

/// Replaces an old `rayx` VM with a new one from the pinned Ubuntu, side by side. The old VM must
/// be running when the migration starts fresh (its clones are inspected through it).
pub fn migrate_lima(
    m: &LimaMigration,
    runner: &mut Runner,
    prompter: &mut Option<&mut dyn Prompter>,
    out: &mut dyn Write,
    old: &LimaOld,
) -> Result<(Migration, Option<VmRecord>), String> {
    let tool = Tool {
        path_dir: m.path_dir.clone(),
    };
    let resumed = old.current_name != INSTANCE;
    let old_name = if resumed {
        old.current_name.clone()
    } else {
        format!("rayx-{}", old.version)
    };
    let writable = m
        .machine
        .read_to_string(&Path::new(&old.instance.dir).join("lima.yaml"))
        .is_some_and(|yaml| home_is_writable(&yaml, m.home));
    let settings = VmSettings {
        cpus: old.instance.cpus,
        memory_gib: u32::try_from(old.instance.memory_gib()).unwrap_or(u32::MAX),
        disk_gib: u32::try_from(old.instance.disk_gib()).unwrap_or(u32::MAX),
        home_writable: writable,
    };

    if !resumed {
        let old_guest = Guest::lima(INSTANCE).with_path_dir(m.path_dir.clone());
        let dirty = guest::dirty_clones(&old_guest, runner);
        let question = format!(
            "Create {INSTANCE} with Ubuntu {} next to {old_name}?",
            pinned_ubuntu()
        );
        let _ = writeln!(
            out,
            "{INSTANCE} runs Ubuntu {}, older than {}: it is never upgraded in place. The old VM \
             is kept, retired, as {old_name} until you remove it: rayx lima remove {old_name}",
            old.version,
            pinned_ubuntu()
        );
        report_dirty(out, &old_name, &dirty);
        if m.options.check {
            let _ = writeln!(
                out,
                "{question}\nwould run: limactl stop {INSTANCE}; limactl rename {INSTANCE} \
                 {old_name}; limactl create ... {}; limactl start {INSTANCE}; then the Linux \
                 rayx, the recorded clones and rayx setup inside it",
                guest::lima_template(pinned_ubuntu())
            );
            return Ok((Migration::Planned, None));
        }
        if !consent(m.options, prompter, &question) {
            return Ok((
                Migration::Declined(format!(
                    "{INSTANCE} runs Ubuntu {}, older than {}; nothing was changed: run `rayx \
                     setup --lima --yes` to replace it",
                    old.version,
                    pinned_ubuntu()
                )),
                None,
            ));
        }
        // The rename and its record are one resumable step: nothing is stopped or renamed unless
        // the record can be read and written.
        records::update_guests(m.machine, m.host.os, |_| {}).map_err(|e| {
            format!("{e}; {INSTANCE} was not changed, fix or remove that file and run this again")
        })?;
        let instances = lima::list_instances(&tool, runner).map_err(|e| e.to_string())?;
        let running = instances
            .iter()
            .find(|i| i.name == INSTANCE)
            .unwrap_or(&old.instance)
            .is_running();
        if instances.iter().any(|i| i.name == old_name) {
            return Err(format!(
                "the VM {old_name} exists already: remove it with `rayx lima remove {old_name}` \
                 first"
            ));
        }
        if running {
            let _ = writeln!(out, "==> lima: stopping the VM {INSTANCE}");
            lima::run_limactl(
                runner,
                &tool.spec(
                    CommandSpec::new("limactl")
                        .args(["stop", INSTANCE])
                        .interactive(),
                ),
                "stopping the VM",
            )
            .map_err(|e| e.to_string())?;
        }
        let _ = writeln!(out, "==> lima: renaming the VM {INSTANCE} to {old_name}");
        lima::run_limactl(
            runner,
            &tool.spec(
                CommandSpec::new("limactl")
                    // `rename` asks "start the instance now?" in a terminal unless told not to.
                    .args(["rename", "--tty=false", INSTANCE, &old_name])
                    .interactive(),
            ),
            "renaming the VM",
        )
        .map_err(|e| e.to_string())?;
        let version = old.version.clone();
        let renamed = old_name.clone();
        if let Err(error) = records::update_guests(m.machine, m.host.os, |file| {
            file.rename_and_retire(records::LIMA, INSTANCE, &renamed, &version);
        }) {
            // Put the name back so the next run starts from a state it understands.
            let undone = lima::run_limactl(
                runner,
                &tool.spec(
                    CommandSpec::new("limactl")
                        .args(["rename", "--tty=false", &old_name, INSTANCE])
                        .interactive(),
                ),
                "undoing the rename",
            );
            return Err(match undone {
                Ok(()) => format!("{error}; the VM was renamed back to {INSTANCE}"),
                Err(_) => format!(
                    "{error}; the VM is now named {old_name}: rename it back with `limactl \
                     rename {old_name} {INSTANCE}` and run this again"
                ),
            });
        }
    }

    if resumed && m.options.check {
        let _ = writeln!(
            out,
            "A migration of {INSTANCE} was interrupted: {old_name} is renamed and recorded. Without \
             --check this run would create {INSTANCE} from {} with the old VM's settings, then \
             set it up and record it",
            guest::lima_template(pinned_ubuntu())
        );
        return Ok((Migration::Planned, None));
    }
    // The new VM, unless an interrupted run made it.
    let instances = lima::list_instances(&tool, runner).map_err(|e| e.to_string())?;
    match instances.iter().find(|i| i.name == INSTANCE) {
        None => {
            let _ = writeln!(
                out,
                "==> lima: creating the VM {INSTANCE} (Ubuntu {}, {} CPUs, {} GiB memory, {} GiB \
                 disk, like {old_name})",
                pinned_ubuntu(),
                settings.cpus,
                settings.memory_gib,
                settings.disk_gib
            );
            lima::run_limactl(
                runner,
                &tool.spec(lima::create_command(&settings, m.home)),
                "creating the VM",
            )
            .map_err(|e| e.to_string())?;
            lima::start_vm(runner, &tool, out).map_err(|e| e.to_string())?;
        }
        Some(existing) if !existing.is_running() => {
            lima::start_vm(runner, &tool, out).map_err(|e| e.to_string())?;
        }
        Some(_) => {}
    }

    let new_guest = Guest::lima(INSTANCE).with_path_dir(m.path_dir.clone());
    let binary = LinuxBinary {
        version: m.options.version.clone(),
        arch: m.host.arch,
        override_path: m.options.lima_binary.clone(),
    };
    guest::ensure_linux_binary(&new_guest, &binary, runner, out)?;
    let restored = restore_clones(
        &new_guest,
        old.entry.as_ref(),
        m.machine,
        runner,
        out,
        |source| source.to_string(),
    )?;
    let targets: Vec<String> = restored.iter().map(|c| c.target.clone()).collect();
    let flags = setup_flags(m.options, old.entry.as_ref());
    setup_in_clones(&new_guest, &flags, &targets, runner, out)?;

    let vm = VmRecord {
        cpus: settings.cpus,
        memory_gib: settings.memory_gib,
        disk_gib: settings.disk_gib,
        home_writable: settings.home_writable,
    };
    let ubuntu = guest::os_version(&new_guest, runner).unwrap_or_else(|| pinned_ubuntu().into());
    let recorded_flags = old
        .entry
        .as_ref()
        .map(|e| e.flags.clone())
        .unwrap_or_default();
    for clone in restored.iter().map(Some).chain(std::iter::once(None)) {
        let update = GuestUpdate {
            kind: records::LIMA,
            name: INSTANCE.to_string(),
            ubuntu: ubuntu.clone(),
            flags: recorded_flags.clone(),
            clone: clone.cloned(),
            vm: Some(vm),
        };
        if let Err(error) = records::record_guest(m.machine, m.host.os, &update, false) {
            record_error(&error);
            break;
        }
    }
    let finished = old_name.clone();
    if let Err(error) = records::update_guests(m.machine, m.host.os, |file| {
        file.finish_migration(records::LIMA, &finished);
    }) {
        record_error(&error);
    }
    let _ = writeln!(
        out,
        "{INSTANCE} now runs Ubuntu {ubuntu}. The old VM {old_name} is kept, retired; when you no \
         longer need it: rayx lima remove {old_name}"
    );
    Ok((
        Migration::Done {
            new: INSTANCE.to_string(),
            old: old_name,
        },
        Some(vm),
    ))
}

/// Re-clones every recorded clone of the old guest into the new one from its host source.
/// `host_path` turns the recorded source into the path the new guest reads it at.
fn restore_clones(
    guest: &Guest,
    entry: Option<&GuestRecord>,
    machine: &dyn Machine,
    runner: &mut Runner,
    out: &mut dyn Write,
    host_path: impl Fn(&str) -> String,
) -> Result<Vec<CloneRecord>, String> {
    let mut restored = Vec::new();
    let Some(entry) = entry else {
        return Ok(restored);
    };
    for clone in &entry.clones {
        if !machine.is_dir(Path::new(&clone.source)) {
            eprintln!(
                "rayx: the checkout {} no longer exists on this machine; its clone {} is not \
                 restored",
                clone.source, clone.target
            );
            continue;
        }
        let name = guest::checkout_name(Path::new(&clone.source))
            .ok_or_else(|| format!("{} has no directory name", clone.source))?;
        let source = CloneSource {
            name,
            path: host_path(&clone.source),
            branch: clone.branch.clone(),
        };
        guest::clone_checkout(guest, &clone.target, &source, runner, out)?;
        restored.push(clone.clone());
    }
    Ok(restored)
}

// ---------------------------------------------------------------------------------------------
// WSL

/// What a WSL migration needs from the run around it.
pub struct WslMigration<'a> {
    pub options: &'a RefreshOptions,
    pub host: &'a HostFacts,
    pub machine: &'a dyn Machine,
}

/// The recorded WSL distribution older than the pin that a migration replaces, if any.
pub fn outdated_wsl(file: &GuestsFile) -> Option<GuestRecord> {
    file.guests
        .iter()
        .find(|g| {
            g.kind == records::WSL
                && !g.retired
                && g.name != wsl::distribution()
                && older_than_pin(&g.ubuntu)
        })
        .cloned()
}

/// Installs `Ubuntu-<pin>` next to the old distribution, restores its clones and flags, and
/// retires the old entry once the new distribution is set up.
pub fn migrate_wsl(
    m: &WslMigration,
    runner: &mut Runner,
    prompter: &mut Option<&mut dyn Prompter>,
    out: &mut dyn Write,
    old: &GuestRecord,
    old_version: &str,
) -> Result<Migration, String> {
    let new_name = wsl::distribution();
    let registered = m
        .machine
        .wsl_distributions()
        .iter()
        .any(|d| d.name == new_name);
    if !registered {
        let old_guest = Guest::wsl(&old.name);
        let dirty = guest::dirty_clones(&old_guest, runner);
        let question = format!(
            "Create {new_name} with Ubuntu {} next to {}?",
            pinned_ubuntu(),
            old.name
        );
        let _ = writeln!(
            out,
            "{} runs Ubuntu {old_version}, older than {}: it is never upgraded in place. The old \
             distribution is kept, retired, until you remove it: rayx wsl remove {}",
            old.name,
            pinned_ubuntu(),
            old.name
        );
        report_dirty(out, &old.name, &dirty);
        if m.options.check {
            let _ = writeln!(
                out,
                "{question}\nwould run: wsl --install -d {new_name}; then the Linux rayx, the \
                 recorded clones and rayx setup inside it"
            );
            return Ok(Migration::Planned);
        }
        if !consent(m.options, prompter, &question) {
            return Ok(Migration::Declined(format!(
                "{} runs Ubuntu {old_version}, older than {}; nothing was changed: run `rayx \
                 setup --wsl --yes` to replace it",
                old.name,
                pinned_ubuntu()
            )));
        }
    }
    wsl::ensure_distribution(runner, m.machine, out).map_err(|e| e.to_string())?;

    let new_guest = Guest::wsl(&new_name);
    let binary = LinuxBinary {
        version: m.options.version.clone(),
        arch: m.host.arch,
        override_path: m.options.wsl_binary.clone(),
    };
    guest::ensure_linux_binary(&new_guest, &binary, runner, out)?;
    let restored = restore_clones_wsl(&new_guest, old, m.machine, runner, out)?;
    let targets: Vec<String> = restored.iter().map(|c| c.target.clone()).collect();
    let flags = setup_flags(m.options, Some(old));
    setup_in_clones(&new_guest, &flags, &targets, runner, out)?;

    let ubuntu = guest::os_version(&new_guest, runner).unwrap_or_else(|| pinned_ubuntu().into());
    for clone in restored.iter().map(Some).chain(std::iter::once(None)) {
        let update = GuestUpdate {
            kind: records::WSL,
            name: new_name.clone(),
            ubuntu: ubuntu.clone(),
            flags: old.flags.clone(),
            clone: clone.cloned(),
            vm: None,
        };
        if let Err(error) = records::record_guest(m.machine, m.host.os, &update, false) {
            record_error(&error);
            break;
        }
    }
    let old_name = old.name.clone();
    if let Err(error) = records::update_guests(m.machine, m.host.os, |file| {
        file.retire(records::WSL, &old_name);
    }) {
        record_error(&error);
    }
    let _ = writeln!(
        out,
        "{new_name} now runs Ubuntu {ubuntu}. The old distribution {} is kept, retired; when you \
         no longer need it: rayx wsl remove {}",
        old.name, old.name
    );
    Ok(Migration::Done {
        new: new_name,
        old: old.name.clone(),
    })
}

fn restore_clones_wsl(
    guest: &Guest,
    old: &GuestRecord,
    machine: &dyn Machine,
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<Vec<CloneRecord>, String> {
    let mut restored = Vec::new();
    for clone in &old.clones {
        let windows = Path::new(&clone.source);
        let Ok(path) = wsl::to_wsl_path(windows) else {
            eprintln!(
                "rayx: the checkout {} is not on a Windows drive; its clone {} is not restored",
                clone.source, clone.target
            );
            continue;
        };
        if !machine.is_dir(windows) {
            eprintln!(
                "rayx: the checkout {} no longer exists on this machine; its clone {} is not \
                 restored",
                clone.source, clone.target
            );
            continue;
        }
        let name = guest::checkout_name(windows)
            .ok_or_else(|| format!("{} has no directory name", clone.source))?;
        let source = CloneSource {
            name,
            path,
            branch: clone.branch.clone(),
        };
        guest::clone_checkout(guest, &clone.target, &source, runner, out)?;
        restored.push(clone.clone());
    }
    Ok(restored)
}
