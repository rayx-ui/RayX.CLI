//! The name of a Lima VM `rayx` manages. The default `rayx` keeps every earlier behaviour; a
//! project that wants its own VM chooses another one with `--vm`.

use std::fmt;

/// The most characters a VM name may have.
pub const MAX_LEN: usize = 40;

/// A usable Lima VM name: 1 to 40 lowercase letters, digits and hyphens, starting with a letter,
/// and not Lima's own `default`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VmName(String);

/// Why a text is not a usable VM name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmNameError {
    text: String,
}

impl fmt::Display for VmNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a usable VM name: use 1 to {MAX_LEN} lowercase letters, digits and \
             hyphens, starting with a letter and not `default`, for example `app` or `my-project`",
            self.text
        )
    }
}

impl std::error::Error for VmNameError {}

impl VmName {
    /// Checks `text` against the rules of a VM name.
    pub fn parse(text: &str) -> Result<Self, VmNameError> {
        let usable = !text.is_empty()
            && text.len() <= MAX_LEN
            && text != "default"
            && text.starts_with(|c: char| c.is_ascii_lowercase())
            && text
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if usable {
            Ok(Self(text.to_string()))
        } else {
            Err(VmNameError {
                text: text.to_string(),
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the name every VM had before names could be chosen.
    pub fn is_default(&self) -> bool {
        self.0 == super::INSTANCE
    }

    /// The name a migrated VM is kept under: `<name>-<old Ubuntu version>`. A version has a dot,
    /// which no VM name has, so no VM's retired name is another VM's name.
    pub fn retired(&self, ubuntu: &str) -> String {
        format!("{}-{ubuntu}", self.0)
    }

    /// The VM a retired `<name>-<version>` entry belongs to, when `entry` has that form for
    /// `ubuntu` and its prefix is a usable name.
    pub fn of_retired(entry: &str, ubuntu: &str) -> Option<Self> {
        let prefix = entry.strip_suffix(ubuntu)?.strip_suffix('-')?;
        Self::parse(prefix).ok()
    }

    /// The flags that select this VM in a `rayx setup` command line: `--lima` for the default,
    /// `--lima --vm <name>` for another.
    pub fn lima_flags(&self) -> String {
        if self.is_default() {
            "--lima".to_string()
        } else {
            format!("--lima --vm {}", self.0)
        }
    }
}

impl Default for VmName {
    fn default() -> Self {
        Self(super::INSTANCE.to_string())
    }
}

impl fmt::Display for VmName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
