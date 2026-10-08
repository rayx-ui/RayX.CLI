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

    let simulator = ios_simulator()?;
    let simulator_id = simulator.id;
    if !simulator.booted {
        run_process(Command::new("xcrun").args(["simctl", "boot", &simulator_id]))?;
    }
    // Installing into a device that is still booting fails; wait until it is ready.
    run_process(Command::new("xcrun").args(["simctl", "bootstatus", &simulator_id]))?;
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
    let simulator = pack_destination(args)?;
    build_with_simulator(app, features, profile, simulator)?;
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

/// `pack ios` accepts the same destination selectors as `build ios`; without one it packs the
/// Simulator build, as it always has.
fn pack_destination(args: Vec<String>) -> Result<bool> {
    let device = args.iter().any(|arg| arg == "--device");
    let simulator = args.iter().any(|arg| arg == "--simulator");
    if device && simulator {
        bail!("Use only one iOS destination selector: --simulator or --device");
    }
    ios_simulator_arg(args)?;
    Ok(!device)
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

/// One iPhone line of `xcrun simctl list devices available`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Simulator {
    id: String,
    booted: bool,
}

/// The iPhone simulators of `xcrun simctl list devices`, in the order listed (runtimes oldest
/// first). A device line ends with its id and its state: `    iPhone 17 Pro (<UDID>) (Shutdown)`.
fn parse_iphone_simulators(listing: &str) -> Vec<Simulator> {
    listing
        .lines()
        .filter(|line| line.trim_start().starts_with("iPhone"))
        .filter_map(|line| {
            let groups: Vec<&str> = line
                .split('(')
                .skip(1)
                .filter_map(|part| part.split(')').next())
                .collect();
            let [.., id, state] = groups.as_slice() else {
                return None;
            };
            let looks_like_id = id.len() == 36 && id.matches('-').count() == 4;
            looks_like_id.then(|| Simulator {
                id: (*id).to_string(),
                booted: *state == "Booted",
            })
        })
        .collect()
}

/// A booted iPhone when there is one, else the last available one (the newest runtime).
fn pick_simulator(simulators: &[Simulator]) -> Option<&Simulator> {
    simulators
        .iter()
        .find(|simulator| simulator.booted)
        .or_else(|| simulators.last())
}

fn ios_simulator() -> Result<Simulator> {
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available"])
        .output()
        .map_err(anyhow::Error::from)?;
    if !output.status.success() {
        bail!("xcrun simctl list devices failed");
    }
    let simulators = parse_iphone_simulators(&String::from_utf8_lossy(&output.stdout));
    pick_simulator(&simulators)
        .cloned()
        .ok_or_else(|| anyhow!("no available iOS simulator found: run `rayx setup --ios`"))
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

    const SIMCTL_LIST: &str = "== Devices ==
-- iOS 26.4 --
    iPhone 17 Pro (683ABED6-E61B-48D2-B27C-1B618A55076A) (Shutdown) 
    iPad Pro 13-inch (M5) (4D16A9ED-7D1A-4A4E-B6D8-1E1E21CFEDC8) (Shutdown) 
-- iOS 26.5 --
    iPhone 17 Pro (770A724B-DDCF-441A-99B5-5B173009D4B6) (Shutdown) 
    iPhone Air (EC9F9598-F7B7-4060-8F32-F21BEE3CA827) (Booted) 
-- tvOS 26.4 --
    Apple TV (11111111-2222-3333-4444-555555555555) (Shutdown) 
";

    #[test]
    fn simulator_ids_come_from_the_id_group_not_the_state() {
        let simulators = parse_iphone_simulators(SIMCTL_LIST);
        assert_eq!(
            simulators.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            [
                "683ABED6-E61B-48D2-B27C-1B618A55076A",
                "770A724B-DDCF-441A-99B5-5B173009D4B6",
                "EC9F9598-F7B7-4060-8F32-F21BEE3CA827"
            ],
            "iPhones only, never the word `Shutdown`"
        );
        assert_eq!(
            pick_simulator(&simulators).map(|s| s.id.as_str()),
            Some("EC9F9598-F7B7-4060-8F32-F21BEE3CA827"),
            "a booted iPhone is reused"
        );
    }

    #[test]
    fn without_a_booted_iphone_the_newest_listed_one_is_chosen() {
        let simulators = parse_iphone_simulators(&SIMCTL_LIST.replace("(Booted)", "(Shutdown)"));
        assert_eq!(
            pick_simulator(&simulators).map(|s| s.id.as_str()),
            Some("EC9F9598-F7B7-4060-8F32-F21BEE3CA827")
        );
        assert!(parse_iphone_simulators("== Devices ==\n").is_empty());
        assert!(pick_simulator(&[]).is_none());
    }

    #[test]
    fn pack_takes_the_build_destination_flags_and_defaults_to_the_simulator() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(pack_destination(args(&[])).unwrap());
        assert!(pack_destination(args(&["--simulator"])).unwrap());
        assert!(!pack_destination(args(&["--device"])).unwrap());
        assert!(pack_destination(args(&["--simulator", "--device"])).is_err());
        assert!(pack_destination(args(&["--bogus"])).is_err());
    }

    #[test]
    fn ios_asset_staging_dir_targets_platform_resource_assets()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_app_root("xtask-ios-assets")?;
        fs::create_dir_all(root.join("platform/ios"))?;
        fs::write(root.join("Cargo.toml"), cargo_manifest("xtask-ios-assets"))?;
        let app = crate::app::test_support::resolve_str(root.to_str().expect("utf-8 path"))?;

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
        let workspace = crate::app::test_support::FixtureWorkspace::new();
        let app = workspace.app();
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
                .is_some_and(|ids| ids.iter().any(|id| id == "demo-components")),
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
        let app = crate::app::test_support::resolve_str(root.to_str().expect("utf-8 path"))?;
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
