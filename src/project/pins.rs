//! The tool versions `rayx` installs and builds with.
//!
//! Pins belong to the project. They resolve in this order and each carries its source:
//!
//! 1. the project: `Cargo.lock`, `rust-toolchain.toml` and `[workspace.metadata.rayx]`;
//! 2. the gpux-checkout profile, for a workspace that contains `gpux-testkit` and declares
//!    nothing for that pin;
//! 3. the built-in defaults in [`DEFAULTS`], the only place a default version is written.

use std::fmt;
use std::path::Path;

use super::Project;

/// Where a pin came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinSource {
    /// The project's own files.
    Project,
    /// The gpux checkout's profile.
    GpuxCheckout,
    /// The CLI's built-in default.
    Default,
}

impl PinSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PinSource::Project => "project",
            PinSource::GpuxCheckout => "gpux-checkout",
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

/// The built-in defaults. They equal the pins RayX and Gpux use; move them together with the RayX
/// skill reference `wasm-toolchain-update.md`.
pub struct Defaults {
    pub web_toolchain: &'static str,
    pub node: &'static str,
    pub jdk: &'static str,
    pub playwright: &'static str,
    pub android_ndk: &'static str,
    pub android_platform: &'static str,
    pub android_ndk_api: &'static str,
    /// `<platform>;<tag>` of the emulator system image; the host architecture is added at install.
    pub android_system_image: &'static str,
}

/// The one table of default versions.
pub const DEFAULTS: Defaults = Defaults {
    web_toolchain: "nightly-2026-06-23",
    node: "22",
    jdk: "21",
    playwright: "1.59.1",
    android_ndk: "28.0.12674087",
    android_platform: "android-34",
    android_ndk_api: "31",
    android_system_image: "android-34;google_apis",
};

/// What a gpux checkout pins for Android (its lane E).
pub struct GpuxProfile {
    pub android_platform: &'static str,
    pub android_build_tools: &'static str,
    pub android_ndk: &'static str,
}

/// The gpux-checkout profile, for a workspace that contains `gpux-testkit`.
pub const GPUX_CHECKOUT: GpuxProfile = GpuxProfile {
    android_platform: "android-36",
    android_build_tools: "36.0.0",
    android_ndk: "28.0.12674087",
};

/// Every version `setup`, `doctor` and the app pipeline need, with where each came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pins {
    /// `wasm-bindgen` from the project's `Cargo.lock`; none outside a project that locks it.
    pub wasm_bindgen: Option<Pin>,
    /// The `channel` of the project's `rust-toolchain.toml`.
    pub rust_toolchain: Option<Pin>,
    /// The nightly that builds the threaded WASM app.
    pub web_toolchain: Pin,
    pub node: Pin,
    pub jdk: Pin,
    pub playwright: Pin,
    pub android_ndk: Pin,
    pub android_platform: Pin,
    pub android_build_tools: Option<Pin>,
    pub android_ndk_api: Pin,
    pub android_system_image: Pin,
    /// An exact Xcode version; none means the newest stable Xcode.
    pub xcode: Option<Pin>,
}

impl Pins {
    /// Resolves the pins of `project`, or only the defaults outside any project.
    pub fn resolve(project: Option<&Project>) -> Pins {
        let metadata = project.and_then(|p| p.rayx_metadata.as_ref());
        let key = |name: &str| metadata.and_then(|table| string_value(table.get(name)));
        let gpux = project.is_some_and(Project::is_gpux_checkout);

        // A key from the project, else the gpux profile (when it has one), else the default.
        let pin = |name: &str, gpux_value: Option<&str>, default: &str| -> Pin {
            if let Some(value) = key(name) {
                Pin::new(value, PinSource::Project)
            } else if let (true, Some(value)) = (gpux, gpux_value) {
                Pin::new(value, PinSource::GpuxCheckout)
            } else {
                Pin::new(default, PinSource::Default)
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

        Pins {
            wasm_bindgen,
            rust_toolchain,
            web_toolchain: pin("web-toolchain", None, DEFAULTS.web_toolchain),
            node: pin("node", None, DEFAULTS.node),
            jdk: pin("jdk", None, DEFAULTS.jdk),
            playwright: pin("playwright", None, DEFAULTS.playwright),
            android_ndk: pin(
                "android-ndk",
                Some(GPUX_CHECKOUT.android_ndk),
                DEFAULTS.android_ndk,
            ),
            android_platform: pin(
                "android-platform",
                Some(GPUX_CHECKOUT.android_platform),
                DEFAULTS.android_platform,
            ),
            android_build_tools: if let Some(value) = key("android-build-tools") {
                Some(Pin::new(value, PinSource::Project))
            } else if gpux {
                Some(Pin::new(
                    GPUX_CHECKOUT.android_build_tools,
                    PinSource::GpuxCheckout,
                ))
            } else {
                None
            },
            android_ndk_api: pin("android-ndk-api", None, DEFAULTS.android_ndk_api),
            android_system_image: pin("android-system-image", None, DEFAULTS.android_system_image),
            xcode: key("xcode").map(|v| Pin::new(v, PinSource::Project)),
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
