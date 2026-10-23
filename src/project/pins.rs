//! The tool versions `rayx` installs and builds with.
//!
//! Pins belong to the project. They resolve in this order and each carries its source:
//!
//! 1. the project: `Cargo.lock`, `rust-toolchain.toml` and `[workspace.metadata.rayx]`;
//! 2. the built-in defaults in [`DEFAULTS`], the only place a default version is written.

use std::fmt;
use std::path::Path;

use super::Project;

/// Where a pin came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinSource {
    /// The project's own files.
    Project,
    /// The CLI's built-in default.
    Default,
}

impl PinSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PinSource::Project => "project",
            PinSource::Default => "default",
        }
    }
}

impl fmt::Display for PinSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A resolved version with its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub value: String,
    pub source: PinSource,
}

impl Pin {
    fn new(value: impl Into<String>, source: PinSource) -> Self {
        Self {
            value: value.into(),
            source,
        }
    }
}

/// The built-in defaults: the versions this release of `rayx` was tested with.
pub struct Defaults {
    pub node: &'static str,
    pub jdk: &'static str,
    /// The NDK nsh's Android build uses.
    pub android_ndk: &'static str,
    /// nsh's `TARGET_SDK`: the platform its Android hosts compile against.
    pub android_platform: &'static str,
    /// The build-tools nsh runs (`aapt2`, `zipalign`, `apksigner`).
    pub android_build_tools: &'static str,
    /// The tag of the emulator system image (`google_apis`); the image is `<platform>;<tag>` of
    /// the platform pin, with the host architecture added at install.
    pub android_image_tag: &'static str,
    /// The Ubuntu version of every guest `rayx` creates (WSL distribution, Lima VM). Guests serve
    /// every project on the machine, so a project cannot override it.
    pub ubuntu: &'static str,
}

/// The one table of default versions.
pub const DEFAULTS: Defaults = Defaults {
    node: "22",
    jdk: "21",
    android_ndk: "28.0.12674087",
    android_platform: "android-35",
    android_build_tools: "35.0.1",
    android_image_tag: "google_apis",
    ubuntu: "24.04",
};

/// Every version `setup`, `doctor` and `rayx app` need, with where each came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pins {
    /// `wasm-bindgen` from the project's `Cargo.lock`; none outside a project that locks it.
    pub wasm_bindgen: Option<Pin>,
    /// The `channel` of the project's `rust-toolchain.toml`.
    pub rust_toolchain: Option<Pin>,
    pub node: Pin,
    pub jdk: Pin,
    pub android_ndk: Pin,
    pub android_platform: Pin,
    pub android_build_tools: Pin,
    pub android_system_image: Pin,
    /// An exact Xcode version; none means the newest stable Xcode.
    pub xcode: Option<Pin>,
    /// The guest Ubuntu version; always the built-in default.
    pub ubuntu: Pin,
}

impl Pins {
    /// Resolves the pins of `project`, or only the defaults outside any project.
    pub fn resolve(project: Option<&Project>) -> Pins {
        let metadata = project.and_then(|p| p.rayx_metadata.as_ref());
        let key = |name: &str| metadata.and_then(|table| string_value(table.get(name)));

        // A key from the project, else the default.
        let pin = |name: &str, default: &str| -> Pin {
            match key(name) {
                Some(value) => Pin::new(value, PinSource::Project),
                None => Pin::new(default, PinSource::Default),
            }
        };

        let (wasm_bindgen, rust_toolchain) = match project {
            Some(project) => (
                locked_version(&project.workspace_root, "wasm-bindgen")
                    .map(|v| Pin::new(v, PinSource::Project)),
                toolchain_channel(&project.app_dir, &project.workspace_root)
                    .map(|v| Pin::new(v, PinSource::Project)),
            ),
            None => (None, None),
        };

        let android_platform = pin("android-platform", DEFAULTS.android_platform);

        Pins {
            wasm_bindgen,
            rust_toolchain,
            node: pin("node", DEFAULTS.node),
            jdk: pin("jdk", DEFAULTS.jdk),
            android_ndk: pin("android-ndk", DEFAULTS.android_ndk),
            android_platform: android_platform.clone(),
            android_build_tools: pin("android-build-tools", DEFAULTS.android_build_tools),
            // Unless set explicitly, the image is at the platform pin, whose source it reports.
            android_system_image: match key("android-system-image") {
                Some(value) => Pin::new(value, PinSource::Project),
                None => Pin::new(
                    format!("{};{}", android_platform.value, DEFAULTS.android_image_tag),
                    android_platform.source,
                ),
            },
            xcode: key("xcode").map(|v| Pin::new(v, PinSource::Project)),
            ubuntu: Pin::new(DEFAULTS.ubuntu, PinSource::Default),
        }
    }
}

/// A TOML/JSON string or integer as text.
fn string_value(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// The version `Cargo.lock` locks for `package`, if it locks one.
fn locked_version(workspace_root: &Path, package: &str) -> Option<String> {
    let text = std::fs::read_to_string(workspace_root.join("Cargo.lock")).ok()?;
    let lock: toml::Table = text.parse().ok()?;
    lock.get("package")?
        .as_array()?
        .iter()
        .filter_map(toml::Value::as_table)
        .find(|entry| entry.get("name").and_then(toml::Value::as_str) == Some(package))
        .and_then(|entry| entry.get("version")?.as_str())
        .map(str::to_string)
}

/// The `channel` of the nearest `rust-toolchain.toml` (or legacy `rust-toolchain`) from `start`
/// up to `ceiling`.
fn toolchain_channel(start: &Path, ceiling: &Path) -> Option<String> {
    let mut dir = Some(start);
    while let Some(current) = dir {
        let toml_file = current.join("rust-toolchain.toml");
        if let Ok(text) = std::fs::read_to_string(&toml_file) {
            let table: toml::Table = text.parse().ok()?;
            return table
                .get("toolchain")?
                .get("channel")?
                .as_str()
                .map(str::to_string);
        }
        if let Ok(text) = std::fs::read_to_string(current.join("rust-toolchain")) {
            let channel = text.trim();
            return (!channel.is_empty() && !channel.contains('\n')).then(|| channel.to_string());
        }
        if current == ceiling {
            break;
        }
        dir = current.parent();
    }
    None
}
