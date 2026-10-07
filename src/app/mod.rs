//! `rayx app`: build, run, pack, publish, deploy and test any app that uses RayX, and manage its
//! assets. The grammar and behavior come from RayX's xtask `app` command (moved from RayX commit
//! d69d375e).

mod android;
mod args;
mod assets;
mod descriptor;
mod desktop;
mod fs_util;
mod ios;
mod node_tools;
mod package_assets;
mod process;
mod wasm;
mod workspace_paths;

pub use descriptor::*;

use anyhow::{Result, bail};
use args::take_profile;

/// Runs `rayx app` with the arguments after `app`.
pub fn run(args: Vec<String>) -> Result<()> {
    app_command(args)
}

fn app_command(mut args: Vec<String>) -> Result<()> {
    if args.len() < 2 {
        print_help();
        bail!("app command requires an app directory and action");
    }

    let app_dir = args.remove(0);
    let action = args.remove(0);
    let app = AppDescriptor::resolve(&app_dir)?;
    if action == "assets" {
        let features = AppFeatureSelection::take_from_args(&mut args)?;
        features.validate_for(&app.manifest())?;
        return assets::run(&app, &features, args);
    }
    if args.is_empty() {
        print_help();
        bail!("app action {action} requires a target");
    }
    let target = normalize_target(&args.remove(0));
    let profile = take_profile(&mut args)?;
    let features = AppFeatureSelection::take_from_args(&mut args)?;
    features.validate_for(&app.manifest())?;
    validate_diagnostic_feature_policy(app.diagnostic_harness, &action, &features)?;

    match action.as_str() {
        "build" => build_target(&app, &features, profile, &target, args),
        "run" => run_target(&app, &features, profile, &target, args),
        "pack" => pack_target(&app, &features, profile, &target, args),
        "publish" => publish_target(&app, &features, profile, &target, args),
        "deploy" => deploy_target(&app, &features, profile, &target, args),
        "test" => test_target(&app, &features, profile, &target, args),
        other => bail!("unknown app action: {other}"),
    }
}

fn build_target(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    target: &str,
    args: Vec<String>,
) -> Result<()> {
    match target {
        "host" | "windows" | "linux" | "macos" => {
            desktop::build(app, features, target, profile, args)
        }
        "mobile" => desktop::build_mobile_crate(app, features, profile, args),
        "ios" => ios::build(app, features, profile, args),
        "android" => android::build(app, features, profile, args),
        "wasm" => wasm::build(app, features, profile, args).map(|_| ()),
        other => bail!("unknown app build target: {other}"),
    }
}

fn run_target(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    target: &str,
    args: Vec<String>,
) -> Result<()> {
    match target {
        "host" | "windows" | "linux" | "macos" => {
            desktop::run(app, features, target, profile, args)
        }
        "mobile" => desktop::build_mobile_crate(app, features, profile, args),
        "ios" => ios::run(app, features, profile, args),
        "android" => android::run(app, features, profile, args),
        "wasm" => wasm::run(app, features, profile, args),
        other => bail!("unknown app run target: {other}"),
    }
}

fn pack_target(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    target: &str,
    args: Vec<String>,
) -> Result<()> {
    match target {
        "host" | "windows" | "linux" | "macos" => {
            desktop::pack(app, features, target, profile, args)
        }
        "mobile" => desktop::build_mobile_crate(app, features, profile, args),
        "ios" => ios::pack(app, features, profile, args),
        "android" => android::pack(app, features, profile, args),
        "wasm" => wasm::build(app, features, profile, args).map(|_| ()),
        other => bail!("unknown app pack target: {other}"),
    }
}

fn publish_target(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    target: &str,
    args: Vec<String>,
) -> Result<()> {
    pack_target(app, features, profile, target, args)?;
    println!(
        "Publish staging is complete for {} {target}; store upload/signing is intentionally not automated yet.",
        app.slug
    );
    Ok(())
}

fn deploy_target(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    target: &str,
    args: Vec<String>,
) -> Result<()> {
    match target {
        "android" => android::deploy(app, features, profile, args),
        other => bail!("app deploy is currently supported only for android, not {other}"),
    }
}

fn test_target(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    target: &str,
    args: Vec<String>,
) -> Result<()> {
    match target {
        "wasm" => wasm::test(app, features, profile, args),
        other => bail!("app test is currently supported only for wasm, not {other}"),
    }
}

fn normalize_target(target: &str) -> String {
    match target {
        "web" => "wasm".to_string(),
        other => other.to_string(),
    }
}

fn print_help() {
    println!("xtask commands:");
    println!("  setup-android");
    println!("  setup-wasm");
    println!("  code-themes import-lumis [--output <directory>] [--check]");
    println!("  theme generate [--check]");
    println!("  component-index --analyze <Type,...> [--designed <Type,...>] [--out <file>]");
    println!("  fmt [--check]");
    println!(
        "  app <app-dir> <build|run|pack|publish|deploy|test> <host|windows|linux|macos|ios|android|mobile|wasm|web> [--debug|--development] [--features <list>] [--all-features|--no-default-features]"
    );
    println!(
        "  app <app-dir> assets <generate|validate|list> [--output <path>] [--manifest <path>] [--features <list>] [--all-features|--no-default-features]"
    );
    println!("  app apps/lab run host");
    println!("  app apps/lab run wasm --port 7878");
    println!("  app apps/test_suite test wasm --headed --webgpu");
    println!(
        "  app apps/test_suite test wasm --headed --webgpu --playwright-test test_suite/rayx-component-diagnostics.spec.ts"
    );
    println!("  app apps/lab build wasm --no-default-features --features code-view-rust-dracula");
    println!("  app apps/lab run android --android-paired");
    println!(
        "  Android device knobs: --android-device <serial> --android-connect <host:port> --android-paired"
    );
    println!("  Android deploy knobs: --android-apk <path> --no-launch");
    println!(
        "  Android diagnostics knobs: --android-devkit --android-devkit-port <port> --android-devkit-registry-dir <path>"
    );
    println!(
        "  Android size knobs: --size-optimized --android-native-opt-level <0|1|2|3|s|z> --android-native-lto <off|thin|fat>"
    );
    println!(
        "  Android packaging knobs: --android-minify <bool> --android-shrink-resources <bool> --android-crunch-pngs <bool> --android-compress-native-libs <bool>"
    );
}

fn validate_diagnostic_feature_policy(
    diagnostic_harness: bool,
    action: &str,
    features: &AppFeatureSelection,
) -> Result<()> {
    if !features
        .normalized_features()
        .iter()
        .any(|feature| feature == "rayx_diagnostics")
    {
        return Ok(());
    }
    if !diagnostic_harness {
        bail!("rayx_diagnostics is reserved for apps marked as diagnostic harnesses");
    }
    if matches!(action, "pack" | "publish" | "deploy") {
        bail!("rayx_diagnostics cannot be used for packaging, publishing, or deployment");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppFeatureSelection;

    #[test]
    fn diagnostic_feature_policy_rejects_production_and_packaging_commands() {
        let diagnostics = AppFeatureSelection {
            features: vec!["rayx_diagnostics".to_string()],
            ..AppFeatureSelection::default()
        };

        assert!(validate_diagnostic_feature_policy(false, "build", &diagnostics).is_err());
        assert!(validate_diagnostic_feature_policy(true, "pack", &diagnostics).is_err());
        assert!(validate_diagnostic_feature_policy(true, "publish", &diagnostics).is_err());
        assert!(validate_diagnostic_feature_policy(true, "deploy", &diagnostics).is_err());
        assert!(validate_diagnostic_feature_policy(true, "test", &diagnostics).is_ok());
    }
}
