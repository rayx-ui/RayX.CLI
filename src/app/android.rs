use crate::app::args::{ensure_empty, take_bool_flag, take_bool_flag_value, take_flag_value};
use crate::app::assets;
use crate::app::fs_util::{
    absolutize_path, collect_files_named, collect_files_with_extension, command_path, reset_dir,
};
use crate::app::process::{command_succeeds, run as run_process};
use crate::app::{AppDescriptor, AppFeatureSelection, BuildProfile};
use crate::project::Pins;
use anyhow::{Context, Result, anyhow, bail};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_ANDROID_ABI: &str = "arm64-v8a";
const ANDROID_DEVKIT_PORT: u16 = 47123;
const DEVKIT_PROTOCOL_VERSION: &str = "0.1.0";

#[derive(Clone, Debug, Default)]
struct AndroidBuildOptions {
    size_optimized: bool,
    native_opt_level: Option<String>,
    native_lto: Option<String>,
    native_codegen_units: Option<String>,
    native_strip: Option<String>,
    native_panic: Option<String>,
    minify: Option<bool>,
    shrink_resources: Option<bool>,
    crunch_pngs: Option<bool>,
    compress_native_libs: Option<bool>,
    device_serial: Option<String>,
    connect_address: Option<String>,
    paired_device: bool,
    apk_path: Option<PathBuf>,
    no_launch: bool,
    devkit: bool,
    devkit_port: Option<u16>,
    devkit_registry_dir: Option<PathBuf>,
}

impl AndroidBuildOptions {
    fn requires_release(&self) -> bool {
        self.size_optimized
            || self.native_opt_level.is_some()
            || self.native_lto.is_some()
            || self.native_codegen_units.is_some()
            || self.native_strip.is_some()
            || self.native_panic.is_some()
            || self.minify.is_some()
            || self.shrink_resources.is_some()
            || self.crunch_pngs.is_some()
            || self.compress_native_libs.is_some()
    }

    fn devkit_port(&self) -> u16 {
        self.devkit_port.unwrap_or(ANDROID_DEVKIT_PORT)
    }
}

pub struct AndroidToolchain {
    sdk_dir: PathBuf,
    ndk_dir: PathBuf,
    adb: PathBuf,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct AndroidBuildTarget {
    abi: &'static str,
    rust_target: &'static str,
}

pub fn build(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    mut args: Vec<String>,
) -> Result<()> {
    let options = take_android_build_options(&mut args, profile)?;
    ensure_empty(&args)?;
    build_android_app(app, features, profile, options)
}

pub fn run(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    args: Vec<String>,
) -> Result<()> {
    deploy(app, features, profile, args)
}

pub fn deploy(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    mut args: Vec<String>,
) -> Result<()> {
    let options = take_android_build_options(&mut args, profile)?;
    ensure_empty(&args)?;
    deploy_android_app(app, features, profile, options)
}

pub fn pack(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    mut args: Vec<String>,
) -> Result<()> {
    let options = take_android_build_options(&mut args, profile)?;
    ensure_empty(&args)?;
    let toolchain = setup_android_machine(&app.context.pins)?;
    let targets = select_android_build_targets(&toolchain, &options)?;
    build_android_app_with_toolchain(app, features, profile, &options, &toolchain, &targets)?;
    let apk_path = android_apk_path(app, profile)?;
    let out_dir = app.artifact_root()?.join("android").join(profile.name());
    fs::create_dir_all(&out_dir)?;
    let destination = out_dir.join(
        apk_path
            .file_name()
            .ok_or_else(|| anyhow!("invalid Android APK path"))?,
    );
    fs::copy(&apk_path, &destination)?;
    println!("Packed Android APK at {}", destination.display());
    Ok(())
}

pub fn setup_android_machine(pins: &Pins) -> Result<AndroidToolchain> {
    ensure_cargo_ndk()?;

    let sdk_dir = find_android_sdk()?;
    ensure_android_sdk_path(
        &sdk_dir,
        Path::new("platform-tools").join(adb_file_name()),
        "platform-tools",
    )?;
    ensure_android_sdk_path(
        &sdk_dir,
        Path::new("platforms")
            .join(&pins.android_platform.value)
            .join("android.jar"),
        &format!("platforms;{}", pins.android_platform.value),
    )?;

    let ndk_dir = match find_android_ndk(&sdk_dir) {
        Some(ndk_dir) => ndk_dir,
        None => {
            install_android_sdk_package(&sdk_dir, &format!("ndk;{}", pins.android_ndk.value))?;
            find_android_ndk(&sdk_dir)
                .ok_or_else(|| anyhow!("Android NDK was not found after sdkmanager install"))?
        }
    };
    let adb = find_android_adb(&sdk_dir)?;

    println!("Android SDK: {}", sdk_dir.display());
    println!("Android NDK: {}", ndk_dir.display());
    println!("Android adb: {}", adb.display());

    Ok(AndroidToolchain {
        sdk_dir,
        ndk_dir,
        adb,
    })
}

fn take_android_build_options(
    args: &mut Vec<String>,
    profile: BuildProfile,
) -> Result<AndroidBuildOptions> {
    let android_paired = take_bool_flag(args, "--android-paired");
    let android_wireless = take_bool_flag(args, "--android-wireless");
    let mut options = AndroidBuildOptions {
        size_optimized: take_bool_flag(args, "--size-optimized"),
        native_opt_level: take_flag_value(args, "--android-native-opt-level")?,
        native_lto: take_flag_value(args, "--android-native-lto")?,
        native_codegen_units: take_flag_value(args, "--android-native-codegen-units")?,
        native_strip: take_flag_value(args, "--android-native-strip")?,
        native_panic: take_flag_value(args, "--android-native-panic")?,
        minify: take_bool_flag_value(args, "--android-minify")?,
        shrink_resources: take_bool_flag_value(args, "--android-shrink-resources")?,
        crunch_pngs: take_bool_flag_value(args, "--android-crunch-pngs")?,
        compress_native_libs: take_bool_flag_value(args, "--android-compress-native-libs")?,
        device_serial: take_flag_value(args, "--android-device")?,
        connect_address: take_flag_value(args, "--android-connect")?,
        paired_device: android_paired || android_wireless,
        apk_path: take_flag_value(args, "--android-apk")?.map(PathBuf::from),
        no_launch: take_bool_flag(args, "--no-launch"),
        devkit: take_bool_flag(args, "--android-devkit"),
        devkit_port: take_flag_value(args, "--android-devkit-port")?
            .map(|value| parse_android_port("--android-devkit-port", &value))
            .transpose()?,
        devkit_registry_dir: take_flag_value(args, "--android-devkit-registry-dir")?
            .map(PathBuf::from),
    };
    if options.device_serial.is_some()
        && (options.connect_address.is_some() || options.paired_device)
    {
        bail!(
            "Use only one Android device selector: --android-device, --android-connect, or --android-paired"
        );
    }
    if options.connect_address.is_some() && options.paired_device {
        bail!("Use only one Android device selector: --android-connect or --android-paired");
    }
    if options.requires_release() && !profile.is_release() {
        bail!("Android size/package options require release mode; remove --debug/--development");
    }

    if let Some(value) = options.native_opt_level.as_deref()
        && !matches!(value, "0" | "1" | "2" | "3" | "s" | "z")
    {
        bail!("--android-native-opt-level must be one of 0, 1, 2, 3, s, z");
    }
    if let Some(value) = options.native_lto.as_deref() {
        options.native_lto = Some(match value {
            "off" | "false" => "false".to_string(),
            "thin" => "thin".to_string(),
            "fat" | "true" => "fat".to_string(),
            _ => bail!("--android-native-lto must be one of off, thin, fat"),
        });
    }
    if let Some(value) = options.native_codegen_units.as_deref() {
        let units = value
            .parse::<u32>()
            .with_context(|| format!("invalid --android-native-codegen-units value {value}"))?;
        if units == 0 {
            bail!("--android-native-codegen-units must be greater than 0");
        }
    }
    if let Some(value) = options.native_strip.as_deref()
        && !matches!(value, "none" | "debuginfo" | "symbols")
    {
        bail!("--android-native-strip must be one of none, debuginfo, symbols");
    }
    if let Some(value) = options.native_panic.as_deref()
        && !matches!(value, "abort" | "unwind")
    {
        bail!("--android-native-panic must be one of abort, unwind");
    }
    if options.minify == Some(false) && options.shrink_resources == Some(true) {
        bail!("--android-shrink-resources true requires --android-minify true");
    }
    if !options.devkit && options.devkit_port.is_some() {
        bail!("--android-devkit-port requires --android-devkit");
    }
    if !options.devkit && options.devkit_registry_dir.is_some() {
        bail!("--android-devkit-registry-dir requires --android-devkit");
    }

    Ok(options)
}

fn build_android_app(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    options: AndroidBuildOptions,
) -> Result<()> {
    let toolchain = setup_android_machine(&app.context.pins)?;
    let targets = select_android_build_targets(&toolchain, &options)?;
    build_android_app_with_toolchain(app, features, profile, &options, &toolchain, &targets)
}

fn build_android_app_with_toolchain(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    options: &AndroidBuildOptions,
    toolchain: &AndroidToolchain,
    targets: &[AndroidBuildTarget],
) -> Result<()> {
    let unstripped_jni_libs_dir = app.temp_root()?.join("android").join("jniLibs-unstripped");
    let packaged_jni_libs_dir = app.android_generated_jni_dir()?;
    reset_dir(&unstripped_jni_libs_dir)?;
    reset_dir(&packaged_jni_libs_dir)?;

    let gradle_project_dir = app.require_android_gradle_dir()?;
    let rust_manifest = app.android_rust_manifest(features)?;
    let toolchain_api_level = app.context.pins.android_ndk_api.value.clone();
    let cargo_manifest = command_path(&rust_manifest);
    let rust_manifest_dir = rust_manifest.parent().ok_or_else(|| {
        anyhow!(
            "invalid Android Rust manifest path {}",
            rust_manifest.display()
        )
    })?;
    let cargo_manifest_dir = command_path(rust_manifest_dir);
    let cargo_unstripped_jni_libs_dir = command_path(&unstripped_jni_libs_dir);
    let gradle_project_command_dir = command_path(&gradle_project_dir);
    for target in targets {
        ensure_rust_target(target.rust_target)?;
        println!(
            "Building Android app {} native library {} from {} for {} ({})",
            app.slug,
            app.lib_name,
            cargo_manifest.display(),
            target.abi,
            target.rust_target
        );

        let mut cargo_ndk = Command::new("cargo");
        cargo_ndk
            .current_dir(&cargo_manifest_dir)
            .args(["ndk", "-t", target.abi, "-o"])
            .arg(&cargo_unstripped_jni_libs_dir)
            .args(["--platform", &toolchain_api_level, "build"])
            .arg("--manifest-path")
            .arg(&cargo_manifest);
        if let Some(profile_arg) = profile.cargo_arg() {
            cargo_ndk.arg(profile_arg);
        }
        if profile.is_release() {
            apply_android_release_profile_overrides(&mut cargo_ndk, options);
        }
        if options.devkit {
            cargo_ndk.args(["--features", "devkit"]).env(
                "RAYX_ANDROID_DEVKIT_PORT",
                options.devkit_port().to_string(),
            );
        }
        apply_android_env(&mut cargo_ndk, toolchain);
        run_process(&mut cargo_ndk)?;
    }
    stop_android_gradle_daemon(&gradle_project_command_dir, toolchain);
    strip_android_jni_libs(toolchain, &unstripped_jni_libs_dir, &packaged_jni_libs_dir)?;
    copy_android_cpp_runtime(toolchain, targets, &packaged_jni_libs_dir)?;
    let android_content = android_content_staging_dir(app)?;
    stage_android_package_content(app, features, &android_content)?;

    let gradle = gradle_command_path(&gradle_project_command_dir);
    let mut gradle_cmd = Command::new(gradle);
    gradle_cmd
        .current_dir(&gradle_project_command_dir)
        .arg(profile.gradle_task())
        .arg("--no-daemon")
        .arg("--warning-mode=fail")
        .arg("--rerun-tasks");
    apply_android_gradle_properties(&mut gradle_cmd, options);
    apply_android_env(&mut gradle_cmd, toolchain);
    run_process(&mut gradle_cmd)
}

fn deploy_android_app(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    profile: BuildProfile,
    options: AndroidBuildOptions,
) -> Result<()> {
    let toolchain = setup_android_machine(&app.context.pins)?;
    let device = select_android_device(&toolchain, &options, true)?
        .ok_or_else(|| anyhow!("no Android device selected for deploy"))?;
    let apk_path = match options.apk_path.as_deref() {
        Some(path) => absolutize_path(path)?,
        None => {
            let target = select_android_build_target(&toolchain, Some(&device))?;
            build_android_app_with_toolchain(
                app,
                features,
                profile,
                &options,
                &toolchain,
                &[target],
            )?;
            android_apk_path(app, profile)?
        }
    };
    install_android_apk(app, &toolchain, &device, &apk_path, !options.no_launch)?;
    if options.devkit && !options.no_launch {
        install_android_devkit_bridge(app, &toolchain, &device, &options)?;
    }
    Ok(())
}

fn install_android_apk(
    app: &AppDescriptor,
    toolchain: &AndroidToolchain,
    device: &str,
    apk_path: &Path,
    launch: bool,
) -> Result<()> {
    if !apk_path.is_file() {
        bail!("Android APK not found at {}", apk_path.display());
    }

    println!(
        "Installing {} on Android device {device}",
        apk_path.display()
    );
    let mut install = android_adb_command(toolchain, Some(device));
    install.arg("install").arg("-r").arg(apk_path);
    run_process(&mut install)?;

    if !launch {
        return Ok(());
    }

    println!("Launching Android app {} on {device}", app.slug);
    let mut launch = android_adb_command(toolchain, Some(device));
    launch.args([
        "shell",
        "am",
        "start",
        "-n",
        &app.android_launch_component()?,
        "-a",
        "android.intent.action.MAIN",
        "-c",
        "android.intent.category.LAUNCHER",
    ]);
    run_process(&mut launch)
}

fn ensure_rust_target(target: &str) -> Result<()> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .context("failed to list installed Rust targets with rustup")?;
    if !output.status.success() {
        bail!("rustup target list --installed failed");
    }
    let installed = String::from_utf8_lossy(&output.stdout);
    if installed.lines().any(|line| line.trim() == target) {
        return Ok(());
    }

    println!("Installing Rust target {target}");
    run_process(Command::new("rustup").args(["target", "add", target]))
}

fn parse_android_port(flag: &str, value: &str) -> Result<u16> {
    let port = value
        .parse::<u16>()
        .with_context(|| format!("invalid {flag} value {value}"))?;
    if port == 0 {
        bail!("{flag} must be greater than 0");
    }
    Ok(port)
}

fn install_android_devkit_bridge(
    app: &AppDescriptor,
    toolchain: &AndroidToolchain,
    device: &str,
    options: &AndroidBuildOptions,
) -> Result<()> {
    let port = options.devkit_port();
    println!("Configuring Android RayX DevKit bridge on 127.0.0.1:{port}");

    remove_android_forward(toolchain, device, port);
    let mut forward = android_adb_command(toolchain, Some(device));
    forward.args(["forward", &format!("tcp:{port}"), &format!("tcp:{port}")]);
    run_process(&mut forward)?;

    wait_for_android_devkit(port)?;
    let registry_path = write_android_devkit_registration(app, toolchain, device, options)?;
    println!(
        "RayX DevKit registration written to {}",
        registry_path.display()
    );
    println!(
        "Inspect with: cargo run -p rayx_devkit_mcp -- --connect auto --registry-dir {}",
        registry_path
            .parent()
            .map(Path::display)
            .ok_or_else(|| anyhow!("invalid DevKit registry path"))?
    );
    Ok(())
}

fn remove_android_forward(toolchain: &AndroidToolchain, device: &str, port: u16) {
    let mut command = android_adb_command(toolchain, Some(device));
    command
        .args(["forward", "--remove", &format!("tcp:{port}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _ = command.status();
}

fn wait_for_android_devkit(port: u16) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let endpoint = format!("127.0.0.1:{port}");
    let request = br#"{"jsonrpc":"2.0","id":1,"method":"devkit.metadata","params":{}}
"#;

    while Instant::now() < deadline {
        match TcpStream::connect(&endpoint) {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                stream.write_all(request)?;
                stream.flush()?;

                let mut response = String::new();
                BufReader::new(stream).read_line(&mut response)?;
                if response.contains(r#""result""#) {
                    return Ok(());
                }
            }
            Err(_) => thread::sleep(Duration::from_millis(250)),
        }
    }

    bail!("RayX DevKit connector did not become reachable at {endpoint}");
}

fn write_android_devkit_registration(
    app: &AppDescriptor,
    toolchain: &AndroidToolchain,
    device: &str,
    options: &AndroidBuildOptions,
) -> Result<PathBuf> {
    let port = options.devkit_port();
    let registry_dir = options
        .devkit_registry_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("rayx-devkit-android"));
    fs::create_dir_all(&registry_dir)
        .with_context(|| format!("creating {}", registry_dir.display()))?;

    remove_stale_android_devkit_registrations(&registry_dir, &app.slug)?;

    let process_id = android_app_process_id(app, toolchain, device)?.unwrap_or_default();
    let registration = serde_json::json!({
        "protocol_version": DEVKIT_PROTOCOL_VERSION,
        "app_id": app.slug,
        "host": "127.0.0.1",
        "port": port,
        "endpoint": format!("127.0.0.1:{port}"),
        "process_id": process_id,
        "auth_required": false
    });
    let path = registry_dir.join(format!(
        "{}-android-{}-{}.json",
        sanitize_devkit_filename(&app.slug),
        process_id,
        port
    ));
    fs::write(&path, serde_json::to_vec_pretty(&registration)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn remove_stale_android_devkit_registrations(registry_dir: &Path, app_id: &str) -> Result<()> {
    let prefix = format!("{}-android-", sanitize_devkit_filename(app_id));
    for entry in
        fs::read_dir(registry_dir).with_context(|| format!("reading {}", registry_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if file_name.starts_with(&prefix) {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

fn sanitize_devkit_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn android_app_process_id(
    app: &AppDescriptor,
    toolchain: &AndroidToolchain,
    device: &str,
) -> Result<Option<u32>> {
    let Some(application_id) = app.android_application_id.as_deref() else {
        return Ok(None);
    };
    let output = android_adb_output(toolchain, Some(device), &["shell", "pidof", application_id])?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<u32>().ok()))
}

fn ensure_cargo_ndk() -> Result<()> {
    if command_succeeds(Command::new("cargo").args(["ndk", "--version"])) {
        return Ok(());
    }

    println!("Installing cargo-ndk");
    run_process(Command::new("cargo").args(["install", "cargo-ndk", "--locked"]))
}

fn find_android_sdk() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(value) = non_empty_env(variable) {
            candidates.push(PathBuf::from(value));
        }
    }

    if cfg!(target_os = "windows") {
        if let Some(local_app_data) = non_empty_env("LOCALAPPDATA") {
            candidates.push(PathBuf::from(local_app_data).join("Android").join("Sdk"));
        }
        candidates.push(PathBuf::from(r"C:\Android\SDK"));
        candidates.push(PathBuf::from(r"C:\Android\sdk"));
    } else if let Some(home) = non_empty_env("HOME") {
        candidates.push(
            PathBuf::from(&home)
                .join("Library")
                .join("Android")
                .join("sdk"),
        );
        candidates.push(PathBuf::from(&home).join("Android").join("Sdk"));
    }

    candidates
        .into_iter()
        .find(|path| path.is_dir())
        .ok_or_else(|| {
            anyhow!(
                "Android SDK not found. Set ANDROID_HOME or install the Android SDK in a standard location."
            )
        })
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn ensure_android_sdk_path(sdk_dir: &Path, relative_path: PathBuf, package: &str) -> Result<()> {
    let path = sdk_dir.join(&relative_path);
    if path.exists() {
        return Ok(());
    }
    install_android_sdk_package(sdk_dir, package)?;
    if path.exists() {
        Ok(())
    } else {
        bail!(
            "Android SDK package {package} did not create expected path {}",
            path.display()
        )
    }
}

fn install_android_sdk_package(sdk_dir: &Path, package: &str) -> Result<()> {
    let sdkmanager = find_sdkmanager(sdk_dir)?;
    println!("Installing Android SDK package {package}");
    let mut command = Command::new(sdkmanager);
    command.arg(package);
    apply_android_sdk_env(&mut command, sdk_dir, None);
    run_process(&mut command)
}

fn find_sdkmanager(sdk_dir: &Path) -> Result<PathBuf> {
    let executable = sdkmanager_file_name();
    let latest = sdk_dir
        .join("cmdline-tools")
        .join("latest")
        .join("bin")
        .join(executable);
    if latest.is_file() {
        return Ok(latest);
    }

    let cmdline_tools = sdk_dir.join("cmdline-tools");
    if cmdline_tools.is_dir() {
        let mut candidates = fs::read_dir(&cmdline_tools)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path().join("bin").join(executable))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        candidates.sort();
        if let Some(candidate) = candidates.pop() {
            return Ok(candidate);
        }
    }

    let legacy = sdk_dir.join("tools").join("bin").join(executable);
    if legacy.is_file() {
        return Ok(legacy);
    }

    bail!(
        "sdkmanager not found under {}. Install Android SDK command-line tools.",
        sdk_dir.display()
    )
}

fn sdkmanager_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "sdkmanager.bat"
    } else {
        "sdkmanager"
    }
}

fn find_android_ndk(sdk_dir: &Path) -> Option<PathBuf> {
    for variable in ["ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "NDK_HOME"] {
        if let Some(value) = non_empty_env(variable) {
            let path = PathBuf::from(value);
            if path.is_dir() {
                return Some(path);
            }
        }
    }

    let ndk_root = sdk_dir.join("ndk");
    let mut candidates = fs::read_dir(ndk_root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.pop()
}

fn find_android_adb(sdk_dir: &Path) -> Result<PathBuf> {
    let sdk_adb = sdk_dir.join("platform-tools").join(adb_file_name());
    if sdk_adb.is_file() {
        return Ok(sdk_adb);
    }
    if command_succeeds(Command::new("adb").arg("version")) {
        return Ok(PathBuf::from("adb"));
    }
    bail!("adb not found. Install Android platform-tools or set ANDROID_HOME to a complete SDK.")
}

fn adb_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "adb.exe"
    } else {
        "adb"
    }
}

fn apply_android_env(command: &mut Command, toolchain: &AndroidToolchain) {
    apply_android_sdk_env(command, &toolchain.sdk_dir, Some(&toolchain.ndk_dir));
}

fn apply_android_sdk_env(command: &mut Command, sdk_dir: &Path, ndk_dir: Option<&Path>) {
    command.env("ANDROID_HOME", sdk_dir);
    command.env("ANDROID_SDK_ROOT", sdk_dir);
    if let Some(ndk_dir) = ndk_dir {
        command.env("ANDROID_NDK_HOME", ndk_dir);
        command.env("ANDROID_NDK_ROOT", ndk_dir);
        command.env("NDK_HOME", ndk_dir);
    }
}

fn apply_android_release_profile_overrides(command: &mut Command, options: &AndroidBuildOptions) {
    if let Some(value) = options
        .native_opt_level
        .as_deref()
        .or(options.size_optimized.then_some("z"))
    {
        command.env("CARGO_PROFILE_RELEASE_OPT_LEVEL", value);
    }
    if let Some(value) = options
        .native_lto
        .as_deref()
        .or(options.size_optimized.then_some("fat"))
    {
        command.env("CARGO_PROFILE_RELEASE_LTO", value);
    }
    if let Some(value) = options
        .native_codegen_units
        .as_deref()
        .or(options.size_optimized.then_some("1"))
    {
        command.env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", value);
    }
    if let Some(value) = options
        .native_strip
        .as_deref()
        .or(options.size_optimized.then_some("symbols"))
    {
        command.env("CARGO_PROFILE_RELEASE_STRIP", value);
    }
    if let Some(value) = options
        .native_panic
        .as_deref()
        .or(options.size_optimized.then_some("abort"))
    {
        command.env("CARGO_PROFILE_RELEASE_PANIC", value);
    }
    if options.size_optimized {
        command
            .env("CARGO_PROFILE_RELEASE_DEBUG", "false")
            .env("CARGO_PROFILE_RELEASE_INCREMENTAL", "false")
            .env("CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "false")
            .env("CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS", "false");
    }
}

fn apply_android_gradle_properties(command: &mut Command, options: &AndroidBuildOptions) {
    if let Some(value) = options.minify.or(options.size_optimized.then_some(true)) {
        command.arg(format!("-PrayxAndroidMinify={value}"));
    }
    if let Some(value) = options
        .shrink_resources
        .or(options.size_optimized.then_some(true))
    {
        command.arg(format!("-PrayxAndroidShrinkResources={value}"));
    }
    if let Some(value) = options
        .crunch_pngs
        .or(options.size_optimized.then_some(true))
    {
        command.arg(format!("-PrayxAndroidCrunchPngs={value}"));
    }
    if let Some(value) = options
        .compress_native_libs
        .or(options.size_optimized.then_some(true))
    {
        command.arg(format!("-PrayxAndroidCompressNativeLibs={value}"));
    }
}

fn strip_android_jni_libs(
    toolchain: &AndroidToolchain,
    source_dir: &Path,
    output_dir: &Path,
) -> Result<()> {
    let strip = find_android_llvm_strip(&toolchain.ndk_dir)?;
    let mut libraries = Vec::new();
    collect_files_with_extension(source_dir, "so", &mut libraries)?;
    for library in libraries {
        let before = fs::metadata(&library)
            .with_context(|| format!("reading metadata for {}", library.display()))?
            .len();
        let relative = library
            .strip_prefix(source_dir)
            .with_context(|| format!("relativizing {}", library.display()))?;
        let stripped = output_dir.join(relative);
        if let Some(parent) = stripped.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut command = Command::new(&strip);
        command
            .arg("--strip-unneeded")
            .arg("-o")
            .arg(&stripped)
            .arg(&library);
        apply_android_env(&mut command, toolchain);
        run_process(&mut command)?;
        let after = fs::metadata(&stripped)
            .with_context(|| format!("reading metadata for {}", stripped.display()))?
            .len();
        if after < before {
            println!(
                "Stripped {} from {} MB to {} MB",
                relative.display(),
                before / 1_000_000,
                after / 1_000_000
            );
        }
    }
    Ok(())
}

fn copy_android_cpp_runtime(
    toolchain: &AndroidToolchain,
    targets: &[AndroidBuildTarget],
    output_dir: &Path,
) -> Result<()> {
    let prebuilt = toolchain
        .ndk_dir
        .join("toolchains")
        .join("llvm")
        .join("prebuilt");
    let mut hosts = Vec::new();
    collect_files_named(&prebuilt, "libc++_shared.so", &mut hosts)?;

    for target in targets {
        let runtime = hosts
            .iter()
            .find(|path| {
                path.ends_with(
                    Path::new("sysroot")
                        .join("usr")
                        .join("lib")
                        .join(target.rust_target)
                        .join("libc++_shared.so"),
                )
            })
            .ok_or_else(|| {
                anyhow!(
                    "libc++_shared.so not found for Android target {} under {}",
                    target.rust_target,
                    toolchain.ndk_dir.display()
                )
            })?;
        let destination = output_dir.join(target.abi).join("libc++_shared.so");
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::copy(runtime, &destination).with_context(|| {
            format!(
                "copying Android C++ runtime from {} to {}",
                runtime.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn stop_android_gradle_daemon(project_dir: &Path, toolchain: &AndroidToolchain) {
    let gradle = gradle_command_path(project_dir);
    if !gradle.is_file() {
        return;
    }
    let mut command = Command::new(gradle);
    command
        .current_dir(project_dir)
        .arg("--stop")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_android_env(&mut command, toolchain);
    let _ = command.status();
}

fn find_android_llvm_strip(ndk_dir: &Path) -> Result<PathBuf> {
    let prebuilt = ndk_dir.join("toolchains").join("llvm").join("prebuilt");
    let mut candidates = Vec::new();
    collect_files_named(&prebuilt, llvm_strip_file_name(), &mut candidates)?;
    candidates.sort();
    candidates.into_iter().next().ok_or_else(|| {
        anyhow!(
            "llvm-strip not found under Android NDK {}",
            ndk_dir.display()
        )
    })
}

fn llvm_strip_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "llvm-strip.exe"
    } else {
        "llvm-strip"
    }
}

fn select_android_device(
    toolchain: &AndroidToolchain,
    options: &AndroidBuildOptions,
    start_emulator_if_missing: bool,
) -> Result<Option<String>> {
    if let Some(address) = options.connect_address.as_deref() {
        connect_android_device(toolchain, address)?;
        wait_for_android_serial(toolchain, address)?;
        println!("Using Android device {address}");
        return Ok(Some(address.to_string()));
    }

    if options.paired_device {
        let device = select_paired_android_device(toolchain)?;
        println!("Using paired Android device {device}");
        return Ok(Some(device));
    }

    if let Some(serial) = options.device_serial.as_deref() {
        ensure_android_serial_connected(toolchain, serial)?;
        println!("Using Android device {serial}");
        return Ok(Some(serial.to_string()));
    }

    let devices = connected_android_devices(toolchain)?;
    match devices.as_slice() {
        [] if start_emulator_if_missing => start_android_emulator(toolchain).map(Some),
        [] => Ok(None),
        [device] => {
            println!("Using Android device {device}");
            Ok(Some(device.clone()))
        }
        _ => bail!(
            "Multiple Android devices are connected: {}. Pass --android-device <serial>, --android-connect <host:port>, or --android-paired.",
            devices.join(", ")
        ),
    }
}

fn select_paired_android_device(toolchain: &AndroidToolchain) -> Result<String> {
    let connected_wireless = connected_android_devices(toolchain)?
        .into_iter()
        .filter(|device| is_android_wireless_serial(device))
        .collect::<Vec<_>>();
    match connected_wireless.as_slice() {
        [device] => return Ok(device.clone()),
        [] => {}
        _ => bail!(
            "Multiple wireless Android devices are connected: {}. Pass --android-device <serial>.",
            connected_wireless.join(", ")
        ),
    }

    let endpoints = discover_android_mdns_connect_endpoints(toolchain)?;
    match endpoints.as_slice() {
        [endpoint] => {
            connect_android_device(toolchain, endpoint)?;
            wait_for_android_serial(toolchain, endpoint)?;
            Ok(endpoint.clone())
        }
        [] => bail!(
            "No paired Android wireless debugging service was discovered. Keep Wireless debugging enabled on the phone, keep both machines on the same network, or pass --android-connect <host:port>."
        ),
        _ => bail!(
            "Multiple paired Android wireless debugging services were discovered: {}. Pass --android-connect <host:port> for the phone you want.",
            endpoints.join(", ")
        ),
    }
}

fn ensure_android_serial_connected(toolchain: &AndroidToolchain, serial: &str) -> Result<()> {
    if connected_android_devices(toolchain)?
        .iter()
        .any(|device| device == serial)
    {
        return Ok(());
    }

    if is_android_wireless_serial(serial) {
        connect_android_device(toolchain, serial)?;
        wait_for_android_serial(toolchain, serial)?;
        return Ok(());
    }

    bail!(
        "Android device {serial} is not connected. Connected devices: {}",
        connected_android_devices(toolchain)?.join(", ")
    )
}

fn is_android_wireless_serial(serial: &str) -> bool {
    serial.contains(':') && !serial.starts_with("emulator-")
}

fn discover_android_mdns_connect_endpoints(toolchain: &AndroidToolchain) -> Result<Vec<String>> {
    let output = android_adb_output(toolchain, None, &["mdns", "services"])?;
    if !output.status.success() {
        bail!("adb mdns services failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut endpoints = Vec::new();
    for line in stdout.lines() {
        if !line.contains("adb-tls-connect") {
            continue;
        }
        for token in line.split_whitespace() {
            if let Some(endpoint) = parse_android_connect_endpoint(token)
                && !endpoints.contains(&endpoint)
            {
                endpoints.push(endpoint);
            }
        }
    }
    Ok(endpoints)
}

fn parse_android_connect_endpoint(token: &str) -> Option<String> {
    let endpoint =
        token.trim_matches(|ch: char| matches!(ch, ',' | ';' | '"' | '\'' | '[' | ']' | '(' | ')'));
    let (host, port) = endpoint.rsplit_once(':')?;
    if host.is_empty() || port.is_empty() || !port.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(endpoint.to_string())
}

fn connect_android_device(toolchain: &AndroidToolchain, address: &str) -> Result<()> {
    println!("Connecting Android device {address}");
    let output = android_adb_output(toolchain, None, &["connect", address])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stdout.lines().chain(stderr.lines()) {
        if !line.trim().is_empty() {
            println!("{line}");
        }
    }
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    if !output.status.success()
        || combined.contains("failed")
        || combined.contains("unable")
        || combined.contains("cannot")
    {
        bail!("failed to connect Android device {address}");
    }
    Ok(())
}

fn connected_android_devices(toolchain: &AndroidToolchain) -> Result<Vec<String>> {
    let output = android_adb_output(toolchain, None, &["devices"])?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            let state = parts.next()?;
            (state == "device").then(|| serial.to_string())
        })
        .collect())
}

fn select_android_build_target(
    toolchain: &AndroidToolchain,
    device: Option<&str>,
) -> Result<AndroidBuildTarget> {
    let abis = connected_android_abis(toolchain, device)?;
    if let Some(target) = abis.iter().find_map(|abi| android_target_for_abi(abi)) {
        return Ok(target);
    }

    let target = default_android_build_target()?;
    if abis.is_empty() {
        println!(
            "No Android ABI reported by adb; defaulting to {} ({})",
            target.abi, target.rust_target
        );
    } else {
        println!(
            "No supported Android ABI found in {}; defaulting to {} ({})",
            abis.join(", "),
            target.abi,
            target.rust_target
        );
    }
    Ok(target)
}

fn select_android_build_targets(
    toolchain: &AndroidToolchain,
    options: &AndroidBuildOptions,
) -> Result<Vec<AndroidBuildTarget>> {
    if options.device_serial.is_some() || options.connect_address.is_some() || options.paired_device
    {
        let device = select_android_device(toolchain, options, false)?;
        return Ok(vec![select_android_build_target(
            toolchain,
            device.as_deref(),
        )?]);
    }

    let devices = connected_android_devices(toolchain)?;
    if devices.is_empty() {
        return Ok(vec![default_android_build_target()?]);
    }

    let mut targets = Vec::new();
    for device in devices {
        let abis = connected_android_abis(toolchain, Some(&device))?;
        if let Some(target) = abis.iter().find_map(|abi| android_target_for_abi(abi)) {
            push_unique_android_target(&mut targets, target);
        }
    }

    if targets.is_empty() {
        targets.push(default_android_build_target()?);
    }
    Ok(targets)
}

fn default_android_build_target() -> Result<AndroidBuildTarget> {
    android_target_for_abi(DEFAULT_ANDROID_ABI)
        .ok_or_else(|| anyhow!("default Android ABI {DEFAULT_ANDROID_ABI} is not supported"))
}

fn push_unique_android_target(targets: &mut Vec<AndroidBuildTarget>, target: AndroidBuildTarget) {
    if !targets.contains(&target) {
        targets.push(target);
    }
}

fn connected_android_abis(
    toolchain: &AndroidToolchain,
    device: Option<&str>,
) -> Result<Vec<String>> {
    let selected_device = match device {
        Some(device) => Some(device.to_string()),
        None => match connected_android_devices(toolchain)?.as_slice() {
            [] => None,
            [device] => Some(device.clone()),
            devices => bail!(
                "Multiple Android devices are connected: {}. Pass --android-device <serial>, --android-connect <host:port>, or --android-paired.",
                devices.join(", ")
            ),
        },
    };
    let Some(selected_device) = selected_device else {
        return Ok(Vec::new());
    };

    for property in [
        "ro.product.cpu.abilist64",
        "ro.product.cpu.abilist",
        "ro.product.cpu.abi",
    ] {
        let output = android_adb_output(
            toolchain,
            Some(&selected_device),
            &["shell", "getprop", property],
        )?;
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let abis = stdout
            .split(',')
            .map(str::trim)
            .filter(|abi| !abi.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if !abis.is_empty() {
            return Ok(abis);
        }
    }

    Ok(Vec::new())
}

fn android_target_for_abi(abi: &str) -> Option<AndroidBuildTarget> {
    match abi {
        "arm64-v8a" => Some(AndroidBuildTarget {
            abi: "arm64-v8a",
            rust_target: "aarch64-linux-android",
        }),
        "x86_64" => Some(AndroidBuildTarget {
            abi: "x86_64",
            rust_target: "x86_64-linux-android",
        }),
        "armeabi-v7a" => Some(AndroidBuildTarget {
            abi: "armeabi-v7a",
            rust_target: "armv7-linux-androideabi",
        }),
        "x86" => Some(AndroidBuildTarget {
            abi: "x86",
            rust_target: "i686-linux-android",
        }),
        _ => None,
    }
}

fn android_adb_command(toolchain: &AndroidToolchain, device: Option<&str>) -> Command {
    let mut command = Command::new(&toolchain.adb);
    if let Some(device) = device {
        command.args(["-s", device]);
    }
    apply_android_env(&mut command, toolchain);
    command
}

fn android_adb_output(
    toolchain: &AndroidToolchain,
    device: Option<&str>,
    args: &[&str],
) -> Result<std::process::Output> {
    let mut command = android_adb_command(toolchain, device);
    command.args(args);
    command.output().with_context(|| {
        format!(
            "failed to run Android adb command {} {}",
            toolchain.adb.display(),
            args.join(" ")
        )
    })
}

fn find_android_emulator(sdk_dir: &Path) -> Option<PathBuf> {
    let sdk_emulator = sdk_dir.join("emulator").join(emulator_file_name());
    if sdk_emulator.is_file() {
        return Some(sdk_emulator);
    }
    command_succeeds(Command::new("emulator").arg("-version")).then(|| PathBuf::from("emulator"))
}

fn emulator_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "emulator.exe"
    } else {
        "emulator"
    }
}

fn first_android_avd(emulator: &Path, toolchain: &AndroidToolchain) -> Result<Option<String>> {
    let mut command = Command::new(emulator);
    command.arg("-list-avds");
    apply_android_env(&mut command, toolchain);
    let output = command
        .output()
        .with_context(|| format!("failed to list Android AVDs with {}", emulator.display()))?;
    if !output.status.success() {
        bail!("Android emulator -list-avds failed");
    }
    Ok(first_android_avd_name(&String::from_utf8_lossy(&output.stdout)).map(str::to_string))
}

fn first_android_avd_name(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| {
        !line.is_empty()
            && !matches!(
                line.split_once('|').map(|(prefix, _)| prefix.trim()),
                Some("INFO" | "WARNING" | "ERROR" | "DEBUG")
            )
    })
}

fn start_android_emulator(toolchain: &AndroidToolchain) -> Result<String> {
    let emulator = match find_android_emulator(&toolchain.sdk_dir) {
        Some(emulator) => emulator,
        None => bail!(
            "No Android device/emulator is connected, and emulator was not found under {}",
            toolchain.sdk_dir.display()
        ),
    };
    let avd = first_android_avd(&emulator, toolchain)?.ok_or_else(|| {
        anyhow!("No Android device is connected and no Android AVD is configured")
    })?;

    println!("No Android device found; starting emulator {avd}");
    let mut command = Command::new(&emulator);
    command
        .args(["-avd", &avd, "-netdelay", "none", "-netspeed", "full"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_android_env(&mut command, toolchain);
    command
        .spawn()
        .with_context(|| format!("failed to start Android emulator {}", emulator.display()))?;

    wait_for_android_device(toolchain)
}

fn wait_for_android_device(toolchain: &AndroidToolchain) -> Result<String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        for device in connected_android_devices(toolchain)? {
            if android_boot_completed(toolchain, &device)? {
                println!("Android emulator is ready");
                return Ok(device);
            }
        }
        thread::sleep(Duration::from_secs(2));
    }
    bail!("Timed out waiting for Android emulator to boot")
}

fn wait_for_android_serial(toolchain: &AndroidToolchain, serial: &str) -> Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if connected_android_devices(toolchain)?
            .iter()
            .any(|device| device == serial)
        {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    bail!("Timed out waiting for Android device {serial} to connect")
}

fn android_boot_completed(toolchain: &AndroidToolchain, device: &str) -> Result<bool> {
    let output = android_adb_output(
        toolchain,
        Some(device),
        &["shell", "getprop", "sys.boot_completed"],
    )?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "1")
}

fn android_apk_path(app: &AppDescriptor, profile: BuildProfile) -> Result<PathBuf> {
    let variant = profile.name();
    Ok(app
        .require_android_gradle_dir()?
        .join("app")
        .join("build")
        .join("outputs")
        .join("apk")
        .join(variant)
        .join(format!("app-{variant}.apk")))
}

fn android_content_staging_dir(app: &AppDescriptor) -> Result<PathBuf> {
    Ok(app
        .require_android_gradle_dir()?
        .join("app")
        .join("src")
        .join("main")
        .join("assets"))
}

fn stage_android_package_content(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    destination: &Path,
) -> Result<()> {
    assets::package_app_assets_with_selection(app, features, &destination.join("assets"))
        .with_context(|| {
            format!(
                "staging merged Android app/package assets at {}",
                destination.display()
            )
        })?;
    assets::package_app_content(app, destination).with_context(|| {
        format!(
            "staging declared Android app content at {}",
            destination.display()
        )
    })?;
    Ok(())
}

fn gradle_command_path(project_dir: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        project_dir.join("gradlew.bat")
    } else {
        project_dir.join("gradlew")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn android_avd_discovery_ignores_emulator_diagnostics() {
        let output = "INFO    | Storing crashdata in: C:\\temp\\emu-crash.db\nrayx-test-suite-api35-x86_64\n";

        assert_eq!(
            first_android_avd_name(output),
            Some("rayx-test-suite-api35-x86_64")
        );
    }

    #[test]
    fn android_build_uses_selected_rust_manifest_source() {
        let source = include_str!("android.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("android source should contain production code before tests");

        assert!(
            production_source.contains("app.android_rust_manifest(features)"),
            "Android builds should select the generated feature-specific Rust manifest"
        );
        assert!(
            production_source.contains(".arg(\"--manifest-path\")"),
            "cargo ndk should receive an explicit Android Rust manifest path"
        );
        assert!(
            !production_source.contains(".current_dir(&app.root)"),
            "Android builds should not hardcode the app root as the cargo build manifest"
        );
    }

    #[test]
    fn android_packages_cpp_runtime_for_each_selected_abi()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_app_root("android-cpp-runtime")?;
        let runtime = root
            .join("ndk/toolchains/llvm/prebuilt/host/sysroot/usr/lib/x86_64-linux-android/libc++_shared.so");
        fs::create_dir_all(runtime.parent().expect("runtime parent"))?;
        fs::write(&runtime, b"cpp-runtime")?;
        let output = root.join("jniLibs");
        let toolchain = AndroidToolchain {
            sdk_dir: root.join("sdk"),
            ndk_dir: root.join("ndk"),
            adb: root.join("adb"),
        };

        copy_android_cpp_runtime(
            &toolchain,
            &[AndroidBuildTarget {
                abi: "x86_64",
                rust_target: "x86_64-linux-android",
            }],
            &output,
        )?;

        assert_eq!(
            fs::read(output.join("x86_64/libc++_shared.so"))?,
            b"cpp-runtime"
        );
        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn android_asset_staging_dir_targets_packaged_app_assets()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_app_root("xtask-android-assets")?;
        fs::create_dir_all(root.join("platform/android/gradle"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-android-assets"),
        )?;
        let app = crate::app::test_support::resolve_str(root.to_str().expect("utf-8 path"))?;

        assert_eq!(
            android_content_staging_dir(&app)?.join("assets"),
            app.root
                .join("platform")
                .join("android")
                .join("gradle")
                .join("app")
                .join("src")
                .join("main")
                .join("assets")
                .join("assets")
        );

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn android_package_asset_staging_includes_theme_provenance_and_removes_stale_files()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let workspace = crate::app::test_support::FixtureWorkspace::new();
        let app = workspace.app();
        let staging_root = temp_app_root("xtask-android-package-assets")?;
        let content_root = staging_root.join("generated/assets");
        let destination = content_root.join("assets");
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("stale.txt"), "stale")?;

        stage_android_package_content(&app, &AppFeatureSelection::default(), &content_root)?;

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
            "Android sidecar should retain component-core package provenance: {sidecar}"
        );

        fs::remove_dir_all(staging_root).ok();
        Ok(())
    }

    #[test]
    fn android_app_content_is_at_asset_manager_root_outside_catalog()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_app_root("xtask-android-app-content")?;
        fs::create_dir_all(root.join("assets"))?;
        fs::write(
            root.join("Cargo.toml"),
            format!(
                "{}\n[lib]\npath = \"fixture.rs\"\n",
                cargo_manifest("xtask-android-app-content")
            ),
        )?;
        fs::write(root.join("fixture.rs"), "pub fn fixture() {}")?;
        fs::write(root.join("assets/icon.svg"), "<svg/>")?;
        fs::write(root.join("app_settings.json"), "{\"android\":true}")?;
        fs::write(
            root.join("rayx.assets.toml"),
            "version = 1\n[assets]\nroots = [\"assets\"]\n[app_content]\nfiles = [\"app_settings.json\"]\n",
        )?;
        let app = crate::app::test_support::resolve_str(root.to_str().expect("utf-8 path"))?;
        let content_root = root.join("generated/main/assets");
        stage_android_package_content(&app, &AppFeatureSelection::default(), &content_root)?;

        assert_eq!(
            fs::read_to_string(content_root.join("app_settings.json"))?,
            "{\"android\":true}"
        );
        assert!(content_root.join("assets/index.json").is_file());
        assert!(
            !fs::read_to_string(content_root.join("assets/index.json"))?.contains("app_settings")
        );
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
