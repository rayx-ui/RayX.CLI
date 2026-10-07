use crate::app::args::{ensure_empty, take_bool_flag};
use crate::app::assets;
use crate::app::process::run as run_process;
use crate::app::{AppDescriptor, AppFeatureSelection, BuildProfile};
use anyhow::{Context as _, Result, anyhow, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn build(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    args: Vec<String>,
) -> Result<()> {
    let simulator = ios_simulator_arg(args)?;
    build_with_simulator(app, features, profile, simulator)
}

pub fn run(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    args: Vec<String>,
) -> Result<()> {
    let simulator = ios_simulator_arg(args)?;
    build_with_simulator(app, features, profile, simulator)?;
    if !simulator {
        println!(
            "iOS device build completed. Install/launch requires a signed device target; use Xcode or add signing flags before store/device publishing."
        );
        return Ok(());
    }

    let app_path = ios_app_bundle_path(app, profile, true)?;
    if !app_path.is_dir() {
        bail!(
            "iOS simulator app bundle not found at {}",
            app_path.display()
        );
    }

    let simulator_id = first_available_ios_simulator()?;
    run_process(Command::new("xcrun").args(["simctl", "boot", &simulator_id]))?;
    run_process(Command::new("open").args(["-a", "Simulator"]))?;
    run_process(
        Command::new("xcrun").args([
            "simctl",
            "install",
            &simulator_id,
            app_path
                .to_str()
                .ok_or_else(|| anyhow!("invalid iOS app path"))?,
        ]),
    )?;
    run_process(Command::new("xcrun").args([
        "simctl",
        "launch",
        &simulator_id,
        app.ios_bundle_id()?,
    ]))
}

pub fn pack(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    args: Vec<String>,
) -> Result<()> {
    ensure_empty(&args)?;
    build_with_simulator(app, features, profile, true)?;
    let out_dir = app.artifact_root()?.join("ios").join(profile.name());
    fs::create_dir_all(&out_dir)?;
    println!(
        "iOS build staged under {}",
        app.ios_dir().join("build").display()
    );
    println!("Pack output directory prepared at {}", out_dir.display());
    Ok(())
}

fn build_with_simulator(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    simulator: bool,
) -> Result<()> {
    ensure_macos("iOS app builds")?;
    let ios_dir = app.require_ios_dir()?;
    let rust_manifest = app.ios_rust_manifest(features)?;
    let ios_assets = ios_asset_staging_dir(app)?;
    stage_ios_package_content(app, features, &ios_assets)?;
    run_process(
        Command::new("xcodegen")
            .arg("generate")
            .current_dir(&ios_dir),
    )?;

    let destination = if simulator {
        "generic/platform=iOS Simulator"
    } else {
        "generic/platform=iOS"
    };
    run_process(
        Command::new("xcodebuild")
            .args([
                "-project",
                &app.ios_project()?,
                "-scheme",
                app.ios_scheme()?,
                "-configuration",
                profile.xcode_configuration(),
                "-destination",
                destination,
                "-derivedDataPath",
                "build",
                "build",
            ])
            .env("RAYX_IOS_RUST_MANIFEST", &rust_manifest)
            .current_dir(&ios_dir),
    )?;

    let app_bundle = ios_app_bundle_path(app, profile, simulator)?;
    verify_ios_app_bundle_assets(&ios_assets, &app_bundle)
}

fn ios_asset_staging_dir(app: &AppDescriptor) -> Result<PathBuf> {
    Ok(app.require_ios_dir()?.join("assets"))
}

fn stage_ios_package_content(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    destination: &Path,
) -> Result<()> {
    assets::package_app_assets_with_selection(app, features, destination).with_context(|| {
        format!(
            "staging merged iOS app/package assets at {}",
            destination.display()
        )
    })?;
    assets::package_app_content(app, destination).with_context(|| {
        format!(
            "staging declared iOS app content at {}",
            destination.display()
        )
    })?;
    Ok(())
}

fn ios_app_bundle_path(
    app: &AppDescriptor,
    profile: BuildProfile,
    simulator: bool,
) -> Result<PathBuf> {
    let sdk = if simulator {
        "iphonesimulator"
    } else {
        "iphoneos"
    };
    Ok(app
        .require_ios_dir()?
        .join("build")
        .join("Build")
        .join("Products")
        .join(format!("{}-{sdk}", profile.xcode_configuration()))
        .join(format!("{}.app", app.ios_app_name()?)))
}

fn verify_ios_app_bundle_assets(staged_assets: &Path, app_bundle: &Path) -> Result<()> {
    let bundled_assets = app_bundle.join("assets");
    if !bundled_assets.is_dir() {
        bail!(
            "iOS app bundle is missing packaged assets directory {}",
            bundled_assets.display()
        );
    }

    let staged_files = relative_files(staged_assets)?;
    let bundled_files = relative_files(&bundled_assets)?;
    if staged_files != bundled_files {
        let missing = staged_files
            .iter()
            .filter(|path| bundled_files.binary_search(path).is_err())
            .map(|path| logical_path_display(path))
            .collect::<Vec<_>>();
        let unexpected = bundled_files
            .iter()
            .filter(|path| staged_files.binary_search(path).is_err())
            .map(|path| logical_path_display(path))
            .collect::<Vec<_>>();
        bail!(
            "iOS app bundle asset inventory differs from staging (missing: [{}], unexpected: [{}])",
            missing.join(", "),
            unexpected.join(", ")
        );
    }

    for relative_path in &staged_files {
        let staged_path = staged_assets.join(relative_path);
        let bundled_path = bundled_assets.join(relative_path);
        if fs::read(&staged_path)
            .with_context(|| format!("reading staged iOS asset {}", staged_path.display()))?
            != fs::read(&bundled_path)
                .with_context(|| format!("reading bundled iOS asset {}", bundled_path.display()))?
        {
            bail!(
                "iOS app bundle asset {} does not match the staged bytes",
                logical_path_display(relative_path)
            );
        }
    }

    println!(
        "Verified {} packaged iOS assets in {}",
        staged_files.len(),
        bundled_assets.display()
    );
    Ok(())
}

fn logical_path_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn relative_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.is_dir() {
        bail!("asset tree is missing: {}", root.display());
    }

    fn visit(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(directory)
            .with_context(|| format!("reading asset directory {}", directory.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                bail!("asset tree contains a symbolic link: {}", path.display());
            }
            if file_type.is_dir() {
                visit(root, &path, files)?;
            } else if file_type.is_file() {
                files.push(path.strip_prefix(root)?.to_path_buf());
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn ios_simulator_arg(mut args: Vec<String>) -> Result<bool> {
    let simulator = take_bool_flag(&mut args, "--simulator");
    let device = take_bool_flag(&mut args, "--device");
    ensure_empty(&args)?;
    if simulator && device {
        bail!("Use only one iOS destination selector: --simulator or --device");
    }
    Ok(simulator)
}

fn first_available_ios_simulator() -> Result<String> {
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available"])
        .output()
        .map_err(anyhow::Error::from)?;
    if !output.status.success() {
        bail!("xcrun simctl list devices failed");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if !line.contains("iPhone") {
            continue;
        }
        if let Some(start) = line.rfind('(')
            && let Some(end) = line[start + 1..].find(')')
        {
            return Ok(line[start + 1..start + 1 + end].to_string());
        }
    }
    bail!("no available iOS simulator found")
}

fn ensure_macos(task: &str) -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        bail!("{task} require macOS with Xcode/XcodeGen")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::UNIX_EPOCH;

    #[test]
    fn ios_asset_staging_dir_targets_platform_resource_assets()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_app_root("xtask-ios-assets")?;
        fs::create_dir_all(root.join("platform/ios"))?;
        fs::write(root.join("Cargo.toml"), cargo_manifest("xtask-ios-assets"))?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;

        assert_eq!(
            ios_asset_staging_dir(&app)?,
            app.root.join("platform").join("ios").join("assets")
        );

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn ios_package_asset_staging_includes_theme_provenance_and_removes_stale_files()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask should be inside the repository root")
            .to_path_buf();
        let app_root = repository_root.join("apps/lab");
        let app = AppDescriptor::resolve(app_root.to_str().expect("utf-8 app path"))?;
        let staging_root = temp_app_root("xtask-ios-package-assets")?;
        let destination = staging_root.join("platform/ios/assets");
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("stale.txt"), "stale")?;

        stage_ios_package_content(&app, &AppFeatureSelection::default(), &destination)?;

        assert!(destination.join("icons/bot.svg").is_file());
        assert!(destination.join("index.json").is_file());
        assert!(destination.join("index.entries.json").is_file());
        assert!(!destination.join("stale.txt").exists());
        let sidecar: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(destination.join("index.entries.json"))?)?;
        assert!(
            sidecar["dict"]["packageId"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == "rayx-components-core")),
            "iOS sidecar should retain component-core package provenance: {sidecar}"
        );

        fs::remove_dir_all(staging_root).ok();
        Ok(())
    }

    #[test]
    fn ios_app_content_is_in_bundled_resource_folder_outside_catalog()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_app_root("xtask-ios-app-content")?;
        fs::create_dir_all(root.join("assets"))?;
        fs::write(
            root.join("Cargo.toml"),
            format!(
                "{}\n[lib]\npath = \"fixture.rs\"\n",
                cargo_manifest("xtask-ios-app-content")
            ),
        )?;
        fs::write(root.join("fixture.rs"), "pub fn fixture() {}")?;
        fs::write(root.join("assets/icon.svg"), "<svg/>")?;
        fs::write(root.join("app_settings.json"), "{\"ios\":true}")?;
        fs::write(
            root.join("rayx.assets.toml"),
            "version = 1\n[assets]\nroots = [\"assets\"]\n[app_content]\nfiles = [\"app_settings.json\"]\n",
        )?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let resource_root = root.join("platform/ios/assets");
        stage_ios_package_content(&app, &AppFeatureSelection::default(), &resource_root)?;

        assert_eq!(
            fs::read_to_string(resource_root.join("app_settings.json"))?,
            "{\"ios\":true}"
        );
        assert!(resource_root.join("index.json").is_file());
        assert!(!fs::read_to_string(resource_root.join("index.json"))?.contains("app_settings"));
        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn ios_projects_package_generated_assets_as_folder_resources()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask should be inside the repository root")
            .to_path_buf();
        for project in [
            "apps/lab/platform/ios/project.yml",
            "apps/examples_mobile/platform/ios/project.yml",
        ] {
            let source = fs::read_to_string(repository_root.join(project))?.replace("\r\n", "\n");
            assert!(
                source.contains(
                    "- path: assets\n        type: folder\n        buildPhase: resources"
                ),
                "{project} should copy the generated public asset tree into the app bundle"
            );
        }
        let ignore = fs::read_to_string(repository_root.join(".gitignore"))?;
        assert!(
            ignore.contains("**/platform/ios/assets/"),
            "generated iOS resource assets should remain untracked"
        );
        Ok(())
    }

    #[test]
    fn ios_built_app_bundle_must_contain_the_exact_staged_asset_tree()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_app_root("xtask-ios-bundle-assets")?;
        let staged = root.join("staged");
        let app_bundle = root.join("RayXTest.app");
        fs::create_dir_all(staged.join("icons"))?;
        fs::write(staged.join("index.json"), r#"{"paths":["icons/bot.svg"]}"#)?;
        fs::write(staged.join("index.entries.json"), r#"{"entries":[]}"#)?;
        fs::write(staged.join("icons/bot.svg"), "<svg>expected</svg>")?;

        let missing = verify_ios_app_bundle_assets(&staged, &app_bundle)
            .expect_err("a missing app bundle asset directory must fail");
        assert!(missing.to_string().contains("assets"));

        let bundled = app_bundle.join("assets");
        fs::create_dir_all(bundled.join("icons"))?;
        fs::copy(staged.join("index.json"), bundled.join("index.json"))?;
        fs::copy(
            staged.join("index.entries.json"),
            bundled.join("index.entries.json"),
        )?;
        fs::write(bundled.join("icons/bot.svg"), "<svg>wrong</svg>")?;
        let mismatched = verify_ios_app_bundle_assets(&staged, &app_bundle)
            .expect_err("mismatched packaged bytes must fail");
        assert!(mismatched.to_string().contains("icons/bot.svg"));

        fs::copy(staged.join("icons/bot.svg"), bundled.join("icons/bot.svg"))?;
        verify_ios_app_bundle_assets(&staged, &app_bundle)?;

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    fn cargo_manifest(name: &str) -> String {
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
"#
        )
    }

    fn temp_app_root(name: &str) -> std::io::Result<PathBuf> {
        let root = std::env::temp_dir().join(format!(
            "rayx-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;
        Ok(root)
    }
}
