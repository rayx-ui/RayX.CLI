//! The Android workstation set. It does what RayX xtask's `setup-android` ensured (SDK
//! platform-tools, platform and NDK through `sdkmanager`, `cargo-ndk`) and adds a complete
//! workstation: JDK, `JAVA_HOME`, Android Studio, the command-line tools, the emulator with a
//! system image for the host CPU, a default AVD and emulator acceleration.
//!
//! Package names verified with `winget show` on 2026-10-08: `Microsoft.OpenJDK.21` (Microsoft
//! Build of OpenJDK 21) and `Google.AndroidStudio`.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::host::{Arch, CommandSpec, Os, Privilege};

use super::{Action, Cx, PlanEnv, Probed, Set, SetupError, Step, rust};

/// The command-line tools build downloaded when the machine has no Android SDK.
const CMDLINE_TOOLS_BUILD: &str = "11076708";
const ANDROID_RUST_TARGETS: [&str; 2] = ["aarch64-linux-android", "x86_64-linux-android"];

/// The system image ABI for the host CPU.
pub fn host_abi(arch: Arch) -> &'static str {
    if arch == Arch::Arm64 {
        "arm64-v8a"
    } else {
        "x86_64"
    }
}

fn exe(os: Os, name: &str) -> String {
    if os == Os::Windows {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// A command-line tool of the SDK by its full path. A `.bat` started by its full path is handled
/// by the standard library, quoting included, so SDK paths with spaces work.
fn cmdline_tool(path: &Path) -> CommandSpec {
    CommandSpec::new(path.display().to_string())
}

fn bat(os: Os, name: &str) -> String {
    if os == Os::Windows {
        format!("{name}.bat")
    } else {
        name.to_string()
    }
}

/// A variable from the running environment or, failing that, the persistent user setting.
fn setting(cx: &Cx, name: &str) -> Option<String> {
    cx.machine
        .env(name)
        .filter(|value| !value.is_empty())
        .or_else(|| cx.user_path.variable(name).ok().flatten())
}

// ---------------------------------------------------------------------------------------------
// JDK

fn jdk_roots(cx: &Cx) -> Vec<PathBuf> {
    match cx.env.host.os {
        Os::Windows => {
            let pf = PathBuf::from(
                cx.machine
                    .env("ProgramFiles")
                    .unwrap_or_else(|| r"C:\Program Files".into()),
            );
            vec![
                pf.join("Microsoft"),
                pf.join("Eclipse Adoptium"),
                pf.join("Java"),
            ]
        }
        Os::MacOs => vec![PathBuf::from("/Library/Java/JavaVirtualMachines")],
        Os::Linux => vec![PathBuf::from("/usr/lib/jvm")],
    }
}

/// The Java major version of the JDK at `home`, from its `release` file.
pub fn jdk_major_at(cx: &Cx, home: &Path) -> Option<u32> {
    let release = cx.machine.read_to_string(&home.join("release"))?;
    let version = release
        .lines()
        .find_map(|line| line.strip_prefix("JAVA_VERSION="))?
        .trim()
        .trim_matches('"');
    let mut parts = version.split('.');
    let first: u32 = parts.next()?.parse().ok()?;
    // `1.8.0_x` is Java 8.
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

/// The installed JDK homes of the pinned major.
fn jdk_homes(cx: &Cx, major: u32) -> Vec<PathBuf> {
    let mut homes = Vec::new();
    for root in jdk_roots(cx) {
        for name in cx.machine.list_dir(&root) {
            let mut home = root.join(&name);
            if cx.env.host.os == Os::MacOs {
                home = home.join("Contents").join("Home");
            }
            if jdk_major_at(cx, &home) == Some(major) {
                homes.push(home);
            }
        }
    }
    homes
}

/// The Java major of the newest JDK installed in the usual places, whatever it is.
fn other_jdk_major(cx: &Cx) -> Option<u32> {
    let mut majors = Vec::new();
    for root in jdk_roots(cx) {
        for name in cx.machine.list_dir(&root) {
            let mut home = root.join(&name);
            if cx.env.host.os == Os::MacOs {
                home = home.join("Contents").join("Home");
            }
            majors.extend(jdk_major_at(cx, &home));
        }
    }
    majors.into_iter().max()
}

fn jdk_step(env: &PlanEnv) -> Step {
    let major: u32 = env.pins.jdk.value.trim().parse().unwrap_or(21);
    if env.host.os == Os::Linux {
        let package = format!("openjdk-{major}-jdk");
        return Step::apt(
            "jdk",
            Set::Android,
            format!("JDK {major} (apt)"),
            &[package.as_str()],
        );
    }
    Step::new(
        "jdk",
        Set::Android,
        format!("JDK {major} (Microsoft Build of OpenJDK)"),
        Privilege::None,
        move |cx| {
            if !jdk_homes(cx, major).is_empty() {
                return Probed::ok().with_found(major.to_string());
            }
            match other_jdk_major(cx) {
                Some(other) => Probed::missing(format!("only JDK {other} is installed"))
                    .with_found(other.to_string()),
                None => Probed::missing(format!("no JDK {major} found")),
            }
        },
        move |cx, _| {
            Ok(vec![Action::Run(match cx.env.host.os {
                Os::Windows => {
                    let mut install = CommandSpec::new("winget").args([
                        "install",
                        "--id",
                        &format!("Microsoft.OpenJDK.{major}"),
                        "--exact",
                    ]);
                    if cx.env.yes {
                        install = install
                            .args(["--accept-package-agreements", "--accept-source-agreements"]);
                    }
                    install.interactive()
                }
                _ => {
                    let brew = super::macos::brew_program(cx)
                        .map_or_else(|| "brew".to_string(), |path| path.display().to_string());
                    CommandSpec::new(brew)
                        .args(["install", "--cask", &format!("microsoft-openjdk@{major}")])
                        .interactive()
                }
            })])
        },
    )
}

fn java_home_step(env: &PlanEnv) -> Step {
    let major: u32 = env.pins.jdk.value.trim().parse().unwrap_or(21);
    Step::new(
        "java-home",
        Set::Android,
        format!("JAVA_HOME points at JDK {major}"),
        Privilege::None,
        move |cx| match setting(cx, "JAVA_HOME") {
            Some(home) => match jdk_major_at(cx, Path::new(&home)) {
                Some(found) if found == major => Probed::ok().with_found(found.to_string()),
                Some(found) => Probed::missing(format!("JAVA_HOME is {home}, a JDK {found}"))
                    .with_found(found.to_string()),
                None => Probed::missing(format!("JAVA_HOME is {home}, which is not a JDK")),
            },
            None => Probed::missing("JAVA_HOME is not set"),
        },
        move |cx, _| {
            let home = jdk_homes(cx, major).into_iter().next().ok_or_else(|| {
                SetupError::Prerequisite(format!(
                    "no JDK {major} was found to point JAVA_HOME at; install it first"
                ))
            })?;
            Ok(vec![Action::SetUserEnv {
                name: "JAVA_HOME".to_string(),
                value: home,
            }])
        },
    )
}

// ---------------------------------------------------------------------------------------------
// Android Studio

fn studio_step(env: &PlanEnv) -> Step {
    let os = env.host.os;
    let wsl = env.host.wsl;
    Step::new(
        "android-studio",
        Set::Android,
        "Android Studio",
        match os {
            Os::Linux => Privilege::Root,
            // winget's default is the installer's silent mode (`/S`), which cannot elevate itself:
            // it exits "successfully" and installs nothing. `--interactive` runs the installer
            // wizard, which asks for administrator rights through its own UAC prompt.
            Os::Windows | Os::MacOs => Privilege::None,
        },
        move |cx| {
            if wsl {
                return Probed {
                    satisfied: true,
                    detail: "provided by the Windows host".into(),
                    ..Probed::default()
                };
            }
            let installed = match os {
                Os::Windows => {
                    let pf = PathBuf::from(
                        cx.machine
                            .env("ProgramFiles")
                            .unwrap_or_else(|| r"C:\Program Files".into()),
                    );
                    let local = cx.machine.env("LOCALAPPDATA").map(PathBuf::from);
                    let in_pf = pf
                        .join("Android")
                        .join("Android Studio")
                        .join("bin")
                        .join("studio64.exe");
                    let in_local = local.map(|dir| {
                        dir.join("Programs")
                            .join("Android Studio")
                            .join("bin")
                            .join("studio64.exe")
                    });
                    cx.machine.exists(&in_pf) || in_local.is_some_and(|p| cx.machine.exists(&p))
                }
                Os::MacOs => cx
                    .machine
                    .exists(Path::new("/Applications/Android Studio.app")),
                Os::Linux => {
                    let desktop = ["XDG_CURRENT_DESKTOP", "DISPLAY", "WAYLAND_DISPLAY"]
                        .iter()
                        .any(|key| cx.machine.env(key).is_some_and(|v| !v.is_empty()));
                    if !desktop {
                        return Probed {
                            satisfied: true,
                            detail: "no desktop session: skipped".into(),
                            ..Probed::default()
                        };
                    }
                    cx.machine.exists(Path::new("/snap/bin/android-studio"))
                        || cx.machine.which("android-studio").is_some()
                }
            };
            if installed {
                Probed::ok()
            } else {
                Probed::missing("Android Studio is not installed")
            }
        },
        move |cx, _| {
            Ok(vec![Action::Run(match os {
                Os::Windows => {
                    let mut install = CommandSpec::new("winget").args([
                        "install",
                        "--id",
                        "Google.AndroidStudio",
                        "--exact",
                        "--interactive",
                    ]);
                    if cx.env.yes {
                        install = install
                            .args(["--accept-package-agreements", "--accept-source-agreements"]);
                    }
                    install.interactive()
                }
                Os::MacOs => {
                    let brew = super::macos::brew_program(cx)
                        .map_or_else(|| "brew".to_string(), |path| path.display().to_string());
                    CommandSpec::new(brew)
                        .args(["install", "--cask", "android-studio"])
                        .interactive()
                }
                Os::Linux => CommandSpec::new("snap")
                    .args(["install", "android-studio", "--classic"])
                    .root()
                    .interactive(),
            })])
        },
    )
    .optional_for_builds()
}

// ---------------------------------------------------------------------------------------------
// SDK

/// The default SDK location Android Studio also uses.
pub fn default_sdk_dir(cx: &Cx) -> Option<PathBuf> {
    Some(match cx.env.host.os {
        Os::Windows => PathBuf::from(cx.machine.env("LOCALAPPDATA")?)
            .join("Android")
            .join("Sdk"),
        Os::MacOs => cx
            .machine
            .home()?
            .join("Library")
            .join("Android")
            .join("sdk"),
        Os::Linux => cx.machine.home()?.join("Android").join("Sdk"),
    })
}

/// The Android SDK, found the way RayX xtask finds it: `ANDROID_HOME`, `ANDROID_SDK_ROOT`, then
/// the platform's default locations.
pub fn find_sdk(cx: &Cx) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|name| setting(cx, name))
        .map(PathBuf::from)
        .collect();
    if cx.env.host.os == Os::Windows {
        candidates.extend(default_sdk_dir(cx));
        candidates.push(PathBuf::from(r"C:\Android\SDK"));
        candidates.push(PathBuf::from(r"C:\Android\sdk"));
    } else if let Some(home) = cx.machine.home() {
        candidates.push(home.join("Library").join("Android").join("sdk"));
        candidates.push(home.join("Android").join("Sdk"));
    }
    candidates.into_iter().find(|path| cx.machine.is_dir(path))
}

/// `sdkmanager` under the SDK: `cmdline-tools/latest`, the newest other `cmdline-tools` version,
/// or the legacy `tools`.
pub fn find_sdkmanager(cx: &Cx, sdk: &Path) -> Option<PathBuf> {
    find_cmdline_tool(cx, sdk, "sdkmanager")
}

/// A command-line tool (`sdkmanager`, `avdmanager`) under the SDK, searched in the same layout.
pub fn find_cmdline_tool(cx: &Cx, sdk: &Path, tool: &str) -> Option<PathBuf> {
    let file = bat(cx.env.host.os, tool);
    let tools = sdk.join("cmdline-tools");
    let latest = tools.join("latest").join("bin").join(&file);
    if cx.machine.exists(&latest) {
        return Some(latest);
    }
    let mut versions = cx.machine.list_dir(&tools);
    versions.sort();
    versions
        .iter()
        .rev()
        .map(|version| tools.join(version).join("bin").join(&file))
        .find(|path| cx.machine.exists(path))
        .or_else(|| {
            let legacy = sdk.join("tools").join("bin").join(&file);
            cx.machine.exists(&legacy).then_some(legacy)
        })
}

/// An NDK found the way RayX xtask finds one: `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT`, `NDK_HOME`,
/// else the newest under `<sdk>/ndk`.
pub fn find_ndk(cx: &Cx, sdk: Option<&Path>) -> Option<PathBuf> {
    for name in ["ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "NDK_HOME"] {
        if let Some(value) = setting(cx, name) {
            let path = PathBuf::from(value);
            if cx.machine.is_dir(&path) {
                return Some(path);
            }
        }
    }
    let ndk_root = sdk?.join("ndk");
    let mut versions = cx.machine.list_dir(&ndk_root);
    versions.sort();
    versions.pop().map(|version| ndk_root.join(version))
}

fn sdk_step(env: &PlanEnv) -> Step {
    let os = env.host.os;
    Step::new(
        "android-sdk",
        Set::Android,
        "Android SDK command-line tools",
        Privilege::None,
        |cx| match find_sdk(cx) {
            Some(sdk) if find_sdkmanager(cx, &sdk).is_some() => Probed::ok(),
            Some(sdk) => Probed::missing(format!("no sdkmanager under {}", sdk.display())),
            None => Probed::missing("no Android SDK found"),
        },
        move |cx, _| {
            let sdk = find_sdk(cx)
                .or_else(|| default_sdk_dir(cx))
                .ok_or_else(|| {
                    SetupError::Prerequisite("the home directory is unknown".to_string())
                })?;
            let platform = match os {
                Os::Windows => "win",
                Os::MacOs => "mac",
                Os::Linux => "linux",
            };
            let url = format!(
                "https://dl.google.com/android/repository/commandlinetools-{platform}-{CMDLINE_TOOLS_BUILD}_latest.zip"
            );
            let temp = if os == Os::Windows {
                PathBuf::from(
                    cx.machine
                        .env("TEMP")
                        .unwrap_or_else(|| r"C:\Windows\Temp".into()),
                )
            } else {
                PathBuf::from("/tmp")
            };
            let zip = temp.join("rayx-commandlinetools.zip");
            let unpacked = sdk.join("cmdline-tools-download");
            let extract = if os == Os::Linux {
                CommandSpec::new("unzip")
                    .args(["-q", "-o", &zip.display().to_string(), "-d"])
                    .arg(unpacked.display().to_string())
            } else {
                CommandSpec::new("tar")
                    .args(["-xf", &zip.display().to_string(), "-C"])
                    .arg(unpacked.display().to_string())
            };
            let latest = sdk.join("cmdline-tools").join("latest");
            let (from, to, scratch) = (
                unpacked.join("cmdline-tools"),
                latest.clone(),
                unpacked.clone(),
            );
            Ok(vec![
                Action::CreateDir(sdk.join("cmdline-tools")),
                Action::CreateDir(unpacked.clone()),
                Action::Run(
                    CommandSpec::new(if os == Os::Windows {
                        "curl.exe"
                    } else {
                        "curl"
                    })
                    .args(["-fL", "--proto", "=https", "-o"])
                    .arg(zip.display().to_string())
                    .arg(url)
                    .interactive(),
                ),
                Action::Run(extract.interactive()),
                Action::Custom {
                    description: format!("move the unpacked cmdline-tools to {}", latest.display()),
                    run: Rc::new(move |cx: &mut Cx| {
                        if !cx.runner.is_dry_run() {
                            if to.exists() {
                                std::fs::remove_dir_all(&to)?;
                            }
                            std::fs::rename(&from, &to)?;
                            let _ = std::fs::remove_dir_all(&scratch);
                        }
                        Ok(())
                    }),
                },
                Action::RemoveFile(zip),
                Action::SetUserEnv {
                    name: "ANDROID_HOME".to_string(),
                    value: sdk,
                },
            ])
        },
    )
}

/// The SDK packages the set needs and which of them are missing.
fn wanted_packages(cx: &Cx, sdk: &Path) -> Vec<(String, bool)> {
    let pins = &cx.env.pins;
    let os = cx.env.host.os;
    let abi = host_abi(cx.env.host.arch);
    let platform = &pins.android_platform.value;
    let mut wanted = vec![
        (
            "platform-tools".to_string(),
            cx.machine
                .exists(&sdk.join("platform-tools").join(exe(os, "adb"))),
        ),
        (
            format!("platforms;{platform}"),
            cx.machine
                .exists(&sdk.join("platforms").join(platform).join("android.jar")),
        ),
        (
            format!("ndk;{}", pins.android_ndk.value),
            find_ndk(cx, Some(sdk)).is_some(),
        ),
    ];
    if let Some(build_tools) = &pins.android_build_tools {
        wanted.push((
            format!("build-tools;{}", build_tools.value),
            cx.machine
                .is_dir(&sdk.join("build-tools").join(&build_tools.value)),
        ));
    }
    if !cx.env.host.wsl {
        wanted.push((
            "emulator".to_string(),
            cx.machine
                .exists(&sdk.join("emulator").join(exe(os, "emulator"))),
        ));
        let image = &pins.android_system_image.value;
        let image_dir = image
            .split(';')
            .fold(sdk.join("system-images"), |dir, part| dir.join(part))
            .join(abi);
        wanted.push((
            format!("system-images;{image};{abi}"),
            cx.machine.is_dir(&image_dir),
        ));
    }
    wanted
}

fn sdk_packages_step() -> Step {
    Step::new(
        "android-packages",
        Set::Android,
        "Android SDK packages (platform-tools, platform, NDK, build-tools, emulator, system image)",
        Privilege::None,
        |cx| {
            let Some(sdk) = find_sdk(cx) else {
                return Probed::missing("no Android SDK found");
            };
            let missing = Probed::missing_items(
                wanted_packages(cx, &sdk)
                    .into_iter()
                    .filter(|(_, present)| !present)
                    .map(|(package, _)| package)
                    .collect(),
            );
            match find_ndk(cx, Some(&sdk)).and_then(|ndk| {
                ndk.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            }) {
                Some(ndk) => missing.with_found(format!("NDK {ndk}")),
                None => missing,
            }
        },
        |cx, probed| {
            let sdk = find_sdk(cx).ok_or_else(|| {
                SetupError::Prerequisite("the Android SDK was not found; install it first".into())
            })?;
            let manager = find_sdkmanager(cx, &sdk).ok_or_else(|| {
                SetupError::Prerequisite(format!(
                    "sdkmanager was not found under {}",
                    sdk.display()
                ))
            })?;
            let env = |spec: CommandSpec| {
                let mut spec = spec.env("ANDROID_HOME", sdk.display().to_string());
                if let Some(home) = setting(cx, "JAVA_HOME") {
                    spec = spec.env("JAVA_HOME", home);
                }
                spec.interactive()
            };
            let sdk_root = format!("--sdk_root={}", sdk.display());
            let mut licenses = cmdline_tool(&manager).args([sdk_root.as_str(), "--licenses"]);
            if cx.env.yes {
                // Licenses are accepted only on request; otherwise the prompt reaches the user.
                licenses = licenses.stdin("y\n".repeat(100));
            }
            let install = cmdline_tool(&manager)
                .arg(sdk_root.as_str())
                .arg("--install")
                .args(probed.missing.iter().cloned());
            Ok(vec![Action::Run(env(licenses)), Action::Run(env(install))])
        },
    )
    .with_prompt()
}

// ---------------------------------------------------------------------------------------------
// AVD and emulator acceleration

/// The first AVD name in `emulator -list-avds` output, ignoring the `INFO |`, `WARNING |`,
/// `ERROR |` and `DEBUG |` log lines the emulator can print on standard output (RayX xtask does
/// the same).
pub fn first_avd_name(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| {
        !line.is_empty()
            && !matches!(
                line.split_once('|').map(|(prefix, _)| prefix.trim()),
                Some("INFO" | "WARNING" | "ERROR" | "DEBUG")
            )
    })
}

/// The default AVD name: `rayx-<platform>-<abi>`.
pub fn avd_name(env: &PlanEnv) -> String {
    format!(
        "rayx-{}-{}",
        env.pins.android_platform.value,
        host_abi(env.host.arch)
    )
}

fn avd_step(env: &PlanEnv) -> Step {
    let wsl = env.host.wsl;
    Step::new(
        "android-avd",
        Set::Android,
        format!("default AVD {}", avd_name(env)),
        Privilege::None,
        move |cx| {
            if wsl {
                return Probed {
                    satisfied: true,
                    detail: "the emulator is provided by the Windows host".into(),
                    ..Probed::default()
                };
            }
            let Some(sdk) = find_sdk(cx) else {
                return Probed::missing("no Android SDK found");
            };
            let emulator = sdk.join("emulator").join(exe(cx.env.host.os, "emulator"));
            if !cx.machine.exists(&emulator) {
                return Probed::missing("the emulator is not installed yet");
            }
            let listed = cx
                .query(&CommandSpec::new(emulator.display().to_string()).arg("-list-avds"))
                .filter(|outcome| outcome.is_success())
                .is_some_and(|outcome| first_avd_name(&outcome.stdout).is_some());
            if listed {
                Probed::ok()
            } else {
                Probed::missing("no AVD exists")
            }
        },
        |cx, _| {
            let sdk = find_sdk(cx).ok_or_else(|| {
                SetupError::Prerequisite("the Android SDK was not found; install it first".into())
            })?;
            let manager = find_cmdline_tool(cx, &sdk, "avdmanager").ok_or_else(|| {
                SetupError::Prerequisite(format!(
                    "avdmanager was not found under {}",
                    sdk.display()
                ))
            })?;
            let image = format!(
                "system-images;{};{}",
                cx.env.pins.android_system_image.value,
                host_abi(cx.env.host.arch)
            );
            let mut create = cmdline_tool(&manager)
                .args([
                    "create",
                    "avd",
                    "--name",
                    &avd_name(cx.env),
                    "--package",
                    &image,
                ])
                .env("ANDROID_HOME", sdk.display().to_string())
                // "Do you wish to create a custom hardware profile?"
                .stdin("no\n")
                .interactive();
            if let Some(home) = setting(cx, "JAVA_HOME") {
                create = create.env("JAVA_HOME", home);
            }
            Ok(vec![Action::Run(create)])
        },
    )
}

fn acceleration_step(env: &PlanEnv) -> Step {
    let os = env.host.os;
    let wsl = env.host.wsl;
    let privilege = match os {
        Os::Linux => Privilege::Root,
        Os::Windows => Privilege::Admin,
        Os::MacOs => Privilege::None,
    };
    let step = Step::new(
        "android-acceleration",
        Set::Android,
        match os {
            Os::Linux => "emulator acceleration (/dev/kvm access)",
            Os::Windows => "emulator acceleration (Windows Hypervisor Platform)",
            Os::MacOs => "emulator acceleration (Hypervisor.framework)",
        },
        privilege,
        move |cx| {
            if wsl {
                return Probed {
                    satisfied: true,
                    detail: "the emulator is provided by the Windows host".into(),
                    ..Probed::default()
                };
            }
            match os {
                Os::MacOs => Probed {
                    satisfied: true,
                    detail: "built into macOS".into(),
                    ..Probed::default()
                },
                Os::Linux => {
                    if !cx.machine.exists(Path::new("/dev/kvm")) {
                        return Probed::missing(
                            "/dev/kvm does not exist: enable hardware virtualization",
                        );
                    }
                    let groups = cx
                        .query(&CommandSpec::new("id").arg("-nG"))
                        .map(|outcome| outcome.stdout)
                        .unwrap_or_default();
                    if groups.split_whitespace().any(|group| group == "kvm") {
                        Probed::ok()
                    } else {
                        Probed::missing("the user is not in the kvm group")
                    }
                }
                Os::Windows => {
                    let Some(sdk) = find_sdk(cx) else {
                        return Probed::missing("no Android SDK found");
                    };
                    let emulator = sdk.join("emulator").join("emulator.exe");
                    if !cx.machine.exists(&emulator) {
                        return Probed::missing("the emulator is not installed yet");
                    }
                    let check = cx.query(
                        &CommandSpec::new(emulator.display().to_string()).arg("-accel-check"),
                    );
                    let usable = check.is_some_and(|outcome| {
                        outcome.is_success()
                            && format!("{}{}", outcome.stdout, outcome.stderr).contains("usable")
                    });
                    if usable {
                        Probed::ok()
                    } else {
                        Probed::missing("the Windows Hypervisor Platform is not enabled")
                    }
                }
            }
        },
        move |cx, _| match os {
            Os::MacOs => Ok(Vec::new()),
            Os::Windows => Ok(vec![Action::Run(
                CommandSpec::new("dism")
                    .args([
                        "/online",
                        "/Enable-Feature",
                        "/FeatureName:HypervisorPlatform",
                        "/All",
                        "/NoRestart",
                    ])
                    .admin()
                    .interactive(),
            )]),
            Os::Linux => {
                if !cx.machine.exists(Path::new("/dev/kvm")) {
                    return Err(SetupError::Prerequisite(
                        "/dev/kvm does not exist: enable hardware virtualization in the firmware \
                         (and nested virtualization inside a virtual machine), then run \
                         `rayx setup --android` again"
                            .to_string(),
                    ));
                }
                let user = cx
                    .machine
                    .env("USER")
                    .or_else(|| cx.machine.env("LOGNAME"))
                    .ok_or_else(|| {
                        SetupError::Prerequisite("the user name is unknown".to_string())
                    })?;
                Ok(vec![Action::Run(
                    CommandSpec::new("usermod")
                        .args(["-aG", "kvm", user.as_str()])
                        .root()
                        .interactive(),
                )])
            }
        },
    );
    match os {
        Os::Windows => step.with_reboot_notice(
            "Restart Windows for the Windows Hypervisor Platform to take effect.",
        ),
        Os::Linux => step.with_reboot_notice(
            "Sign out and back in for the kvm group membership to take effect.",
        ),
        Os::MacOs => step,
    }
}

// ---------------------------------------------------------------------------------------------
// cargo-ndk and Rust targets

fn cargo_ndk_step() -> Step {
    Step::new(
        "cargo-ndk",
        Set::Android,
        "cargo-ndk",
        Privilege::None,
        |cx| {
            let cargo = rust::tool_program(cx, "cargo");
            let ok = cx
                .query(&CommandSpec::new(cargo).args(["ndk", "--version"]))
                .filter(|outcome| outcome.is_success());
            match ok {
                Some(outcome) => {
                    let version = outcome
                        .stdout
                        .split_whitespace()
                        .last()
                        .unwrap_or("installed")
                        .to_string();
                    Probed::ok().with_found(version)
                }
                None => Probed::missing("cargo-ndk is not installed"),
            }
        },
        |cx, _| {
            let cargo = rust::tool_program(cx, "cargo");
            Ok(vec![Action::Run(
                CommandSpec::new(cargo)
                    .args(["install", "cargo-ndk", "--locked"])
                    .interactive(),
            )])
        },
    )
}

fn rust_targets_step(env: &PlanEnv) -> Step {
    let toolchain = env
        .pins
        .rust_toolchain
        .as_ref()
        .map_or_else(|| "stable".to_string(), |pin| pin.value.clone());
    let probe_toolchain = toolchain.clone();
    Step::new(
        "android-rust-targets",
        Set::Android,
        format!("Rust Android targets on {toolchain}"),
        Privilege::None,
        move |cx| {
            let program = rust::rustup_program(cx);
            let spec = CommandSpec::new(program)
                .args([
                    "target",
                    "list",
                    "--toolchain",
                    probe_toolchain.as_str(),
                    "--installed",
                ])
                .env("RUSTUP_AUTO_INSTALL", "0");
            let Some(listing) = cx.query(&spec).filter(|o| o.is_success()) else {
                return Probed::missing(format!("{probe_toolchain} is not installed"));
            };
            Probed::missing_items(
                ANDROID_RUST_TARGETS
                    .iter()
                    .filter(|target| {
                        !listing
                            .stdout
                            .lines()
                            .any(|line| line.split_whitespace().next() == Some(**target))
                    })
                    .map(|target| target.to_string())
                    .collect(),
            )
        },
        move |cx, probed| {
            let program = rust::rustup_program(cx);
            Ok(vec![Action::Run(
                CommandSpec::new(program)
                    .args(["target", "add", "--toolchain", toolchain.as_str()])
                    .args(probed.missing.iter().cloned())
                    .interactive(),
            )])
        },
    )
}

pub fn steps(env: &PlanEnv) -> Result<Vec<Step>, SetupError> {
    let mut steps = vec![jdk_step(env), java_home_step(env), studio_step(env)];
    if env.host.os == Os::Linux {
        // `unzip` unpacks the downloaded command-line tools.
        steps.push(Step::apt(
            "android-unzip",
            Set::Android,
            "unzip (unpacks the Android command-line tools)",
            &["unzip"],
        ));
    }
    steps.extend([
        sdk_step(env),
        sdk_packages_step(),
        avd_step(env),
        acceleration_step(env),
    ]);
    steps.extend([cargo_ndk_step(), rust_targets_step(env)]);
    Ok(steps)
}
