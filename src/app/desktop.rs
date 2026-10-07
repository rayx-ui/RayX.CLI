use crate::app::args::{ensure_empty, take_flag_value};
use crate::app::assets;
use crate::app::process::run as run_process;
use crate::app::{AppDescriptor, AppFeatureSelection, BuildProfile, package_name_from_manifest};
use crate::host::{Arch, HostFacts};
use anyhow::{Result, anyhow, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RAYX_ASSETS_ROOT_ENV: &str = "RAYX_ASSETS_ROOT";
const RAYX_APP_CONTENT_ROOT_ENV: &str = "RAYX_APP_CONTENT_ROOT";

pub fn build(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    target_name: &str,
    profile: BuildProfile,
    mut args: Vec<String>,
) -> Result<()> {
    let target_triple = desktop_target_triple(target_name, &app.context.host, &mut args)?;
    ensure_empty(&args)?;
    let manifest = desktop_manifest(app, target_name)?;
    build_manifest(
        &manifest,
        &app.target_dir()?,
        features,
        profile,
        target_triple.as_deref(),
    )
}

pub fn run(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    target_name: &str,
    profile: BuildProfile,
    mut args: Vec<String>,
) -> Result<()> {
    let target_triple = desktop_target_triple(target_name, &app.context.host, &mut args)?;
    let app_args = split_app_args(&mut args);
    ensure_empty(&args)?;
    let manifest = desktop_manifest(app, target_name)?;
    let content_root = stage_run_content(app, features, target_name, profile)?;
    let assets_root = content_root.join("assets");
    let mut command = Command::new("cargo");
    command
        .arg("run")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--target-dir")
        .arg(app.target_dir()?)
        .env(RAYX_ASSETS_ROOT_ENV, &assets_root)
        .env(RAYX_APP_CONTENT_ROOT_ENV, &content_root);
    if let Some(target_triple) = target_triple.as_deref() {
        command.args(["--target", target_triple]);
    }
    if let Some(profile_arg) = profile.cargo_arg() {
        command.arg(profile_arg);
    }
    features.apply_to_cargo(&mut command)?;
    if !app_args.is_empty() {
        command.arg("--").args(app_args);
    }
    run_process(&mut command)
}

pub fn pack(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    target_name: &str,
    profile: BuildProfile,
    mut args: Vec<String>,
) -> Result<()> {
    let target_triple = desktop_target_triple(target_name, &app.context.host, &mut args)?;
    ensure_empty(&args)?;
    let manifest = desktop_manifest(app, target_name)?;
    build_manifest(
        &manifest,
        &app.target_dir()?,
        features,
        profile,
        target_triple.as_deref(),
    )?;

    let package_name = package_name_from_manifest(&manifest)?;
    let exe_name = executable_name_for_target(&package_name, target_triple.as_deref());
    let mut binary_path = app.target_dir()?;
    if let Some(target_triple) = target_triple.as_deref() {
        binary_path = binary_path.join(target_triple);
    }
    binary_path = binary_path.join(profile.name()).join(exe_name);
    if !binary_path.is_file() {
        bail!("built binary not found at {}", binary_path.display());
    }

    let out_dir = app.artifact_root()?.join(target_name).join(profile.name());
    fs::create_dir_all(&out_dir)?;
    let destination = out_dir.join(
        binary_path
            .file_name()
            .ok_or_else(|| anyhow!("invalid binary path"))?,
    );
    fs::copy(&binary_path, &destination)?;
    copy_deployment_content(app, features, &out_dir)?;
    println!("Packed {} desktop app at {}", app.slug, out_dir.display());
    Ok(())
}

pub fn build_mobile_crate(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    args: Vec<String>,
) -> Result<()> {
    ensure_empty(&args)?;
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(app.manifest());
    if let Some(profile_arg) = profile.cargo_arg() {
        command.arg(profile_arg);
    }
    features.apply_to_cargo(&mut command)?;
    run_process(&mut command)
}

pub fn build_manifest(
    manifest: &Path,
    target_dir: &Path,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    target: Option<&str>,
) -> Result<()> {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--target-dir")
        .arg(target_dir);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    if let Some(profile_arg) = profile.cargo_arg() {
        command.arg(profile_arg);
    }
    features.apply_to_cargo(&mut command)?;
    run_process(&mut command)
}

/// `rayx app <dir> test host|windows|linux|macos`: the app package's own tests, `cargo test -p
/// <package>` run from the workspace root with the selected features and profile. Arguments after
/// `--` go to the test binary.
pub fn test(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    target_name: &str,
    profile: BuildProfile,
    mut args: Vec<String>,
) -> Result<()> {
    let target_triple = desktop_target_triple(target_name, &app.context.host, &mut args)?;
    let test_args = split_app_args(&mut args);
    ensure_empty(&args)?;
    let package_name = package_name_from_manifest(&desktop_manifest(app, target_name)?)?;
    let mut command = Command::new("cargo");
    command
        .current_dir(app.context.workspace_root())
        .args(["test", "-p", &package_name, "--target-dir"])
        .arg(app.target_dir()?);
    if let Some(target_triple) = target_triple.as_deref() {
        command.args(["--target", target_triple]);
    }
    if let Some(profile_arg) = profile.cargo_arg() {
        command.arg(profile_arg);
    }
    features.apply_to_cargo(&mut command)?;
    if !test_args.is_empty() {
        command.arg("--").args(test_args);
    }
    run_process(&mut command)
}

fn desktop_manifest(app: &AppDescriptor, target_name: &str) -> Result<PathBuf> {
    match target_name {
        "host" | "windows" | "linux" | "macos" => Ok(app.manifest()),
        other => bail!("unknown desktop target: {other}"),
    }
}

/// The explicit `--target`, else the native triple of the host for `windows`, `linux` and `macos`
/// (an ARM64 Windows machine builds `aarch64-pc-windows-msvc`).
pub fn desktop_target_triple(
    target_name: &str,
    host: &HostFacts,
    args: &mut Vec<String>,
) -> Result<Option<String>> {
    let explicit = take_flag_value(args, "--target")?;
    let arch = if host.arch == Arch::Arm64 {
        "aarch64"
    } else {
        "x86_64"
    };
    Ok(match (target_name, explicit) {
        (_, Some(target)) => Some(target),
        ("windows", None) => Some(format!("{arch}-pc-windows-msvc")),
        ("linux", None) => Some(format!("{arch}-unknown-linux-gnu")),
        ("macos", None) => Some(format!("{arch}-apple-darwin")),
        _ => None,
    })
}

fn split_app_args(args: &mut Vec<String>) -> Vec<String> {
    let Some(separator) = args.iter().position(|arg| arg == "--") else {
        return Vec::new();
    };
    args.drain(separator..).skip(1).collect()
}

fn executable_name_for_target(package_name: &str, target_triple: Option<&str>) -> String {
    let base = package_name.to_string();
    if target_triple
        .map(|target| target.contains("windows"))
        .unwrap_or_else(|| cfg!(target_os = "windows"))
    {
        format!("{base}.exe")
    } else {
        base
    }
}

fn copy_deployment_content(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    out_dir: &Path,
) -> Result<()> {
    assets::package_app_assets_with_selection(app, features, &out_dir.join("assets"))?;
    assets::package_app_content(app, out_dir)?;
    Ok(())
}

fn stage_run_content(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    target_name: &str,
    profile: BuildProfile,
) -> Result<PathBuf> {
    let destination = app
        .temp_root()?
        .join(target_name)
        .join(profile.name())
        .join("content");
    let assets_root = destination.join("assets");
    assets::package_app_assets_with_selection(app, features, &assets_root)?;
    assets::package_app_content(app, &destination)?;
    validate_staged_assets_root(&assets_root)?;
    Ok(destination)
}

fn validate_staged_assets_root(root: &Path) -> Result<()> {
    if !root.is_absolute() {
        bail!(
            "staged desktop asset root must be absolute: {}",
            root.display()
        );
    }
    if !root.is_dir() {
        bail!("staged desktop asset root is missing: {}", root.display());
    }
    for manifest in ["index.json", "index.entries.json"] {
        let path = root.join(manifest);
        if !path.is_file() {
            bail!(
                "staged desktop asset root {} is missing {manifest}",
                root.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn desktop_app_content_staging_uses_deployment_root_and_preserves_sources()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!("rayx-desktop-package-assets-{nonce}"));
        let app_root = root.join("app");
        let theme_root = root.join("theme");
        fs::create_dir_all(app_root.join("assets/icons"))?;
        fs::create_dir_all(theme_root.join("assets/icons"))?;
        fs::write(app_root.join("fixture.rs"), "pub fn app() {}")?;
        fs::write(app_root.join("app_settings.json"), "{\"desktop\":true}")?;
        fs::write(theme_root.join("fixture.rs"), "pub fn theme() {}")?;
        fs::write(app_root.join("assets/icons/app.svg"), "<svg id=\"app\"/>")?;
        fs::write(
            theme_root.join("assets/icons/theme.svg"),
            "<svg id=\"theme\"/>",
        )?;
        fs::write(
            app_root.join("Cargo.toml"),
            format!(
                r#"[package]
name = "desktop-fixture-{nonce}"
version = "0.1.0"
edition = "2024"

[lib]
path = "fixture.rs"

[dependencies]
desktop-fixture-theme = {{ path = "../theme" }}
"#
            ),
        )?;
        fs::write(
            theme_root.join("Cargo.toml"),
            r#"[package]
name = "desktop-fixture-theme"
version = "0.1.0"
edition = "2024"

[package.metadata.rayx.assets]
manifest = "rayx.assets.toml"

[lib]
path = "fixture.rs"
"#,
        )?;
        fs::write(
            app_root.join("rayx.assets.toml"),
            r#"version = 1

[package_assets]
mode = "explicit"
include = ["desktop-fixture-theme"]

[assets]
roots = ["assets"]

[app_content]
files = ["app_settings.json"]
"#,
        )?;
        fs::write(
            theme_root.join("rayx.assets.toml"),
            r#"version = 1

[asset_package]
id = "desktop-fixture-theme"
kind = "theme"
namespace = ""

[assets]
roots = [{ path = "assets/icons", mount = "icons" }]
"#,
        )?;

        let app_source = fs::read(app_root.join("assets/icons/app.svg"))?;
        let theme_source = fs::read(theme_root.join("assets/icons/theme.svg"))?;
        let app =
            crate::app::test_support::resolve_str(app_root.to_str().expect("UTF-8 fixture path"))?;
        let content_root = stage_run_content(
            &app,
            &AppFeatureSelection::default(),
            "host",
            BuildProfile::Debug,
        )?;
        let staged = content_root.join("assets");

        assert!(staged.starts_with(app.temp_root()?));
        assert!(staged.join("icons/app.svg").is_file());
        assert!(staged.join("icons/theme.svg").is_file());
        assert_eq!(
            fs::read_to_string(content_root.join("app_settings.json"))?,
            "{\"desktop\":true}"
        );
        assert_eq!(fs::read(app_root.join("assets/icons/app.svg"))?, app_source);
        assert_eq!(
            fs::read(theme_root.join("assets/icons/theme.svg"))?,
            theme_source
        );
        let sidecar: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(staged.join("index.entries.json"))?)?;
        assert!(
            sidecar["dict"]["packageId"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == "desktop-fixture-theme"))
        );

        fs::remove_dir_all(app.temp_root()?).ok();
        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn desktop_package_asset_staging_rejects_incomplete_roots()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "rayx-desktop-incomplete-assets-{}",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir_all(&root)?;
        fs::write(root.join("index.json"), "{}")?;

        let error = validate_staged_assets_root(&root).expect_err("sidecar must be required");
        assert!(error.to_string().contains("index.entries.json"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }
}
