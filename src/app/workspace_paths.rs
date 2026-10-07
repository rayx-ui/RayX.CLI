//! Locations outside the RayX crates that xtask needs: the sibling gpux workspace and the
//! sources of the root workspace's `[patch]` entries. Both are read from the root
//! `Cargo.toml`, so moving those trees only changes that manifest.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::app::fs_util::repo_root;

fn root_manifest() -> Result<(PathBuf, toml::Table)> {
    let root = repo_root()?;
    let path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let manifest = toml::from_str::<toml::Table>(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok((root, manifest))
}

/// The gpux workspace root, from the root manifest's
/// `[workspace.dependencies] gpux = { path = "<gpux>/crates/gpui" }` entry.
pub fn gpux_root() -> Result<PathBuf> {
    let (root, manifest) = root_manifest()?;
    let gpui = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(|dependencies| dependencies.get("gpux"))
        .and_then(|gpux| gpux.get("path"))
        .and_then(|path| path.as_str())
        .ok_or_else(|| {
            anyhow!("the root Cargo.toml must declare [workspace.dependencies] gpux with a path")
        })?;
    let gpux = root
        .join(gpui)
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("gpux path {gpui} is not `<gpux>/crates/gpui`"))?;
    if !gpux.join("Cargo.toml").is_file() {
        bail!("the gpux workspace is missing at {}", gpux.display());
    }
    Ok(gpux)
}

/// The threaded-WASM toolchain, from the root manifest's
/// `[workspace.metadata.rayx] web-toolchain = "nightly-<date>"` entry.
pub fn web_toolchain() -> Result<String> {
    let (_, manifest) = root_manifest()?;
    manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("metadata"))
        .and_then(|metadata| metadata.get("rayx"))
        .and_then(|rayx| rayx.get("web-toolchain"))
        .and_then(|toolchain| toolchain.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "the root Cargo.toml must declare [workspace.metadata.rayx] web-toolchain = \"nightly-<date>\""
            )
        })
}

/// The absolute source path of the root manifest's `[patch.<registry>] <name>` entry,
/// or `None` when the root workspace does not patch it.
pub fn root_patch_path(registry: &str, name: &str) -> Result<Option<PathBuf>> {
    let (root, manifest) = root_manifest()?;
    Ok(manifest
        .get("patch")
        .and_then(|patch| patch.get(registry))
        .and_then(|registry| registry.get(name))
        .and_then(|entry| entry.get("path"))
        .and_then(|path| path.as_str())
        .map(|path| root.join(path)))
}
