//! What size the VM `rayx` gets: the defaults capped to what this Mac can spare, the `--vm-*`
//! flags validated against the same limits, and the one `Customize? [y/N]` question on creation.

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::host::{CommandSpec, Outcome, Runner};
use crate::setup::{Machine, ask_on_terminal};

use super::{Instance, LimaError, VmFlags, VmSettings};

/// The smallest memory and disk a VM is given, in GiB.
pub const MIN_MEMORY_GIB: u32 = 2;
pub const MIN_DISK_GIB: u32 = 30;

/// Free space on the Lima home volume below which no VM is created, in GiB.
pub const MIN_FREE_GIB: u64 = 10;

const GIB: u64 = 1024 * 1024 * 1024;

/// What this Mac has: CPUs, memory in bytes and free space on the Lima home volume in KiB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostCapacity {
    pub cpus: u32,
    pub memory_bytes: u64,
    pub free_kib: u64,
}

impl HostCapacity {
    pub fn memory_gib(&self) -> u64 {
        self.memory_bytes / GIB
    }

    pub fn free_gib(&self) -> u64 {
        self.free_kib / (1024 * 1024)
    }

    /// The most CPUs a VM gets: all but two, and at least one.
    pub fn cpu_cap(&self) -> u32 {
        self.cpus.saturating_sub(2).max(1)
    }

    /// The most memory a VM gets in GiB: half of this Mac's, and at least two.
    pub fn memory_cap_gib(&self) -> u32 {
        u32::try_from(self.memory_gib() / 2)
            .unwrap_or(u32::MAX)
            .max(MIN_MEMORY_GIB)
    }
}

/// Where Lima keeps its instances: `$LIMA_HOME`, else `~/.lima`; the home itself while that does
/// not exist yet (the volume is the same).
fn lima_home_volume(machine: &dyn Machine, home: &Path) -> PathBuf {
    let lima_home = machine
        .env("LIMA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".lima"));
    if machine.is_dir(&lima_home) {
        lima_home
    } else {
        home.to_path_buf()
    }
}

/// Reads this Mac with `sysctl -n hw.ncpu hw.memsize` and `df -k` on the Lima home volume.
pub fn read_capacity(
    runner: &mut Runner,
    machine: &dyn Machine,
    home: &Path,
) -> Result<HostCapacity, LimaError> {
    let unreadable =
        |what: &str| LimaError::Failed(format!("cannot read this Mac's {what} to size the VM"));
    let sysctl = runner
        .query(&CommandSpec::new("sysctl").args(["-n", "hw.ncpu", "hw.memsize"]))
        .ok()
        .filter(Outcome::is_success)
        .ok_or_else(|| unreadable("CPUs and memory"))?;
    let mut numbers = sysctl
        .stdout
        .split_whitespace()
        .filter_map(|word| word.parse::<u64>().ok());
    let (Some(cpus), Some(memory_bytes)) = (numbers.next(), numbers.next()) else {
        return Err(unreadable("CPUs and memory"));
    };
    let volume = lima_home_volume(machine, home);
    let df = runner
        .query(
            &CommandSpec::new("df")
                .arg("-k")
                .arg(volume.display().to_string()),
        )
        .ok()
        .filter(Outcome::is_success)
        .ok_or_else(|| unreadable("free disk space"))?;
    let free_kib = parse_df_available(&df.stdout).ok_or_else(|| unreadable("free disk space"))?;
    Ok(HostCapacity {
        cpus: u32::try_from(cpus).unwrap_or(u32::MAX),
        memory_bytes,
        free_kib,
    })
}

/// The `Available` column (KiB) of the last line of `df -k`.
pub fn parse_df_available(text: &str) -> Option<u64> {
    text.lines()
        .rfind(|line| !line.trim().is_empty())?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()
}

/// The settings a new VM gets and what to tell the owner about them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sizing {
    pub settings: VmSettings,
    pub notes: Vec<String>,
}

/// The defaults capped to `capacity`, then the explicit `flags` validated against the same
/// limits. A flag outside them is an error naming the allowed range.
pub fn effective(
    defaults: &VmSettings,
    capacity: &HostCapacity,
    flags: &VmFlags,
) -> Result<Sizing, LimaError> {
    let (cpu_cap, memory_cap) = (capacity.cpu_cap(), capacity.memory_cap_gib());
    let mut notes = Vec::new();
    let mut settings = *defaults;

    match flags.cpus {
        Some(cpus) if !(1..=cpu_cap).contains(&cpus) => {
            return Err(refused(
                "--vm-cpus",
                cpus,
                "CPUs",
                1,
                cpu_cap,
                &format!("this Mac has {} CPUs", capacity.cpus),
            ));
        }
        Some(cpus) => settings.cpus = cpus,
        None if defaults.cpus > cpu_cap => {
            notes.push(format!(
                "this Mac has {} CPUs: the VM gets {cpu_cap} CPUs",
                capacity.cpus
            ));
            settings.cpus = cpu_cap;
        }
        None => {}
    }
    match flags.memory_gib {
        Some(memory) if !(MIN_MEMORY_GIB..=memory_cap).contains(&memory) => {
            return Err(refused(
                "--vm-memory",
                memory,
                "GiB memory",
                MIN_MEMORY_GIB,
                memory_cap,
                &format!("this Mac has {} GiB memory", capacity.memory_gib()),
            ));
        }
        Some(memory) => settings.memory_gib = memory,
        None if defaults.memory_gib > memory_cap => {
            notes.push(format!(
                "this Mac has {} GiB memory: the VM gets {memory_cap} GiB",
                capacity.memory_gib()
            ));
            settings.memory_gib = memory_cap;
        }
        None => {}
    }
    if let Some(disk) = flags.disk_gib {
        if disk < MIN_DISK_GIB {
            return Err(LimaError::Failed(format!(
                "--vm-disk {disk} is too small: the disk needs at least {MIN_DISK_GIB} GiB"
            )));
        }
        settings.disk_gib = disk;
    }
    settings.home_writable = flags.home_writable;
    Ok(Sizing { settings, notes })
}

fn refused(flag: &str, value: u32, unit: &str, min: u32, max: u32, host: &str) -> LimaError {
    LimaError::Failed(format!(
        "{flag} {value} is outside what this Mac can spare ({host}): choose {min} to {max} {unit}"
    ))
}

/// Whether the Lima home volume has room, as a note or an error: fewer than 10 GiB free stops;
/// less than the disk size only notes it, since the disk image is sparse.
pub fn check_free_space(
    capacity: &HostCapacity,
    settings: &VmSettings,
    notes: &mut Vec<String>,
) -> Result<(), LimaError> {
    let free = capacity.free_gib();
    if free < MIN_FREE_GIB {
        return Err(LimaError::Failed(format!(
            "only {free} GiB are free on the volume Lima uses: free at least {MIN_FREE_GIB} GiB \
             before creating the VM"
        )));
    }
    if free < u64::from(settings.disk_gib) {
        notes.push(format!(
            "{free} GiB are free on the volume Lima uses and the disk is {} GiB: the disk image \
             is sparse and grows as the VM fills it",
            settings.disk_gib
        ));
    }
    Ok(())
}

/// A warning when the VMs already running plus the new one would take more than three quarters
/// of this Mac's memory.
pub fn memory_warning(
    instances: &[Instance],
    settings: &VmSettings,
    capacity: &HostCapacity,
    name: &str,
) -> Option<String> {
    let running: Vec<&Instance> = instances.iter().filter(|i| i.is_running()).collect();
    let total =
        running.iter().map(|i| i.memory).sum::<u64>() + u64::from(settings.memory_gib) * GIB;
    if running.is_empty() || total * 4 <= capacity.memory_bytes * 3 {
        return None;
    }
    let names: Vec<String> = running
        .iter()
        .map(|i| format!("{} ({} GiB)", i.name, i.memory / GIB))
        .collect();
    Some(format!(
        "running VMs {} plus {name} ({} GiB) take more than three quarters of this Mac's {} GiB \
         memory; stop one if the Mac gets slow",
        names.join(", "),
        settings.memory_gib,
        capacity.memory_gib()
    ))
}

/// Asks the owner questions. A terminal implementation and a scripted one for tests.
pub trait Prompter {
    /// Whether anybody can answer; without a terminal nothing is asked.
    fn interactive(&self) -> bool {
        true
    }
    /// A yes/no question (shown with `[y/N]`); anything but yes is no.
    fn confirm(&mut self, question: &str) -> bool;
    /// A question with the current value shown; `None` when nothing could be read, an empty
    /// answer keeps the shown value.
    fn ask(&mut self, question: &str, shown: &str) -> Option<String>;
}

/// Asks on the terminal when there is one.
pub struct TerminalPrompter {
    pub terminal: bool,
}

impl Prompter for TerminalPrompter {
    fn interactive(&self) -> bool {
        self.terminal
    }

    fn confirm(&mut self, question: &str) -> bool {
        self.terminal && ask_on_terminal(question)
    }

    fn ask(&mut self, question: &str, shown: &str) -> Option<String> {
        if !self.terminal {
            return None;
        }
        print!("{question} [{shown}]: ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        match std::io::stdin().lock().read_line(&mut answer) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(answer.trim().to_string()),
        }
    }
}

/// Answers from a script and records the questions, for tests.
#[derive(Debug, Default)]
pub struct ScriptedPrompter {
    pub terminal: bool,
    /// Answers to `confirm`, in order; once used up every question is answered `fallback`.
    pub confirms: VecDeque<bool>,
    pub fallback: bool,
    /// Answers to `ask`, in order; once used up `ask` reads nothing.
    pub answers: VecDeque<String>,
    /// Every question asked, `confirm` and `ask` alike.
    pub asked: Vec<String>,
}

impl ScriptedPrompter {
    /// A terminal that answers every yes/no question with `answer`.
    pub fn always(answer: bool) -> Self {
        Self {
            terminal: true,
            fallback: answer,
            ..Self::default()
        }
    }

    pub fn with_confirms(mut self, answers: impl IntoIterator<Item = bool>) -> Self {
        self.confirms.extend(answers);
        self
    }

    pub fn with_answers<'a>(mut self, answers: impl IntoIterator<Item = &'a str>) -> Self {
        self.answers.extend(answers.into_iter().map(str::to_string));
        self
    }
}

impl Prompter for ScriptedPrompter {
    fn interactive(&self) -> bool {
        self.terminal
    }

    fn confirm(&mut self, question: &str) -> bool {
        self.asked.push(question.to_string());
        self.confirms.pop_front().unwrap_or(self.fallback)
    }

    fn ask(&mut self, question: &str, shown: &str) -> Option<String> {
        self.asked.push(format!("{question} [{shown}]"));
        self.answers.pop_front()
    }
}

/// The summary line of settings, as the customize question shows it.
pub fn summary(name: &str, settings: &VmSettings) -> String {
    format!(
        "Create Lima VM {name}: {} CPUs, {} GiB memory, {} GiB disk, home mounted {}.",
        settings.cpus,
        settings.memory_gib,
        settings.disk_gib,
        if settings.home_writable {
            "writable"
        } else {
            "read-only"
        }
    )
}

/// Asks once whether to customize `settings` and, on yes, each value in turn. Invalid input asks
/// again with the allowed range; nothing left to read keeps the shown value.
pub fn customize(
    name: &str,
    settings: VmSettings,
    capacity: &HostCapacity,
    prompter: &mut dyn Prompter,
    out: &mut dyn Write,
) -> VmSettings {
    if !prompter.confirm(&format!("{} Customize?", summary(name, &settings))) {
        return settings;
    }
    let mut chosen = settings;
    chosen.cpus = ask_number(prompter, out, "CPUs", settings.cpus, 1, capacity.cpu_cap());
    chosen.memory_gib = ask_number(
        prompter,
        out,
        "Memory in GiB",
        settings.memory_gib,
        MIN_MEMORY_GIB,
        capacity.memory_cap_gib(),
    );
    chosen.disk_gib = ask_number(
        prompter,
        out,
        "Disk in GiB",
        settings.disk_gib,
        MIN_DISK_GIB,
        u32::MAX,
    );
    // A home the flag already made writable stays writable: nothing is asked about it.
    chosen.home_writable = settings.home_writable || prompter.confirm("Mount the home writable?");
    chosen
}

fn ask_number(
    prompter: &mut dyn Prompter,
    out: &mut dyn Write,
    question: &str,
    shown: u32,
    min: u32,
    max: u32,
) -> u32 {
    loop {
        let Some(answer) = prompter.ask(question, &shown.to_string()) else {
            return shown;
        };
        if answer.is_empty() {
            return shown;
        }
        match answer.parse::<u32>() {
            Ok(value) if (min..=max).contains(&value) => return value,
            _ => {
                let range = if max == u32::MAX {
                    format!("at least {min}")
                } else {
                    format!("{min} to {max}")
                };
                let _ = writeln!(out, "{question}: enter a whole number, {range}");
            }
        }
    }
}
