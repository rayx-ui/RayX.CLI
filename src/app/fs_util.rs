use anyhow::{Context, Result, anyhow};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub fn repo_root() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("xtask manifest directory has no repository parent"))
}

pub fn absolutize_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(repo_root()?.join(path))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn command_path_removes_windows_verbatim_disk_prefix() {
        let path = Path::new(r"\\?\C:\repo\apps\example\Cargo.toml");

        assert_eq!(
            command_path(path),
            PathBuf::from(r"C:\repo\apps\example\Cargo.toml")
        );
    }

    #[cfg(windows)]
    #[test]
    fn command_path_removes_windows_verbatim_unc_prefix() {
        let path = Path::new(r"\\?\UNC\server\share\Cargo.toml");

        assert_eq!(
            command_path(path),
            PathBuf::from(r"\\server\share\Cargo.toml")
        );
    }
}
