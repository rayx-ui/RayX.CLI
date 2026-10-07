//! The iOS set (macOS only): Xcode, its selection, license and first launch, the Simulator
//! runtime, XcodeGen and the iOS Rust targets. Elsewhere the set reports that it does not apply.

use std::path::{Path, PathBuf};

use crate::host::{CommandSpec, Os, Privilege};

use super::{Action, Cx, PlanEnv, Probed, Set, SetupError, Step, macos, rust};

const XCODE_APP_STORE_ID: &str = "497799835";
const IOS_RUST_TARGETS: [&str; 2] = ["aarch64-apple-ios", "aarch64-apple-ios-sim"];

/// An Xcode application bundle and the version in its `Contents/version.plist`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XcodeApp {
    pub path: PathBuf,
    pub version: Option<String>,
}

/// The `CFBundleShortVersionString` of a `version.plist`.
pub fn plist_version(plist: &str) -> Option<String> {
    let after_key = plist
        .split("<key>CFBundleShortVersionString</key>")
        .nth(1)?;
    let start = after_key.find("<string>")? + "<string>".len();
    let end = after_key[start..].find("</string>")? + start;
    Some(after_key[start..end].trim().to_string())
}

fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// The Xcode apps in `/Applications` with their versions, newest version first. The version comes
/// from the bundle itself, so it is known without selecting the app or accepting its license.
pub fn xcode_apps(cx: &Cx) -> Vec<XcodeApp> {
    let root = Path::new("/Applications");
    let mut apps: Vec<XcodeApp> = cx
        .machine
        .list_dir(root)
        .into_iter()
        .filter(|name| name.starts_with("Xcode") && name.ends_with(".app"))
        .map(|name| {
            let path = root.join(name);
            let version = cx
                .machine
                .read_to_string(&path.join("Contents").join("version.plist"))
                .and_then(|plist| plist_version(&plist));
            XcodeApp { path, version }
        })
        .collect();
    apps.sort_by(|a, b| {
        let key = |app: &XcodeApp| app.version.as_deref().map(version_key);
        key(b).cmp(&key(a)).then_with(|| a.path.cmp(&b.path))
    });
    apps
}

/// Whether an installed `found` version satisfies the pin `want` (`26.1` matches `26.1.1`).
fn matches_pin(found: &str, want: &str) -> bool {
    found == want || found.starts_with(&format!("{want}."))
}

/// The app to use: the one matching the pin, else the newest.
pub fn chosen_xcode(cx: &Cx) -> Option<XcodeApp> {
    let apps = xcode_apps(cx);
    match &cx.env.pins.xcode {
        Some(pin) => apps.into_iter().find(|app| {
            app.version
                .as_deref()
                .is_some_and(|found| matches_pin(found, &pin.value))
        }),
        None => apps.into_iter().next(),
    }
}

fn brew(cx: &Cx) -> String {
    macos::brew_program(cx).map_or_else(|| "brew".to_string(), |path| path.display().to_string())
}

fn xcode_step(env: &PlanEnv) -> Step {
    let pinned = env.pins.xcode.as_ref().map(|pin| pin.value.clone());
    let probe_pinned = pinned.clone();
    let title = match &pinned {
        Some(version) => format!("Xcode {version}"),
        None => "Xcode (newest stable)".to_string(),
    };
    Step::new(
        "xcode",
        Set::Ios,
        title,
        Privilege::None,
        move |cx| {
            let apps = xcode_apps(cx);
            match &probe_pinned {
                Some(want) => {
                    let matching = apps.iter().find(|app| {
                        app.version
                            .as_deref()
                            .is_some_and(|found| matches_pin(found, want))
                    });
                    if let Some(app) = matching {
                        return Probed::ok().with_found(app.version.clone().unwrap_or_default());
                    }
                    match apps.iter().find_map(|app| app.version.clone()) {
                        Some(found) => Probed::missing(format!(
                            "Xcode {found} is installed, the project pins {want}"
                        ))
                        .with_found(found),
                        None => Probed::missing("Xcode is not installed"),
                    }
                }
                None => match apps.first() {
                    Some(app) => {
                        Probed::ok().with_found(app.version.clone().unwrap_or_else(|| "?".into()))
                    }
                    None => Probed::missing("Xcode is not installed"),
                },
            }
        },
        move |cx, _| {
            let mut actions = Vec::new();
            // The App Store route needs a signed-in account and no version pin.
            let mas_ready = pinned.is_none()
                && cx.machine.which("mas").is_some()
                && cx
                    .query(&CommandSpec::new("mas").arg("account"))
                    .is_some_and(|outcome| outcome.is_success());
            if mas_ready {
                actions.push(Action::Run(
                    CommandSpec::new("mas")
                        .args(["install", XCODE_APP_STORE_ID])
                        .interactive(),
                ));
                return Ok(actions);
            }
            if cx.machine.which("xcodes").is_none() {
                actions.push(Action::Run(
                    CommandSpec::new(brew(cx))
                        .args(["install", "xcodesorg/made/xcodes"])
                        .interactive(),
                ));
            }
            // `xcodes` asks for the Apple ID in the user's console; `rayx` never sees it.
            actions.push(Action::Run(
                match &pinned {
                    Some(version) => CommandSpec::new("xcodes").args(["install", version.as_str()]),
                    None => CommandSpec::new("xcodes").args(["install", "--latest"]),
                }
                .interactive(),
            ));
            Ok(actions)
        },
    )
}

fn developer_dir(app: &Path) -> PathBuf {
    app.join("Contents").join("Developer")
}

fn select_step() -> Step {
    Step::new(
        "xcode-select",
        Set::Ios,
        "xcode-select points at Xcode",
        Privilege::Root,
        |cx| {
            let selected = cx
                .query(&CommandSpec::new("xcode-select").arg("-p"))
                .filter(|outcome| outcome.is_success())
                .map(|outcome| outcome.stdout.trim().to_string());
            let wanted = chosen_xcode(cx).map(|app| developer_dir(&app.path));
            match (selected, wanted) {
                (Some(path), Some(wanted)) if Path::new(&path) == wanted => Probed::ok(),
                (Some(path), Some(wanted)) => {
                    Probed::missing(format!("selected: {path}, should be {}", wanted.display()))
                }
                // Xcode is not installed yet: the Xcode step comes first and reports that.
                (Some(path), None) => Probed::missing(format!("selected: {path}")),
                (None, _) => Probed::missing("no developer directory is selected"),
            }
        },
        |cx, _| {
            let app = chosen_xcode(cx).ok_or_else(|| {
                SetupError::Prerequisite(
                    "no Xcode.app matching the project's pin was found in /Applications; \
                     install Xcode first"
                        .to_string(),
                )
            })?;
            Ok(vec![Action::Run(
                CommandSpec::new("xcode-select")
                    .arg("--switch")
                    .arg(developer_dir(&app.path).display().to_string())
                    .root()
                    .interactive(),
            )])
        },
    )
}

fn license_step() -> Step {
    Step::new(
        "xcode-license",
        Set::Ios,
        "Xcode license",
        Privilege::Root,
        |cx| {
            let accepted = cx
                .query(&CommandSpec::new("xcodebuild").args(["-license", "check"]))
                .is_some_and(|outcome| outcome.is_success());
            if accepted {
                Probed::ok()
            } else {
                Probed::missing("the license is not accepted")
            }
        },
        |cx, _| {
            // The license is accepted non-interactively only on request.
            let mut license = CommandSpec::new("xcodebuild").arg("-license");
            if cx.env.yes {
                license = license.arg("accept");
            }
            Ok(vec![Action::Run(license.root().interactive())])
        },
    )
}

fn first_launch_step() -> Step {
    Step::new(
        "xcode-first-launch",
        Set::Ios,
        "Xcode first-launch components",
        Privilege::Root,
        |cx| {
            let done = cx
                .query(&CommandSpec::new("xcodebuild").arg("-checkFirstLaunchStatus"))
                .is_some_and(|outcome| outcome.is_success());
            if done {
                Probed::ok()
            } else {
                Probed::missing("first launch has not run")
            }
        },
        |_, _| {
            Ok(vec![Action::Run(
                CommandSpec::new("xcodebuild")
                    .arg("-runFirstLaunch")
                    .root()
                    .interactive(),
            )])
        },
    )
}

fn simulator_step() -> Step {
    Step::new(
        "ios-simulator",
        Set::Ios,
        "iOS Simulator runtime",
        Privilege::None,
        |cx| {
            let listed = cx
                .query(&CommandSpec::new("xcrun").args(["simctl", "list", "runtimes"]))
                .filter(|outcome| outcome.is_success())
                .is_some_and(|outcome| outcome.stdout.lines().any(|l| l.contains("iOS")));
            if listed {
                Probed::ok()
            } else {
                Probed::missing("no iOS Simulator runtime is installed")
            }
        },
        |_, _| {
            Ok(vec![Action::Run(
                CommandSpec::new("xcodebuild")
                    .args(["-downloadPlatform", "iOS"])
                    .interactive(),
            )])
        },
    )
}

fn xcodegen_step() -> Step {
    Step::new(
        "xcodegen",
        Set::Ios,
        "XcodeGen",
        Privilege::None,
        |cx| {
            if cx.machine.which("xcodegen").is_some() {
                Probed::ok()
            } else {
                Probed::missing("xcodegen is not installed")
            }
        },
        |cx, _| {
            Ok(vec![Action::Run(
                CommandSpec::new(brew(cx))
                    .args(["install", "xcodegen"])
                    .interactive(),
            )])
        },
    )
}

fn ios_targets_step(env: &PlanEnv) -> Step {
    let toolchain = env
        .pins
        .rust_toolchain
        .as_ref()
        .map_or_else(|| "stable".to_string(), |pin| pin.value.clone());
    let probe_toolchain = toolchain.clone();
    Step::new(
        "ios-rust-targets",
        Set::Ios,
        format!("Rust iOS targets on {toolchain}"),
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
                IOS_RUST_TARGETS
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
    if env.host.os != Os::MacOs {
        // Reported, not an error: iOS tooling needs a Mac.
        return Ok(vec![Step::new(
            "ios-not-applicable",
            Set::Ios,
            "iOS tooling",
            Privilege::None,
            |_| Probed {
                satisfied: true,
                detail: "not applicable on this host: iOS tooling needs macOS".into(),
                ..Probed::default()
            },
            |_, _| Ok(Vec::new()),
        )]);
    }
    Ok(vec![
        xcode_step(env),
        select_step(),
        license_step(),
        first_launch_step(),
        simulator_step(),
        xcodegen_step(),
        ios_targets_step(env),
    ])
}
