//! Persistent, idempotent edits of the user PATH.
//!
//! Windows keeps the user PATH in `HKCU\Environment\Path`; [`PathStore`] abstracts that value so
//! tests run on an in-memory store. Linux and macOS get one marked block per login profile.
//! Every edit reports whether anything changed, so callers can tell the user to open a new shell.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::Os;

/// The first line of the block `rayx` owns in a shell profile.
pub const BLOCK_START: &str = "# >>> rayx >>>";
/// The last line of the block `rayx` owns in a shell profile.
pub const BLOCK_END: &str = "# <<< rayx <<<";

/// What an edit did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathChange {
    /// The entry was added (or would be added in a dry run).
    Added,
    /// The entry was already present; nothing changed.
    AlreadyPresent,
}

impl PathChange {
    /// The notice to show after the edit: running shells keep their old PATH.
    pub fn notice(self) -> Option<&'static str> {
        match self {
            PathChange::Added => Some("open a new shell for the PATH change to take effect"),
            PathChange::AlreadyPresent => None,
        }
    }
}

/// The type of the stored `Path` value. Windows keeps `%VARIABLE%` references only in
/// `REG_EXPAND_SZ`, so an edit must not turn one into the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathValue {
    /// `REG_SZ`.
    Plain(String),
    /// `REG_EXPAND_SZ`.
    Expandable(String),
}

impl PathValue {
    pub fn text(&self) -> &str {
        match self {
            PathValue::Plain(text) | PathValue::Expandable(text) => text,
        }
    }

    fn with_text(&self, text: String) -> Self {
        match self {
            PathValue::Plain(_) => PathValue::Plain(text),
            PathValue::Expandable(_) => PathValue::Expandable(text),
        }
    }
}

/// Where the Windows user PATH lives.
pub trait PathStore {
    /// The current value, or `None` when the user has no `Path` value yet.
    fn read(&self) -> io::Result<Option<PathValue>>;
    /// Replaces the value.
    fn write(&mut self, value: &PathValue) -> io::Result<()>;
    /// Tells running programs that the environment changed (`WM_SETTINGCHANGE`).
    fn broadcast_change(&mut self) -> io::Result<()>;
    /// Expands `%VARIABLE%` references the way Windows does when it builds a process
    /// environment, so `%USERPROFILE%\bin` and its absolute form count as the same entry.
    fn expand(&self, text: &str) -> String {
        text.to_string()
    }
}

/// A PATH value held in memory, for tests.
#[derive(Clone, Debug, Default)]
pub struct MemoryPathStore {
    value: Option<PathValue>,
    variables: Vec<(String, String)>,
    broadcasts: usize,
    fail_broadcast: bool,
}

impl MemoryPathStore {
    pub fn new(value: Option<PathValue>) -> Self {
        Self {
            value,
            ..Self::default()
        }
    }

    /// Defines an environment variable that [`PathStore::expand`] substitutes.
    pub fn with_variable(mut self, name: &str, value: &str) -> Self {
        self.variables.push((name.to_string(), value.to_string()));
        self
    }

    pub fn value(&self) -> Option<&PathValue> {
        self.value.as_ref()
    }

    /// Makes [`PathStore::broadcast_change`] fail, as a timed-out `WM_SETTINGCHANGE` does.
    pub fn with_failing_broadcast(mut self) -> Self {
        self.fail_broadcast = true;
        self
    }

    /// How many times [`PathStore::broadcast_change`] was called.
    pub fn broadcasts(&self) -> usize {
        self.broadcasts
    }
}

impl PathStore for MemoryPathStore {
    fn read(&self) -> io::Result<Option<PathValue>> {
        Ok(self.value.clone())
    }

    fn write(&mut self, value: &PathValue) -> io::Result<()> {
        self.value = Some(value.clone());
        Ok(())
    }

    fn broadcast_change(&mut self) -> io::Result<()> {
        self.broadcasts += 1;
        if self.fail_broadcast {
            Err(io::Error::other("broadcast timed out"))
        } else {
            Ok(())
        }
    }

    fn expand(&self, text: &str) -> String {
        expand_variables(text, |name| {
            self.variables
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        })
    }
}

/// Substitutes `%NAME%` references using `lookup`; unknown names stay as written.
pub fn expand_variables(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => match lookup(&after[..end]) {
                Some(value) => {
                    out.push_str(&value);
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Prepends `entry` to the Windows user PATH in `store` unless it is already there, preserving the
/// value type, and broadcasts the change (best effort: a failed broadcast is not an error). With `dry_run` the store is left untouched.
pub fn prepend_to_windows_path(
    store: &mut dyn PathStore,
    entry: &str,
    dry_run: bool,
) -> io::Result<PathChange> {
    let current = store.read()?;
    let existing = current.as_ref().map_or("", PathValue::text);
    let wanted = normalize_windows_entry(&store.expand(entry));
    let present = existing
        .split(';')
        .filter(|part| !part.trim().is_empty())
        .any(|part| normalize_windows_entry(&store.expand(part)) == wanted);
    if present {
        return Ok(PathChange::AlreadyPresent);
    }
    if !dry_run {
        let text = if existing.trim().is_empty() {
            entry.to_string()
        } else {
            format!("{entry};{existing}")
        };
        let value = match &current {
            Some(value) => value.with_text(text),
            // A new value is expandable, like the one Windows creates for the user PATH.
            None => PathValue::Expandable(text),
        };
        store.write(&value)?;
        // The value is persisted; the broadcast only refreshes running programs, and a slow
        // window can time it out. The caller's new-shell notice covers the rest.
        let _ = store.broadcast_change();
    }
    Ok(PathChange::Added)
}

fn normalize_windows_entry(entry: &str) -> String {
    entry
        .trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// The login profiles that carry the PATH block for the shell at `shell_path` (the `SHELL`
/// variable): zsh reads `~/.zprofile`; bash logins read `~/.profile` and terminal emulators on
/// Linux start non-login shells that read `~/.bashrc`; anything else reads `~/.profile`.
pub fn profile_files(shell_path: &str, home: &Path) -> Vec<PathBuf> {
    let shell = shell_path.rsplit(['/', '\\']).next().unwrap_or_default();
    match shell {
        "zsh" => vec![home.join(".zprofile")],
        "bash" => vec![home.join(".profile"), home.join(".bashrc")],
        _ => vec![home.join(".profile")],
    }
}

/// Adds `entry` to the `rayx` block of the profile at `profile`, creating the file or the block as
/// needed and leaving the rest of the file untouched. An entry written as `$HOME/...` expands in
/// the shell. With `dry_run` nothing is written.
pub fn add_to_profile(profile: &Path, entry: &str, dry_run: bool) -> io::Result<PathChange> {
    if entry.contains(['"', '`', '\\', '\n'])
        || entry.matches('$').count() > entry.matches("$HOME").count()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("`{entry}` cannot be written to a shell profile"),
        ));
    }
    let text = match fs::read_to_string(profile) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let line = format!("export PATH=\"{entry}:$PATH\"");
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| l.trim() == BLOCK_START);
    let end = start.and_then(|s| {
        lines[s + 1..]
            .iter()
            .position(|l| l.trim() == BLOCK_END)
            .map(|e| s + 1 + e)
    });

    let updated = match (start, end) {
        (Some(start), Some(end)) => {
            if lines[start + 1..end].iter().any(|l| l.trim() == line) {
                return Ok(PathChange::AlreadyPresent);
            }
            let mut out: Vec<&str> = lines[..end].to_vec();
            out.push(&line);
            out.extend_from_slice(&lines[end..]);
            join_lines(&out)
        }
        // A start marker without an end marker is a damaged block: close it instead of adding
        // a second one.
        (Some(start), None) => {
            let mut out: Vec<&str> = lines[..=start].to_vec();
            out.push(&line);
            out.push(BLOCK_END);
            out.extend_from_slice(&lines[start + 1..]);
            join_lines(&out)
        }
        _ => {
            let mut out = text.clone();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("{BLOCK_START}\n{line}\n{BLOCK_END}\n"));
            out
        }
    };
    if !dry_run {
        fs::write(profile, updated)?;
    }
    Ok(PathChange::Added)
}

fn join_lines(lines: &[&str]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// The user PATH of this machine, edited idempotently.
pub enum UserPath {
    Windows(Box<dyn PathStore>),
    Profiles { files: Vec<PathBuf>, home: PathBuf },
}

impl UserPath {
    /// The user PATH of the machine `rayx` runs on: the registry on Windows, and the profile of
    /// `$SHELL` under `$HOME` elsewhere.
    pub fn system(os: Os) -> io::Result<Self> {
        if os == Os::Windows {
            #[cfg(windows)]
            return Ok(UserPath::Windows(Box::new(RegistryPathStore)));
            #[cfg(not(windows))]
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "the Windows registry is not available on this system",
            ));
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        let shell = std::env::var("SHELL").unwrap_or_default();
        Ok(UserPath::Profiles {
            files: profile_files(&shell, &home),
            home,
        })
    }

    /// Adds the directory `entry` to the user PATH. A directory under the home directory is
    /// written as `$HOME/...` in profiles. With `dry_run` nothing is changed and the result says
    /// what would happen.
    pub fn add(&mut self, entry: &Path, dry_run: bool) -> io::Result<PathChange> {
        match self {
            UserPath::Windows(store) => {
                prepend_to_windows_path(store.as_mut(), &entry.display().to_string(), dry_run)
            }
            UserPath::Profiles { files, home } => {
                let text = match entry.strip_prefix(&*home) {
                    Ok(relative) => format!(
                        "$HOME/{}",
                        relative.display().to_string().replace('\\', "/")
                    ),
                    Err(_) => entry.display().to_string(),
                };
                let mut result = PathChange::AlreadyPresent;
                for file in files.iter() {
                    if add_to_profile(file, &text, dry_run)? == PathChange::Added {
                        result = PathChange::Added;
                    }
                }
                Ok(result)
            }
        }
    }
}

#[cfg(windows)]
pub use registry::RegistryPathStore;

#[cfg(windows)]
mod registry {
    use std::io;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, LPARAM, WPARAM};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    use super::{PathStore, PathValue, expand_variables};

    /// The user PATH in `HKCU\Environment\Path`.
    pub struct RegistryPathStore;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn os_error(code: u32) -> io::Error {
        io::Error::from_raw_os_error(code as i32)
    }

    /// An open `HKCU\Environment` key, closed on drop.
    struct Key(HKEY);

    impl Key {
        fn open(access: u32) -> io::Result<Self> {
            let mut key: HKEY = std::ptr::null_mut();
            let name = wide("Environment");
            // SAFETY: `name` is NUL-terminated and `key` is a live out-pointer.
            let status =
                unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut key) };
            if status == ERROR_SUCCESS {
                Ok(Self(key))
            } else {
                Err(os_error(status))
            }
        }
    }

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from `RegOpenKeyExW` and is closed exactly once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    impl PathStore for RegistryPathStore {
        fn read(&self) -> io::Result<Option<PathValue>> {
            let key = Key::open(KEY_QUERY_VALUE)?;
            let name = wide("Path");
            let mut kind = 0u32;
            let mut size = 0u32;
            // SAFETY: a null data pointer asks only for the type and the size in bytes.
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    std::ptr::null_mut(),
                    &mut size,
                )
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            if status != ERROR_SUCCESS {
                return Err(os_error(status));
            }
            let mut buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
            let mut size = (buffer.len() * 2) as u32;
            // SAFETY: `buffer` holds `size` writable bytes.
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    buffer.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            if status != ERROR_SUCCESS {
                return Err(os_error(status));
            }
            buffer.truncate(size as usize / 2);
            while buffer.last() == Some(&0) {
                buffer.pop();
            }
            let text = String::from_utf16_lossy(&buffer);
            match kind {
                REG_SZ => Ok(Some(PathValue::Plain(text))),
                REG_EXPAND_SZ => Ok(Some(PathValue::Expandable(text))),
                other => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("HKCU\\Environment\\Path has unexpected registry type {other}"),
                )),
            }
        }

        fn write(&mut self, value: &PathValue) -> io::Result<()> {
            let key = Key::open(KEY_SET_VALUE)?;
            let name = wide("Path");
            let data = wide(value.text());
            let kind = match value {
                PathValue::Plain(_) => REG_SZ,
                PathValue::Expandable(_) => REG_EXPAND_SZ,
            };
            // SAFETY: `data` is a NUL-terminated UTF-16 buffer of exactly the byte length passed.
            let status = unsafe {
                RegSetValueExW(
                    key.0,
                    name.as_ptr(),
                    0,
                    kind,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                )
            };
            if status == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(os_error(status))
            }
        }

        fn broadcast_change(&mut self) -> io::Result<()> {
            let area = wide("Environment");
            let mut result = 0usize;
            // SAFETY: `area` is a NUL-terminated string that outlives the call; the message is
            // broadcast with a timeout so a hung window cannot block `rayx`.
            let sent = unsafe {
                SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SETTINGCHANGE,
                    0 as WPARAM,
                    area.as_ptr() as LPARAM,
                    SMTO_ABORTIFHUNG,
                    5000,
                    &mut result,
                )
            };
            if sent == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        fn expand(&self, text: &str) -> String {
            expand_variables(text, |name| std::env::var(name).ok())
        }
    }
}
