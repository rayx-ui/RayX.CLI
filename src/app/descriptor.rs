use crate::app::args::{take_bool_flag, take_flag_value};
use crate::app::context::{ProjectContext, dependency_toml};
use crate::app::fs_util::command_path;
use anyhow::{Context, Result, anyhow, bail};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use toml::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    pub fn cargo_arg(self) -> Option<&'static str> {
        match self {
            Self::Debug => None,
            Self::Release => Some("--release"),
        }
    }

    pub fn gradle_task(self) -> &'static str {
        match self {
            Self::Debug => "assembleDebug",
            Self::Release => "assembleRelease",
        }
    }

    pub fn xcode_configuration(self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Release => "Release",
        }
    }

    pub fn is_release(self) -> bool {
        self == Self::Release
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CargoMetadataContext {
    pub target: Option<String>,
    pub features: Vec<String>,
    pub all_features: bool,
    pub no_default_features: bool,
}

impl CargoMetadataContext {
    pub fn apply_to(&self, command: &mut Command) -> Result<()> {
        if self.all_features && self.no_default_features {
            bail!("Cargo metadata context cannot combine all-features with no-default-features");
        }
        if let Some(target) = self.target.as_deref() {
            command.args(["--filter-platform", target]);
        }
        let features = self
            .features
            .iter()
            .map(|feature| feature.trim())
            .filter(|feature| !feature.is_empty())
            .collect::<std::collections::BTreeSet<_>>();
        if !features.is_empty() {
            command
                .arg("--features")
                .arg(features.into_iter().collect::<Vec<_>>().join(","));
        }
        if self.all_features {
            command.arg("--all-features");
        }
        if self.no_default_features {
            command.arg("--no-default-features");
        }
        Ok(())
    }
}

/// Normalized Cargo feature switches shared by every xtask app action.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppFeatureSelection {
    pub features: Vec<String>,
    pub all_features: bool,
    pub no_default_features: bool,
}

impl AppFeatureSelection {
    pub fn take_from_args(args: &mut Vec<String>) -> Result<Self> {
        let mut app_args = args
            .iter()
            .position(|arg| arg == "--")
            .map(|separator| args.split_off(separator))
            .unwrap_or_default();
        let all_features = take_bool_flag(args, "--all-features");
        let no_default_features = take_bool_flag(args, "--no-default-features");
        if all_features && no_default_features {
            bail!("--all-features cannot be combined with --no-default-features");
        }

        let mut features = Vec::new();
        while let Some(feature_set) = take_flag_value(args, "--features")? {
            let normalized = feature_set
                .split([',', ' ', '\t'])
                .map(str::trim)
                .filter(|feature| !feature.is_empty())
                .collect::<Vec<_>>();
            if normalized.is_empty() {
                bail!("--features expects at least one feature name");
            }
            features.extend(normalized.into_iter().map(str::to_owned));
        }
        let selection = Self {
            features,
            all_features,
            no_default_features,
        };
        args.append(&mut app_args);
        Ok(selection)
    }

    pub fn default_features_enabled(&self) -> bool {
        !self.no_default_features
    }

    pub fn normalized_features(&self) -> Vec<String> {
        self.features
            .iter()
            .flat_map(|value| value.split([',', ' ', '\t']))
            .map(str::trim)
            .filter(|feature| !feature.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    pub fn validate_for(&self, manifest: &Path) -> Result<()> {
        let value = read_manifest(manifest)?;
        let known = value
            .get("features")
            .and_then(Value::as_table)
            .map(|features| features.keys().cloned().collect::<BTreeSet<_>>())
            .unwrap_or_default();
        for feature in self.normalized_features() {
            if !known.contains(&feature) {
                bail!(
                    "app manifest {} does not define feature {feature}",
                    manifest.display()
                );
            }
        }
        Ok(())
    }

    pub fn metadata_context(&self) -> CargoMetadataContext {
        CargoMetadataContext {
            features: self.normalized_features(),
            all_features: self.all_features,
            no_default_features: self.no_default_features,
            ..CargoMetadataContext::default()
        }
    }

    pub fn apply_to_cargo(&self, command: &mut Command) -> Result<()> {
        self.metadata_context().apply_to(command)
    }
}

#[derive(Clone, Debug)]
pub struct AppDescriptor {
    pub root: PathBuf,
    pub slug: String,
    pub lib_name: String,
    pub rust_lib_name: String,
    pub android_application_id: Option<String>,
    pub android_activity: Option<String>,
    pub ios_project: Option<String>,
    pub ios_scheme: Option<String>,
    pub ios_bundle_id: Option<String>,
    pub ios_app_name: Option<String>,
    pub diagnostic_harness: bool,
    /// The workspace, pins and host this app is built in.
    pub context: ProjectContext,
}

impl AppDescriptor {
    /// The app in `app_dir`, a path absolute or relative to the current directory.
    pub fn resolve(context: &ProjectContext, app_dir: &str) -> Result<Self> {
        let base = std::env::current_dir().context("reading the current directory")?;
        Self::resolve_from(context, &base, app_dir)
    }

    /// The app in `app_dir`, a path absolute or relative to `base`.
    pub fn resolve_from(context: &ProjectContext, base: &Path, app_dir: &str) -> Result<Self> {
        let requested = PathBuf::from(app_dir);
        let root = if requested.is_absolute() {
            requested
        } else {
            base.join(requested)
        };
        let root = fs::canonicalize(&root)
            .with_context(|| format!("app directory {} was not found", root.display()))?;
        if !root.is_dir() {
            bail!("{} is not an app directory", root.display());
        }

        let manifest = root.join("Cargo.toml");
        if !manifest.is_file() {
            bail!("app directory {} has no Cargo.toml", root.display());
        }
        let manifest_value = read_manifest(&manifest)?;
        let package_name = package_name_from_value(&manifest_value, &manifest)?;
        let slug = metadata_string(&manifest_value, "app_name")
            .or_else(|| metadata_string(&manifest_value, "app_slug"))
            .unwrap_or_else(|| package_name.clone());
        let rust_lib_name = lib_name_from_value(&manifest_value)
            .unwrap_or_else(|| rust_crate_file_stem(&package_name));
        let lib_name = metadata_string(&manifest_value, "native_library")
            .or_else(|| metadata_string(&manifest_value, "android_native_library"))
            .unwrap_or_else(|| rust_lib_name.clone());

        Ok(Self {
            root,
            slug: sanitize_slug(&slug),
            lib_name,
            rust_lib_name,
            android_application_id: metadata_string(&manifest_value, "android_application_id"),
            android_activity: metadata_string(&manifest_value, "android_activity"),
            ios_project: metadata_string(&manifest_value, "ios_project"),
            ios_scheme: metadata_string(&manifest_value, "ios_scheme"),
            ios_bundle_id: metadata_string(&manifest_value, "ios_bundle_id"),
            ios_app_name: metadata_string(&manifest_value, "ios_app_name"),
            diagnostic_harness: metadata_bool(&manifest_value, "diagnostic_harness"),
            context: context.clone(),
        })
    }

    pub fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    /// `[package.metadata.rayx] playwright_specs`: the spec files or directories, relative to the
    /// Playwright package, that `test wasm` runs for this app.
    pub fn playwright_specs(&self) -> Result<Option<Vec<String>>> {
        Ok(metadata_string_array(
            &read_manifest(&self.manifest())?,
            "playwright_specs",
        ))
    }

    /// `[package.metadata.rayx] playwright_projects`: the Playwright projects `test wasm` runs.
    pub fn playwright_projects(&self) -> Result<Option<Vec<String>>> {
        Ok(metadata_string_array(
            &read_manifest(&self.manifest())?,
            "playwright_projects",
        ))
    }

    /// Directory holding bootstrap app content such as `app_settings.json`:
    /// `[package.metadata.rayx] app_content_root`, or the app root itself.
    pub fn app_content_root(&self) -> Result<PathBuf> {
        let manifest_value = read_manifest(&self.manifest())?;
        let root = match metadata_string(&manifest_value, "app_content_root") {
            Some(root) => resolve_app_path(&self.root, root),
            None => self.root.clone(),
        };
        Ok(root)
    }

    pub fn asset_roots(&self) -> Result<Vec<PathBuf>> {
        let manifest_value = read_manifest(&self.manifest())?;
        let roots = metadata_string_array(&manifest_value, "asset_roots")
            .or_else(|| metadata_string_array(&manifest_value, "assets_roots"))
            .map(|roots| {
                roots
                    .into_iter()
                    .map(|root| resolve_app_path(&self.root, root))
                    .collect::<Vec<_>>()
            })
            .or_else(|| {
                metadata_string(&manifest_value, "assets_root")
                    .map(|root| vec![resolve_app_path(&self.root, root)])
            })
            .unwrap_or_else(|| vec![self.assets_dir()]);
        Ok(roots)
    }

    pub fn asset_manifest_path(&self) -> Result<PathBuf> {
        let manifest_value = read_manifest(&self.manifest())?;
        let manifest_path = metadata_string(&manifest_value, "wasm_manifest_path")
            .unwrap_or_else(|| "index.json".to_string());
        let manifest_path = PathBuf::from(manifest_path);
        if manifest_path.is_absolute() {
            Ok(manifest_path)
        } else {
            let root = self
                .asset_roots()?
                .into_iter()
                .next()
                .unwrap_or_else(|| self.assets_dir());
            Ok(root.join(manifest_path))
        }
    }

    #[cfg(test)]
    pub fn platform_manifest(&self, target: &str) -> PathBuf {
        self.root.join("platform").join(target).join("Cargo.toml")
    }

    pub fn android_rust_manifest(&self, features: &AppFeatureSelection) -> Result<PathBuf> {
        self.write_generated_android_rust_manifest(features)
    }

    pub fn ios_rust_manifest(&self, features: &AppFeatureSelection) -> Result<PathBuf> {
        self.write_generated_ios_rust_manifest(features)
    }

    pub fn android_gradle_dir(&self) -> PathBuf {
        self.root.join("platform").join("android").join("gradle")
    }

    pub fn require_android_gradle_dir(&self) -> Result<PathBuf> {
        let dir = self.android_gradle_dir();
        if dir.is_dir() {
            Ok(dir)
        } else {
            bail!(
                "app {} has no Android Gradle wrapper at {}",
                self.root.display(),
                dir.display()
            )
        }
    }

    pub fn ios_dir(&self) -> PathBuf {
        self.root.join("platform").join("ios")
    }

    pub fn require_ios_dir(&self) -> Result<PathBuf> {
        let dir = self.ios_dir();
        if dir.is_dir() {
            Ok(dir)
        } else {
            bail!(
                "app {} has no iOS wrapper at {}",
                self.root.display(),
                dir.display()
            )
        }
    }

    pub fn web_manifest(&self, features: &AppFeatureSelection) -> Result<PathBuf> {
        self.write_generated_web_rust_manifest(features)
    }

    pub fn web_module_base(&self) -> String {
        self.generated_web_lib_name()
    }

    /// Where packaged output goes: `artifacts/apps/<slug>` under the workspace root.
    pub fn artifact_root(&self) -> Result<PathBuf> {
        Ok(self
            .context
            .workspace_root()
            .join("artifacts")
            .join("apps")
            .join(&self.slug))
    }

    /// Generated files: `artifacts-temp/apps/<slug>` under the workspace root.
    pub fn temp_root(&self) -> Result<PathBuf> {
        Ok(self
            .context
            .workspace_root()
            .join("artifacts-temp")
            .join("apps")
            .join(&self.slug))
    }

    /// The Cargo target directory of the workspace.
    pub fn target_dir(&self) -> Result<PathBuf> {
        Ok(self.context.workspace_root().join("target"))
    }

    /// The workspace package of this app.
    pub fn package(&self) -> Result<&crate::project::Package> {
        self.context.package_in(&self.root).ok_or_else(|| {
            anyhow!(
                "{} is not a package of the workspace at {}",
                self.root.display(),
                self.context.workspace_root().display()
            )
        })
    }

    pub fn android_generated_jni_dir(&self) -> Result<PathBuf> {
        Ok(self
            .require_android_gradle_dir()?
            .join("app")
            .join("build")
            .join("generated")
            .join("rustJniLibs"))
    }

    fn generated_android_rust_dir(&self) -> Result<PathBuf> {
        Ok(self.temp_root()?.join("android").join("rust"))
    }

    fn write_generated_android_rust_manifest(
        &self,
        features: &AppFeatureSelection,
    ) -> Result<PathBuf> {
        self.write_generated_rust_manifest(
            self.generated_android_rust_dir()?,
            "android",
            &self.lib_name,
            &["cdylib"],
            false,
            features,
        )
    }

    fn generated_ios_rust_dir(&self) -> Result<PathBuf> {
        Ok(self.temp_root()?.join("ios").join("rust"))
    }

    fn write_generated_ios_rust_manifest(&self, features: &AppFeatureSelection) -> Result<PathBuf> {
        self.write_generated_rust_manifest(
            self.generated_ios_rust_dir()?,
            "ios",
            &self.lib_name,
            &["staticlib"],
            false,
            features,
        )
    }

    fn generated_web_rust_dir(&self) -> Result<PathBuf> {
        Ok(self.temp_root()?.join("wasm").join("rust"))
    }

    fn generated_web_lib_name(&self) -> String {
        format!("{}_web", self.rust_lib_name)
    }

    fn write_generated_web_rust_manifest(&self, features: &AppFeatureSelection) -> Result<PathBuf> {
        self.write_generated_rust_manifest(
            self.generated_web_rust_dir()?,
            "web",
            &self.generated_web_lib_name(),
            &["cdylib", "rlib"],
            false,
            features,
        )
    }

    fn write_generated_rust_manifest(
        &self,
        root: PathBuf,
        target: &str,
        lib_name: &str,
        crate_types: &[&str],
        app_default_features: bool,
        selected_features: &AppFeatureSelection,
    ) -> Result<PathBuf> {
        let src_dir = root.join("src");
        fs::create_dir_all(&src_dir).with_context(|| format!("creating {}", src_dir.display()))?;

        let manifest = root.join("Cargo.toml");
        let app_manifest = self.manifest();
        let package_name = package_name_from_manifest(&app_manifest)?;
        let (enabled_app_features, generated_default_features) = if app_default_features {
            (manifest_default_features(&app_manifest)?, Vec::new())
        } else {
            selected_features.validate_for(&app_manifest)?;
            let mut features = if selected_features.all_features {
                manifest_feature_names(&app_manifest)?
            } else if selected_features.default_features_enabled() {
                manifest_default_features(&app_manifest)?
            } else {
                Vec::new()
            };
            features.extend(selected_features.normalized_features());
            let enabled_app_features = features
                .into_iter()
                .filter(|feature| !feature.starts_with("dep:") && !feature.contains('/'))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let generated_default_features = enabled_app_features
                .iter()
                .map(|feature| format!("{}/{feature}", self.rust_lib_name))
                .collect::<Vec<_>>();
            (enabled_app_features, generated_default_features)
        };
        let generated_devkit_dependency =
            manifest_feature_contains(&app_manifest, "devkit", "dep:rayx_devkit")?;
        let mut feature_lines = manifest_feature_names(&app_manifest)?
            .into_iter()
            .filter(|feature| feature != "devkit")
            .map(|feature| format!("{feature} = [\"{}/{feature}\"]", self.rust_lib_name))
            .collect::<Vec<_>>();
        let devkit_feature_line = if manifest_has_feature(&app_manifest, "devkit")? {
            let mut dependencies = vec![format!("{}/devkit", self.rust_lib_name)];
            if generated_devkit_dependency {
                dependencies.push("dep:rayx_devkit".to_string());
            }
            let dependencies = dependencies
                .into_iter()
                .map(|dependency| format!("\"{dependency}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!("devkit = [{dependencies}]")
        } else {
            "devkit = []".to_string()
        };
        feature_lines.push(devkit_feature_line);
        // The entry crate depends on `rayx` (and `rayx_devkit`) from the same source the app does.
        let package = self.package()?;
        let rayx = package.dependency("rayx").ok_or_else(|| {
            anyhow!(
                "app {} does not depend on `rayx`, which its generated {target} entry crate needs",
                self.root.display()
            )
        })?;
        let rayx_dependency = dependency_toml(rayx, false)?;
        let app_path = cargo_toml_path(&self.root);
        let generated_devkit_dependency_line = if generated_devkit_dependency {
            let devkit = package.dependency("rayx_devkit").ok_or_else(|| {
                anyhow!(
                    "app {} enables `dep:rayx_devkit` but declares no `rayx_devkit` dependency",
                    self.root.display()
                )
            })?;
            format!("rayx_devkit = {}\n", dependency_toml(devkit, true)?)
        } else {
            String::new()
        };
        // `rclite` (through `rxrust`) uses `branches` without `std`, whose nightly path calls
        // `core::intrinsics::abort`, which current nightlies removed. With `std` it aborts through
        // `std::process::abort`, which the threaded build's `build-std` provides.
        let generated_web_dependency_lines = if target == "web" {
            let bindgen = self.context.pins.wasm_bindgen.as_ref().ok_or_else(|| {
                anyhow!(
                    "wasm-bindgen is not locked in {}: build once with `cargo build` so Cargo.lock names it",
                    self.context.workspace_root().join("Cargo.lock").display()
                )
            })?;
            format!(
                "wasm-bindgen = \"={}\"\nwasm-bindgen-futures = \"0.4\"\nzune-core = \"=0.5.1\"\nbranches = {{ version = \"0.4\", features = [\"std\"] }}\n",
                bindgen.value
            )
        } else {
            String::new()
        };
        let generated_web_patch_lines = if target == "web" {
            generated_web_patch_lines(
                &self.context,
                manifest_has_active_dependency(
                    &app_manifest,
                    "rayx_storage_surrealdb",
                    &enabled_app_features,
                )?,
            )?
        } else {
            String::new()
        };
        let crate_type_line = crate_types
            .iter()
            .map(|crate_type| format!("\"{crate_type}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let app_dependency_line = if app_default_features {
            format!(
                "{} = {{ package = \"{}\", path = \"{}\" }}",
                self.rust_lib_name, package_name, app_path
            )
        } else {
            format!(
                "{} = {{ package = \"{}\", path = \"{}\", default-features = false }}",
                self.rust_lib_name, package_name, app_path
            )
        };
        let manifest_text = format!(
            r#"[package]
name = "{}-{}-entry"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
name = "{}"
crate-type = [{}]
path = "src/lib.rs"

[workspace]

[features]
default = [{}]
{}

[dependencies]
rayx = {}
{}{}{}
{}
"#,
            self.slug,
            target,
            lib_name,
            crate_type_line,
            generated_default_features
                .iter()
                .map(|feature| format!("\"{feature}\""))
                .collect::<Vec<_>>()
                .join(", "),
            feature_lines.join("\n"),
            rayx_dependency,
            generated_devkit_dependency_line,
            generated_web_dependency_lines,
            app_dependency_line,
            generated_web_patch_lines
        );
        fs::write(&manifest, manifest_text)
            .with_context(|| format!("writing {}", manifest.display()))?;
        let lockfile = root.join("Cargo.lock");
        if lockfile.is_file() {
            fs::remove_file(&lockfile)
                .with_context(|| format!("removing stale {}", lockfile.display()))?;
        }

        let main_path = cargo_toml_path(&self.root.join("src").join("main.rs"));
        let source = format!(
            r#"#![allow(dead_code)]

#[path = "{}"]
mod rayx_app_main;
"#,
            main_path
        );
        fs::write(src_dir.join("lib.rs"), source)
            .with_context(|| format!("writing {}", src_dir.join("lib.rs").display()))?;
        Ok(manifest)
    }

    pub fn android_launch_component(&self) -> Result<String> {
        let application_id = self.android_application_id.as_deref().ok_or_else(|| {
            anyhow!(
                "app {} must set package.metadata.rayx.android_application_id for Android launch",
                self.root.display()
            )
        })?;
        let activity = self
            .android_activity
            .as_deref()
            .unwrap_or("dev.rayx.mobile.RayXActivity");
        Ok(format!("{application_id}/{activity}"))
    }

    pub fn ios_scheme(&self) -> Result<&str> {
        self.ios_scheme.as_deref().ok_or_else(|| {
            anyhow!(
                "app {} must set package.metadata.rayx.ios_scheme for iOS builds",
                self.root.display()
            )
        })
    }

    pub fn ios_project(&self) -> Result<String> {
        if let Some(project) = self.ios_project.as_deref() {
            Ok(project.to_string())
        } else {
            Ok(format!("{}.xcodeproj", self.ios_scheme()?))
        }
    }

    pub fn ios_app_name(&self) -> Result<String> {
        Ok(self
            .ios_app_name
            .clone()
            .unwrap_or_else(|| self.ios_scheme().unwrap_or(&self.slug).to_string()))
    }

    pub fn ios_bundle_id(&self) -> Result<&str> {
        self.ios_bundle_id.as_deref().ok_or_else(|| {
            anyhow!(
                "app {} must set package.metadata.rayx.ios_bundle_id for iOS simulator launch",
                self.root.display()
            )
        })
    }
}

pub fn package_name_from_manifest(manifest: &Path) -> Result<String> {
    let value = read_manifest(manifest)?;
    package_name_from_value(&value, manifest)
}

pub fn rust_crate_file_stem(package_name: &str) -> String {
    package_name.replace('-', "_")
}

fn read_manifest(manifest: &Path) -> Result<Value> {
    let text = fs::read_to_string(manifest)
        .with_context(|| format!("reading manifest {}", manifest.display()))?;
    toml::from_str::<Value>(&text)
        .with_context(|| format!("parsing manifest {}", manifest.display()))
}

fn package_name_from_value(value: &Value, manifest: &Path) -> Result<String> {
    value
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("manifest {} has no package.name", manifest.display()))
}

fn lib_name_from_value(value: &Value) -> Option<String> {
    value
        .get("lib")
        .and_then(|lib| lib.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn metadata_string(value: &Value, key: &str) -> Option<String> {
    rayx_metadata(value)
        .and_then(|rayx| rayx.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn metadata_string_array(value: &Value, key: &str) -> Option<Vec<String>> {
    rayx_metadata(value)
        .and_then(|rayx| rayx.get(key))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
}

fn metadata_bool(value: &Value, key: &str) -> bool {
    rayx_metadata(value)
        .and_then(|rayx| rayx.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn manifest_has_feature(manifest: &Path, feature: &str) -> Result<bool> {
    let value = read_manifest(manifest)?;
    Ok(value
        .get("features")
        .and_then(|features| features.get(feature))
        .is_some())
}

fn manifest_has_active_dependency(
    manifest: &Path,
    dependency: &str,
    enabled_features: &[String],
) -> Result<bool> {
    let value = read_manifest(manifest)?;
    let mut declarations = Vec::new();
    if let Some(declaration) = value
        .get("dependencies")
        .and_then(|dependencies| dependencies.get(dependency))
    {
        declarations.push(declaration);
    }
    if let Some(targets) = value.get("target").and_then(Value::as_table) {
        declarations.extend(targets.values().filter_map(|target| {
            target
                .get("dependencies")
                .and_then(|dependencies| dependencies.get(dependency))
        }));
    }
    if declarations.is_empty() {
        return Ok(false);
    }
    if declarations.iter().any(|declaration| {
        !declaration
            .get("optional")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }) {
        return Ok(true);
    }

    let Some(features) = value.get("features").and_then(Value::as_table) else {
        return Ok(enabled_features.iter().any(|feature| feature == dependency));
    };
    let mut pending = enabled_features.to_vec();
    let mut visited = std::collections::BTreeSet::new();
    while let Some(feature) = pending.pop() {
        if feature == dependency {
            return Ok(true);
        }
        if !visited.insert(feature.clone()) {
            continue;
        }
        let Some(members) = features.get(&feature).and_then(Value::as_array) else {
            continue;
        };
        for member in members.iter().filter_map(Value::as_str) {
            if member.strip_prefix("dep:") == Some(dependency)
                || member
                    .split_once('/')
                    .is_some_and(|(name, _)| name == dependency)
            {
                return Ok(true);
            }
            if features.contains_key(member) {
                pending.push(member.to_string());
            }
        }
    }
    Ok(false)
}

fn manifest_feature_contains(manifest: &Path, feature: &str, dependency: &str) -> Result<bool> {
    let value = read_manifest(manifest)?;
    Ok(value
        .get("features")
        .and_then(|features| features.get(feature))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|value| value == dependency))
}

fn manifest_default_features(manifest: &Path) -> Result<Vec<String>> {
    let value = read_manifest(manifest)?;
    Ok(value
        .get("features")
        .and_then(|features| features.get("default"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect())
}

fn manifest_feature_names(manifest: &Path) -> Result<Vec<String>> {
    let value = read_manifest(manifest)?;
    Ok(value
        .get("features")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|features| features.keys())
        .filter(|feature| feature.as_str() != "default")
        .cloned()
        .collect())
}

fn rayx_metadata(value: &Value) -> Option<&Value> {
    value
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("rayx"))
}

/// The root workspace's patches a generated web manifest must repeat (patches only apply
/// from the root of the workspace being built): the IndexedDB storage crates when the app
/// enables SurrealDB storage.
fn generated_web_patch_lines(context: &ProjectContext, storage: bool) -> Result<String> {
    if !storage {
        return Ok(String::new());
    }
    let mut crates_io = String::new();
    for name in ["indxdb", "rexie"] {
        if let Some(path) = context.root_patch_path("crates-io", name)? {
            crates_io.push_str(&format!(
                "{name} = {{ path = \"{}\" }}\n",
                cargo_toml_path(&path)
            ));
        }
    }
    if crates_io.is_empty() {
        return Ok(String::new());
    }
    Ok(format!("\n[patch.crates-io]\n{crates_io}"))
}

fn cargo_toml_path(path: &Path) -> String {
    command_path(path).to_string_lossy().replace('\\', "/")
}

fn resolve_app_path(root: &Path, path: String) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn sanitize_slug(value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "app".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::FixtureWorkspace;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    /// Replaces the fixture app's manifest in the temporary copy of the workspace.
    fn rewrite_app_manifest(workspace: &FixtureWorkspace, text: &str) -> std::io::Result<()> {
        std::fs::write(
            workspace.root.join("apps").join("demo").join("Cargo.toml"),
            text,
        )
    }

    fn root_patch(app: &AppDescriptor, registry: &str, name: &str) -> String {
        let path = app
            .context
            .root_patch_path(registry, name)
            .expect("root Cargo.toml parses")
            .expect("the workspace patches this crate");
        cargo_toml_path(&path)
    }

    #[test]
    fn resolve_reads_the_descriptor_from_the_manifest_metadata() {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();

        assert_eq!(app.slug, "demo");
        assert_eq!(app.rust_lib_name, "rayx_demo");
        assert_eq!(app.lib_name, "rayx_demo");
        assert_eq!(
            app.android_application_id.as_deref(),
            Some("dev.example.demo")
        );
        assert!(!app.diagnostic_harness);
        assert!(app.package().is_ok());
    }

    #[test]
    fn resolve_rejects_a_directory_without_a_manifest() {
        let workspace = FixtureWorkspace::new();
        let context = workspace.app().context;
        let error =
            AppDescriptor::resolve_from(&context, &workspace.root, "apps/missing").unwrap_err();
        assert!(error.to_string().contains("was not found"), "{error:#}");

        let error = AppDescriptor::resolve_from(&context, &workspace.root, "crates").unwrap_err();
        assert!(error.to_string().contains("no Cargo.toml"), "{error:#}");
    }

    #[test]
    fn output_directories_live_under_the_workspace_root() -> TestResult {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let root = app.context.workspace_root().to_path_buf();

        assert_eq!(app.artifact_root()?, root.join("artifacts/apps/demo"));
        assert_eq!(app.temp_root()?, root.join("artifacts-temp/apps/demo"));
        assert_eq!(app.target_dir()?, root.join("target"));
        Ok(())
    }

    #[test]
    fn android_entry_manifest_is_generated_from_the_apps_own_rayx_dependency() -> TestResult {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();

        let manifest = app.android_rust_manifest(&AppFeatureSelection::default())?;
        assert!(manifest.starts_with(app.temp_root()?.join("android").join("rust")));
        let text = std::fs::read_to_string(&manifest)?;
        assert!(text.contains("[workspace]"));
        assert!(text.contains("crate-type = [\"cdylib\"]"));
        assert!(text.contains("rayx = { path = "));
        assert!(text.contains("crates/rayx\" }"));
        assert!(text.contains("rayx_demo = { package = \"rayx_demo\", path = "));
        assert!(text.contains("default = [\"rayx_demo/profile-all\"]"));

        let source = std::fs::read_to_string(manifest.parent().unwrap().join("src/lib.rs"))?;
        assert!(source.contains("apps/demo/src/main.rs"));
        Ok(())
    }

    #[test]
    fn android_entry_manifest_ignores_a_checked_in_wrapper_manifest() -> TestResult {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let wrapper = app.root.join("platform").join("android");
        std::fs::write(
            wrapper.join("Cargo.toml"),
            "[package]\nname = \"wrapper\"\n",
        )?;

        let manifest = app.android_rust_manifest(&AppFeatureSelection::default())?;
        assert_ne!(manifest, wrapper.join("Cargo.toml"));
        assert!(manifest.starts_with(app.temp_root()?.join("android").join("rust")));
        Ok(())
    }

    #[test]
    fn ios_entry_manifest_builds_a_static_library() -> TestResult {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();

        assert!(!app.platform_manifest("ios").is_file());
        let manifest = app.ios_rust_manifest(&AppFeatureSelection::default())?;
        assert!(manifest.starts_with(app.temp_root()?.join("ios").join("rust")));
        let text = std::fs::read_to_string(&manifest)?;
        assert!(text.contains("crate-type = [\"staticlib\"]"));
        assert!(text.contains("rayx_demo = { package = \"rayx_demo\""));
        Ok(())
    }

    #[test]
    fn platform_names_come_from_the_manifest_metadata() -> TestResult {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();

        assert_eq!(
            app.android_launch_component()?,
            "dev.example.demo/dev.rayx.mobile.RayXActivity"
        );
        assert_eq!(app.ios_project()?, "Demo.xcodeproj");
        assert_eq!(app.ios_scheme()?, "Demo");
        assert_eq!(app.ios_bundle_id()?, "dev.example.demo");
        assert_eq!(app.ios_app_name()?, "Demo");
        Ok(())
    }

    #[test]
    fn web_manifest_is_generated_with_the_locked_wasm_bindgen() -> TestResult {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let generated_root = app.temp_root()?.join("wasm").join("rust");
        std::fs::create_dir_all(&generated_root)?;
        std::fs::write(generated_root.join("Cargo.lock"), "stale lock")?;

        let manifest = app.web_manifest(&AppFeatureSelection::default())?;
        assert!(manifest.starts_with(&generated_root));
        assert!(!generated_root.join("Cargo.lock").exists());
        assert_eq!(app.web_module_base(), "rayx_demo_web");

        let text = std::fs::read_to_string(&manifest)?;
        assert!(text.contains("name = \"rayx_demo_web\""));
        assert!(text.contains("crate-type = [\"cdylib\", \"rlib\"]"));
        assert!(text.contains("default-features = false"));
        assert!(text.contains("wasm-bindgen = \"=0.2.129\""));
        assert!(text.contains("zune-core = \"=0.5.1\""));
        assert!(text.contains("branches = { version = \"0.4\", features = [\"std\"] }"));
        assert!(!text.contains("[patch"));
        Ok(())
    }

    #[test]
    fn web_manifest_repeats_the_root_storage_patches_only_for_active_storage() -> TestResult {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let storage_app = "[package]\nname = \"rayx_demo\"\nversion = \"0.1.0\"\n[features]\nstorage-fixture = [\"dep:rayx_storage_surrealdb\"]\n[target.'cfg(target_family = \"wasm\")'.dependencies]\nrayx_storage_surrealdb = { version = \"0.1\", optional = true }\n";
        rewrite_app_manifest(&workspace, storage_app)?;

        let text = std::fs::read_to_string(app.web_manifest(&AppFeatureSelection::default())?)?;
        assert!(!text.contains("[patch.crates-io]"));

        let selection = AppFeatureSelection {
            features: vec!["storage-fixture".to_string()],
            no_default_features: true,
            ..AppFeatureSelection::default()
        };
        let text = std::fs::read_to_string(app.web_manifest(&selection)?)?;
        assert!(text.contains("[patch.crates-io]"));
        assert!(text.contains(&root_patch(&app, "crates-io", "indxdb")));
        assert!(text.contains(&root_patch(&app, "crates-io", "rexie")));
        Ok(())
    }

    #[test]
    fn every_entry_manifest_forwards_one_explicit_feature_selection() -> TestResult {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let selection = AppFeatureSelection {
            features: vec!["profile-a, profile-b".to_string()],
            all_features: false,
            no_default_features: true,
        };

        for manifest in [
            app.android_rust_manifest(&selection)?,
            app.ios_rust_manifest(&selection)?,
            app.web_manifest(&selection)?,
        ] {
            let text = std::fs::read_to_string(manifest)?;
            assert!(text.contains("default = [\"rayx_demo/profile-a\", \"rayx_demo/profile-b\"]"));
            assert!(text.contains("default-features = false"));
        }
        Ok(())
    }

    #[test]
    fn all_features_forwards_every_feature_and_the_devkit_dependency() -> TestResult {
        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let selection = AppFeatureSelection {
            all_features: true,
            ..AppFeatureSelection::default()
        };

        let text = std::fs::read_to_string(app.android_rust_manifest(&selection)?)?;
        assert!(text.contains("profile-a = [\"rayx_demo/profile-a\"]"));
        assert!(text.contains("devkit = [\"rayx_demo/devkit\", \"dep:rayx_devkit\"]"));
        assert!(text.contains("rayx_devkit = { path = "));
        assert!(text.contains(", optional = true }"));
        Ok(())
    }

    #[test]
    fn an_app_without_a_rayx_dependency_names_the_missing_dependency() -> TestResult {
        let workspace = FixtureWorkspace::new();
        rewrite_app_manifest(
            &workspace,
            "[package]\nname = \"rayx_demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        let app = workspace.app();

        let error = app
            .android_rust_manifest(&AppFeatureSelection::default())
            .unwrap_err();
        assert!(
            error.to_string().contains("does not depend on `rayx`"),
            "{error:#}"
        );
        Ok(())
    }

    #[test]
    fn app_feature_selection_rejects_conflicting_empty_and_unknown_features() {
        let mut conflicting = vec![
            "--all-features".to_string(),
            "--no-default-features".to_string(),
        ];
        assert!(AppFeatureSelection::take_from_args(&mut conflicting).is_err());

        let mut empty = vec!["--features".to_string(), " , ".to_string()];
        assert!(AppFeatureSelection::take_from_args(&mut empty).is_err());

        let workspace = FixtureWorkspace::new();
        let app = workspace.app();
        let unknown = AppFeatureSelection {
            features: vec!["not-a-profile".to_string()],
            ..AppFeatureSelection::default()
        };
        assert!(unknown.validate_for(&app.manifest()).is_err());
    }

    #[test]
    fn feature_arguments_before_the_separator_are_normalized() -> TestResult {
        let mut args: Vec<String> = [
            "--features",
            "profile-a,profile-b profile-a",
            "--no-default-features",
            "--",
            "--app-flag",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();

        let selection = AppFeatureSelection::take_from_args(&mut args)?;
        assert_eq!(selection.normalized_features(), ["profile-a", "profile-b"]);
        assert!(!selection.default_features_enabled());
        assert_eq!(args, ["--", "--app-flag"]);
        Ok(())
    }

    #[test]
    fn playwright_metadata_is_read_from_the_package_table() -> TestResult {
        let workspace = FixtureWorkspace::new();
        assert_eq!(workspace.app().playwright_specs()?, None);

        rewrite_app_manifest(
            &workspace,
            "[package]\nname = \"rayx_demo\"\nversion = \"0.1.0\"\n[package.metadata.rayx]\nplaywright_specs = [\"demo/\", \"shared/a.spec.ts\"]\nplaywright_projects = [\"chromium\"]\n[dependencies]\nrayx = { path = \"../../crates/rayx\" }\n",
        )?;
        let app = workspace.app();
        assert_eq!(
            app.playwright_specs()?,
            Some(vec!["demo/".to_string(), "shared/a.spec.ts".to_string()])
        );
        assert_eq!(
            app.playwright_projects()?,
            Some(vec!["chromium".to_string()])
        );
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn generated_manifest_paths_strip_windows_verbatim_prefix() {
        assert_eq!(
            cargo_toml_path(Path::new(r"\\?\C:\repo\apps\example")),
            "C:/repo/apps/example"
        );
    }
}
