use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// An absolute path: `path` itself, or `path` under the current directory.
pub fn absolutize_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .context("reading the current directory")?
        .join(path))
}

pub fn command_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }

    path.to_path_buf()
}

pub fn reset_dir(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))?;
    }
    fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))
}

pub fn collect_files_with_extension(
    path: &Path,
    extension: &str,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    if !path.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path).with_context(|| format!("reading {}", path.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files_with_extension(&path, extension, files)?;
        } else if path.extension().and_then(OsStr::to_str) == Some(extension) {
            files.push(path);
        }
    }
    Ok(())
}

pub fn collect_files_named(path: &Path, name: &str, files: &mut Vec<PathBuf>) -> Result<()> {
    if !path.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path).with_context(|| format!("reading {}", path.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files_named(&path, name, files)?;
        } else if path.file_name().and_then(OsStr::to_str) == Some(name) {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn command_path_removes_windows_verbatim_disk_prefix() {
        let path = Path::new(r"\\?\C:\repo\apps\example\Cargo.toml");

        assert_eq!(
            command_path(path),
            PathBuf::from(r"C:\repo\apps\example\Cargo.toml")
        );
    }

    #[test]
    fn command_path_removes_windows_verbatim_unc_prefix() {
        let path = Path::new(r"\\?\UNC\server\share\Cargo.toml");

        assert_eq!(
            command_path(path),
            PathBuf::from(r"\\server\share\Cargo.toml")
        );
    }
}
