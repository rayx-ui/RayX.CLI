use crate::app::args::{take_bool_flag, take_flag_value};
use crate::app::fs_util::{command_path, repo_root};
use crate::app::workspace_paths;
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
}

impl AppDescriptor {
    pub fn resolve(app_dir: &str) -> Result<Self> {
        let requested = PathBuf::from(app_dir);
        let root = if requested.is_absolute() {
            requested
        } else {
            repo_root()?.join(requested)
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
        })
    }

    pub fn manifest(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
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

    pub fn artifact_root(&self) -> Result<PathBuf> {
        Ok(repo_root()?.join("artifacts").join("apps").join(&self.slug))
    }

    pub fn temp_root(&self) -> Result<PathBuf> {
        Ok(repo_root()?
            .join("artifacts-temp")
            .join("apps")
            .join(&self.slug))
    }

    pub fn target_dir(&self) -> Result<PathBuf> {
        Ok(repo_root()?.join("target"))
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
        let rayx_path = cargo_toml_path(&repo_root()?.join("crates").join("rayx"));
        let rayx_devkit_path = cargo_toml_path(&repo_root()?.join("crates").join("rayx_devkit"));
        let app_path = cargo_toml_path(&self.root);
        let generated_devkit_dependency_line = if generated_devkit_dependency {
            format!("rayx_devkit = {{ path = \"{rayx_devkit_path}\", optional = true }}\n")
        } else {
            String::new()
        };
        // `rclite` (through `rxrust`) uses `branches` without `std`, whose nightly path calls
        // `core::intrinsics::abort`, which current nightlies removed. With `std` it aborts through
        // `std::process::abort`, which the threaded build's `build-std` provides.
        let generated_web_dependency_lines = if target == "web" {
            "wasm-bindgen = \"=0.2.129\"\nwasm-bindgen-futures = \"0.4\"\nzune-core = \"=0.5.1\"\nbranches = { version = \"0.4\", features = [\"std\"] }\n"
        } else {
            ""
        };
        let generated_web_patch_lines = if target == "web" {
            generated_web_patch_lines(manifest_has_active_dependency(
                &app_manifest,
                "rayx_storage_surrealdb",
                &enabled_app_features,
            )?)?
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
rayx = {{ path = "{}" }}
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
            rayx_path,
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
fn generated_web_patch_lines(storage: bool) -> Result<String> {
    if !storage {
        return Ok(String::new());
    }
    let mut crates_io = String::new();
    for name in ["indxdb", "rexie"] {
        if let Some(path) = workspace_paths::root_patch_path("crates-io", name)? {
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
    use std::sync::{LazyLock, Mutex};

    static LAB_WEB_MANIFEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn unique_test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rayx-xtask-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ))
    }

    fn descriptor(root: PathBuf) -> AppDescriptor {
        let slug = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("test-app")
            .to_string();
        AppDescriptor {
            root,
            slug,
            lib_name: "test_app".to_string(),
            rust_lib_name: "test_app".to_string(),
            android_application_id: None,
            android_activity: None,
            ios_project: None,
            ios_scheme: None,
            ios_bundle_id: None,
            ios_app_name: None,
            diagnostic_harness: false,
        }
    }

    fn root_patch(registry: &str, name: &str) -> String {
        let path = crate::app::workspace_paths::root_patch_path(registry, name)
            .expect("root Cargo.toml parses")
            .expect("the root workspace patches this crate");
        cargo_toml_path(&path)
    }

    fn repo_root_for_test() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask manifest should live under the repository root")
            .to_path_buf()
    }

    #[test]
    fn android_rust_manifest_generates_temp_manifest_when_wrapper_is_absent()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = unique_test_dir("android-generated-manifest");
        std::fs::create_dir_all(root.join("src"))?;
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"root\"\n")?;
        std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n")?;

        let app = descriptor(root.clone());
        let manifest = app.android_rust_manifest(&AppFeatureSelection::default())?;
        assert!(manifest.starts_with(app.temp_root()?.join("android").join("rust")));
        assert!(manifest.is_file());

        let manifest_text = std::fs::read_to_string(&manifest)?;
        assert!(manifest_text.contains("[workspace]"));
        assert!(manifest_text.contains("crate-type = [\"cdylib\"]"));
        assert!(manifest_text.contains("rayx = { path = "));
        assert!(manifest_text.contains("test_app = { package = \"root\""));

        let manifest_dir = manifest
            .parent()
            .ok_or_else(|| anyhow!("generated manifest should have a parent"))?;
        let source = std::fs::read_to_string(manifest_dir.join("src/lib.rs"))?;
        assert!(source.contains("src/main.rs"));

        std::fs::remove_dir_all(root)?;
        std::fs::remove_dir_all(app.temp_root()?).ok();
        Ok(())
    }

    #[test]
    fn android_rust_manifest_ignores_checked_in_platform_wrapper_manifest()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = unique_test_dir("android-wrapper-ignored");
        let wrapper_dir = root.join("platform").join("android");
        std::fs::create_dir_all(&wrapper_dir)?;
        std::fs::create_dir_all(root.join("src"))?;
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"root\"\n")?;
        std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n")?;
        std::fs::write(
            wrapper_dir.join("Cargo.toml"),
            "[package]\nname = \"wrapper\"\n",
        )?;

        let app = descriptor(root.clone());
        let manifest = app.android_rust_manifest(&AppFeatureSelection::default())?;
        assert_ne!(manifest, wrapper_dir.join("Cargo.toml"));
        assert!(manifest.starts_with(app.temp_root()?.join("android").join("rust")));

        std::fs::remove_dir_all(root)?;
        std::fs::remove_dir_all(app.temp_root()?).ok();
        Ok(())
    }

    #[test]
    fn web_rust_manifest_generates_temp_manifest_when_wrapper_is_absent()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = unique_test_dir("web-generated-manifest");
        std::fs::create_dir_all(root.join("src"))?;
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"root\"\n")?;
        std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n")?;

        let app = descriptor(root.clone());
        let generated_root = app.temp_root()?.join("wasm").join("rust");
        std::fs::create_dir_all(&generated_root)?;
        std::fs::write(generated_root.join("Cargo.lock"), "stale lock")?;

        let manifest = app.web_manifest(&AppFeatureSelection::default())?;
        assert!(manifest.starts_with(app.temp_root()?.join("wasm").join("rust")));
        assert!(manifest.is_file());
        assert!(!generated_root.join("Cargo.lock").exists());
        assert_eq!(app.web_module_base(), "test_app_web");

        let manifest_text = std::fs::read_to_string(&manifest)?;
        assert!(manifest_text.contains("name = \"test_app_web\""));
        assert!(manifest_text.contains("crate-type = [\"cdylib\", \"rlib\"]"));
        assert!(manifest_text.contains("default = []"));
        assert!(manifest_text.contains("rayx = { path = "));
        assert!(manifest_text.contains("test_app = { package = \"root\", path = "));
        assert!(manifest_text.contains("default-features = false"));
        assert!(manifest_text.contains("wasm-bindgen = \"=0.2.129\""));
        assert!(manifest_text.contains("zune-core = \"=0.5.1\""));
        assert!(manifest_text.contains("branches = { version = \"0.4\", features = [\"std\"] }"));
        assert!(!manifest_text.contains("[patch"));

        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"root\"\nversion = \"0.1.0\"\n[features]\nindexeddb-storage-fixture = [\"dep:rayx_storage_surrealdb\"]\n[target.'cfg(target_family = \"wasm\")'.dependencies]\nrayx_storage_surrealdb = { version = \"0.1\", optional = true }\n",
        )?;
        let manifest = app.web_manifest(&AppFeatureSelection::default())?;
        let manifest_text = std::fs::read_to_string(&manifest)?;
        assert!(!manifest_text.contains("[patch.crates-io]"));

        let manifest = app.web_manifest(&AppFeatureSelection {
            features: vec!["indexeddb-storage-fixture".to_string()],
            no_default_features: true,
            ..AppFeatureSelection::default()
        })?;
        let manifest_text = std::fs::read_to_string(&manifest)?;
        assert!(manifest_text.contains("[patch.crates-io]"));
        assert!(manifest_text.contains("indxdb = { path = "));
        assert!(manifest_text.contains(&root_patch("crates-io", "indxdb")));
        assert!(manifest_text.contains("rexie = { path = "));
        assert!(manifest_text.contains(&root_patch("crates-io", "rexie")));

        let manifest_dir = manifest
            .parent()
            .ok_or_else(|| anyhow!("generated manifest should have a parent"))?;
        let source = std::fs::read_to_string(manifest_dir.join("src/lib.rs"))?;
        assert!(source.contains("src/main.rs"));

        std::fs::remove_dir_all(root)?;
        std::fs::remove_dir_all(app.temp_root()?).ok();
        Ok(())
    }

    #[test]
    fn lab_web_manifest_forwards_default_combined_code_surfaces_profile()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let _lock = LAB_WEB_MANIFEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let app_root = repo_root_for_test().join("apps").join("lab");
        let app = AppDescriptor::resolve(app_root.to_str().expect("utf-8 app path"))?;

        let manifest = app.web_manifest(&AppFeatureSelection::default())?;
        let manifest_text = std::fs::read_to_string(&manifest)?;

        assert!(manifest_text.contains("default = [\"rayx_lab_app/code-surfaces-all\"]"));
        assert!(manifest_text.contains("rayx_lab_app = { package = \"rayx_lab\", path = "));
        assert!(manifest_text.contains(&root_patch("crates-io", "indxdb")));
        assert!(manifest_text.contains(&root_patch("crates-io", "rexie")));
        assert!(manifest_text.contains("default-features = false"));

        std::fs::remove_dir_all(app.temp_root()?.join("wasm")).ok();
        Ok(())
    }

    #[test]
    fn lab_web_manifest_forwards_explicit_features_without_app_defaults()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let _lock = LAB_WEB_MANIFEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let app_root = repo_root_for_test().join("apps").join("lab");
        let app = AppDescriptor::resolve(app_root.to_str().expect("utf-8 app path"))?;
        let selection = AppFeatureSelection {
            features: vec!["code-editor-rust-dracula, code-view-rust-dracula".to_string()],
            all_features: false,
            no_default_features: true,
        };

        let manifest = app.web_manifest(&selection)?;
        let manifest_text = std::fs::read_to_string(&manifest)?;

        assert!(manifest_text.contains(
            "default = [\"rayx_lab_app/code-editor-rust-dracula\", \"rayx_lab_app/code-view-rust-dracula\"]"
        ));

        std::fs::remove_dir_all(app.temp_root()?.join("wasm")).ok();
        Ok(())
    }

    #[test]
    fn generated_platform_wrappers_forward_one_explicit_feature_selection()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = unique_test_dir("feature-selection-wrappers");
        std::fs::create_dir_all(root.join("src"))?;
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"root\"\n\n[features]\ndefault = [\"default-profile\"]\ndefault-profile = []\nminimal = []\n",
        )?;
        std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n")?;
        let app = descriptor(root.clone());
        let selection = AppFeatureSelection {
            features: vec!["minimal".to_string()],
            all_features: false,
            no_default_features: true,
        };

        for manifest in [
            app.android_rust_manifest(&selection)?,
            app.ios_rust_manifest(&selection)?,
            app.web_manifest(&selection)?,
        ] {
            let text = std::fs::read_to_string(manifest)?;
            assert!(text.contains("default = [\"test_app/minimal\"]"));
            assert!(text.contains("default-features = false"));
        }

        std::fs::remove_dir_all(root)?;
        std::fs::remove_dir_all(app.temp_root()?).ok();
        Ok(())
    }

    #[test]
    fn app_feature_selection_rejects_conflicting_empty_and_unknown_features()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut conflicting = vec![
            "--all-features".to_string(),
            "--no-default-features".to_string(),
        ];
        assert!(AppFeatureSelection::take_from_args(&mut conflicting).is_err());

        let mut empty = vec!["--features".to_string(), " , ".to_string()];
        assert!(AppFeatureSelection::take_from_args(&mut empty).is_err());

        let app_root = repo_root_for_test().join("apps").join("lab");
        let app = AppDescriptor::resolve(app_root.to_str().expect("utf-8 app path"))?;
        let unknown = AppFeatureSelection {
            features: vec!["not-a-lab-profile".to_string()],
            ..AppFeatureSelection::default()
        };
        assert!(unknown.validate_for(&app.manifest()).is_err());
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

    #[test]
    fn lab_android_uses_generated_unified_entry_manifest()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_root = repo_root_for_test().join("apps").join("lab");
        let app = AppDescriptor::resolve(app_root.to_str().expect("utf-8 app path"))?;
        assert_eq!(
            app.android_launch_component()?,
            "dev.rayx.lab/dev.rayx.mobile.RayXActivity"
        );

        assert!(
            !app.platform_manifest("android").is_file(),
            "Lab should not keep a checked-in Android Rust wrapper manifest"
        );

        let manifest = app.android_rust_manifest(&AppFeatureSelection::default())?;
        assert!(manifest.starts_with(app.temp_root()?.join("android").join("rust")));
        let manifest_text = std::fs::read_to_string(&manifest)?;
        assert!(manifest_text.contains("crate-type = [\"cdylib\"]"));
        assert!(manifest_text.contains("rayx_lab_app = { package = \"rayx_lab\""));

        let source = std::fs::read_to_string(
            manifest
                .parent()
                .expect("generated manifest should have parent")
                .join("src")
                .join("lib.rs"),
        )?;
        assert!(source.contains("apps/lab/src/main.rs"));

        std::fs::remove_dir_all(app.temp_root()?.join("android")).ok();
        Ok(())
    }

    #[test]
    fn lab_ios_uses_generated_unified_entry_manifest()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_root = repo_root_for_test().join("apps").join("lab");
        let app = AppDescriptor::resolve(app_root.to_str().expect("utf-8 app path"))?;
        assert_eq!(app.ios_project()?, "RayXLab.xcodeproj");
        assert_eq!(app.ios_scheme()?, "RayXLab");
        assert_eq!(app.ios_bundle_id()?, "dev.rayx.lab");
        assert_eq!(app.ios_app_name()?, "RayXLab");

        assert!(
            !app.platform_manifest("ios").is_file(),
            "Lab should not keep a checked-in iOS Rust wrapper manifest"
        );

        let manifest = app.ios_rust_manifest(&AppFeatureSelection::default())?;
        assert!(manifest.starts_with(app.temp_root()?.join("ios").join("rust")));
        let manifest_text = std::fs::read_to_string(&manifest)?;
        assert!(manifest_text.contains("crate-type = [\"staticlib\"]"));
        assert!(manifest_text.contains("rayx_lab_app = { package = \"rayx_lab\""));

        let source = std::fs::read_to_string(
            manifest
                .parent()
                .expect("generated manifest should have parent")
                .join("src")
                .join("lib.rs"),
        )?;
        assert!(source.contains("apps/lab/src/main.rs"));

        let project = std::fs::read_to_string(app.ios_dir().join("project.yml"))?;
        assert!(project.contains("RAYX_IOS_RUST_MANIFEST"));
        assert!(project.contains("--manifest-path \"${RAYX_IOS_RUST_MANIFEST}\""));
        assert!(!project.contains("--manifest-path Cargo.toml"));
        assert!(project.contains("$(PROJECT_DIR)/target/aarch64-apple-ios-sim"));
        assert!(project.contains("$(PROJECT_DIR)/target/aarch64-apple-ios"));

        std::fs::remove_dir_all(app.temp_root()?.join("ios")).ok();
        Ok(())
    }

    #[test]
    fn lab_source_uses_unified_main_and_ready_runtime_setup()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_root = repo_root_for_test().join("apps").join("lab");
        let main = std::fs::read_to_string(app_root.join("src").join("main.rs"))?;
        let lib = std::fs::read_to_string(app_root.join("src").join("lib.rs"))?;
        let startup = std::fs::read_to_string(app_root.join("src").join("app").join("startup.rs"))?;

        assert!(main.contains("#[rayx::main]"));
        assert!(main.contains("const APP_MANIFEST_DIR: &str = env!(\"CARGO_MANIFEST_DIR\");"));
        assert!(main.contains("RayXApp::builder()"));
        assert!(main.contains(".with_assets(|options|"));
        assert!(main.contains("options.app_manifest_assets(APP_MANIFEST_DIR)"));
        assert!(main.contains(".with_fonts(|fonts|"));
        assert!(main.contains("fonts.default_alias(\"Inter\")"));
        assert!(main.contains(".with_surrealdb_storage(|options|"));
        assert!(!main.contains("load_theme_packages_from_assets"));
        assert!(main.contains(".with_i18n(|options|"));
        assert!(main.contains(".with_navigation(|options|"));
        assert!(main.contains("options.add_route(route.route_definition())"));
        assert!(main.contains(".build()"));
        assert!(main.contains(".await?"));
        assert!(main.contains(".when_windowed"));
        assert!(main.contains(".custom_title_bar()"));
        assert!(main.contains(".when_single_view"));
        assert!(main.contains(".on_ready(|cx|"));
        assert!(main.contains("cx.update_gpux(initialize_app)?"));
        assert!(main.contains(".run()"));
        assert!(main.contains(".await"));
        assert!(startup.contains("pub(crate) struct LabRuntime"));
        assert!(startup.contains("if cx.has_global::<Self>()"));
        assert!(startup.contains("AppState::install(cx)"));
        assert!(startup.contains("pub fn initialize_app(cx: &mut App)"));
        assert!(!startup.contains("fn install_lab_app_setup"));
        assert!(!lib.contains("mod app_builder"));
        assert!(!lib.contains("APP_MANIFEST_DIR"));
        assert!(
            !app_root
                .join("platform")
                .join("windows")
                .join("Cargo.toml")
                .is_file()
        );
        assert!(
            !app_root
                .join("platform")
                .join("linux")
                .join("Cargo.toml")
                .is_file()
        );
        assert!(
            !app_root
                .join("platform")
                .join("macos")
                .join("Cargo.toml")
                .is_file()
        );
        assert!(
            !app_root
                .join("platform")
                .join("web")
                .join("Cargo.toml")
                .is_file()
        );

        let source = format!("{main}\n{lib}\n{startup}");
        for forbidden in [
            "trait LabAppBuilderExt",
            "with_labapp",
            "run_story_app",
            "run_story_app_blocking",
            "StoryAppResult",
            "LabGpuiStartup",
            "GpuiStartupInstaller",
            "register_gpui_startup_installer",
            "fn android_main(app: android_activity::AndroidApp)",
            "pub extern \"C\" fn gpui_ios_register_app()",
            "StoryAppHost",
            "story_app_builder",
            "with_app_host_extension",
            "rayx::mobile::android::run_native_activity",
            "rayx::mobile::ios::register_app",
            "gpui_mobile::android::jni",
            "gpui_mobile::ios::ffi",
            "Application::with_platform",
            "jni::init_platform",
            "jni::shared_platform",
            "set_app_callback",
        ] {
            assert!(
                !source.contains(forbidden),
                "Lab shared source should not contain low-level mobile startup `{forbidden}`"
            );
        }
        Ok(())
    }

    #[test]
    fn examples_mobile_uses_unified_main_and_inline_service_setup()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_root = repo_root_for_test().join("apps").join("examples_mobile");
        let main = std::fs::read_to_string(app_root.join("src").join("main.rs"))?;
        let lib = std::fs::read_to_string(app_root.join("src").join("lib.rs"))?;

        assert!(main.contains("#[rayx::main]"));
        assert!(main.contains("RayXApp::builder()"));
        assert!(main.contains(".with_assets(|options|"));
        assert!(main.contains("options.app_manifest_assets(APP_MANIFEST_DIR)"));
        assert!(main.contains(".build()"));
        assert!(main.contains(".await?"));
        assert!(main.contains(".when_single_view"));
        assert!(main.contains(".run()"));
        assert!(main.contains(".await"));
        assert!(main.contains("root_view(window, cx)"));

        assert!(!main.contains(".with_examples_mobile()"));
        assert!(!lib.contains("ExamplesMobileAppBuilderExt"));
        assert!(!lib.contains("with_examples_mobile"));
        assert!(lib.contains("pub fn root_view"));
        assert!(!lib.contains("pub fn app() -> RayXAppBuilder"));
        Ok(())
    }

    #[test]
    fn examples_mobile_uses_default_http_client_behavior()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_source = repo_root_for_test()
            .join("apps")
            .join("examples_mobile")
            .join("src")
            .join("lib.rs");
        let app_manifest = repo_root_for_test()
            .join("apps")
            .join("examples_mobile")
            .join("Cargo.toml");

        let source = std::fs::read_to_string(app_source)?;
        let manifest = std::fs::read_to_string(app_manifest)?;
        assert!(!source.contains("DemoHttpClientAppBuilderExt"));
        assert!(!source.contains("DemoHttpClientInstaller"));
        assert!(!source.contains("register_gpui_startup_installer"));
        assert!(!source.contains("fn install_gpui"));
        assert!(!source.contains("cx.set_http_client"));
        assert!(!source.contains("DemoHttpClientHost"));
        assert!(!source.contains("AppHostExtension"));
        assert!(!source.contains("with_app_host_extension"));
        assert!(!manifest.contains("reqwest_client"));
        assert!(!manifest.contains("zed-reqwest"));
        Ok(())
    }

    #[test]
    fn examples_mobile_devkit_feature_opt_in_matches_android_diagnostics()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_root = repo_root_for_test().join("apps").join("examples_mobile");
        let source = std::fs::read_to_string(app_root.join("src").join("main.rs"))?;
        let manifest = std::fs::read_to_string(app_root.join("Cargo.toml"))?;

        assert!(manifest.contains("devkit = [\"dep:rayx_devkit\"]"));
        assert!(manifest.contains("rayx_devkit = { workspace = true, optional = true }"));
        assert!(source.contains("#[cfg(feature = \"devkit\")]"));
        assert!(source.contains("use rayx_devkit::RayXDevKitAppBuilderExt;"));
        assert!(source.contains(".with_devkit(|options|"));
        assert!(source.contains("options.app_id = \"gpui-mobile-example\".to_string();"));
        Ok(())
    }

    #[test]
    fn examples_mobile_selects_initial_route_in_rayx_root()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_source = repo_root_for_test()
            .join("apps")
            .join("examples_mobile")
            .join("src")
            .join("lib.rs");

        let source = std::fs::read_to_string(app_source)?;
        assert!(source.contains("pub fn initial_screen_from_deeplink() -> screens::Screen"));
        assert!(source.contains("pub fn root_view"));
        assert!(source.contains("Router::with_initial_screen(initial_screen_from_deeplink())"));
        assert!(!source.contains("fn open_main_window"));
        Ok(())
    }

    #[test]
    fn examples_mobile_android_uses_generated_unified_entry_manifest()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_root = repo_root_for_test().join("apps").join("examples_mobile");
        let app = AppDescriptor::resolve(app_root.to_str().expect("utf-8 app path"))?;

        assert!(
            !app.platform_manifest("android").is_file(),
            "examples_mobile should not keep a checked-in Android Rust wrapper manifest"
        );

        let manifest = app.android_rust_manifest(&AppFeatureSelection::default())?;
        assert!(manifest.starts_with(app.temp_root()?.join("android").join("rust")));
        let manifest_text = std::fs::read_to_string(&manifest)?;
        assert!(manifest_text.contains("crate-type = [\"cdylib\"]"));
        assert!(
            manifest_text
                .contains("devkit = [\"gpui_mobile_example/devkit\", \"dep:rayx_devkit\"]"),
            "generated Android entry should expose app and direct DevKit dependencies"
        );
        assert!(
            manifest_text.contains("rayx_devkit = { path = ")
                && manifest_text.contains("optional = true }"),
            "generated Android entry should include a direct optional rayx_devkit dependency"
        );
        assert!(
            manifest_text.contains("gpui_mobile_example = { package = \"gpui-mobile-example\"")
        );
        let source = std::fs::read_to_string(
            manifest
                .parent()
                .expect("generated manifest should have parent")
                .join("src")
                .join("lib.rs"),
        )?;
        assert!(source.contains("apps/examples_mobile/src/main.rs"));

        std::fs::remove_dir_all(app.temp_root()?.join("android")).ok();
        Ok(())
    }

    #[test]
    fn examples_mobile_ios_uses_generated_unified_entry_manifest()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_root = repo_root_for_test().join("apps").join("examples_mobile");
        let app = AppDescriptor::resolve(app_root.to_str().expect("utf-8 app path"))?;

        assert!(
            !app.platform_manifest("ios").is_file(),
            "examples_mobile should not keep a checked-in iOS Rust wrapper manifest"
        );

        let manifest = app.ios_rust_manifest(&AppFeatureSelection::default())?;
        assert!(manifest.starts_with(app.temp_root()?.join("ios").join("rust")));
        let manifest_text = std::fs::read_to_string(&manifest)?;
        assert!(manifest_text.contains("crate-type = [\"staticlib\"]"));
        assert!(
            manifest_text
                .contains("devkit = [\"gpui_mobile_example/devkit\", \"dep:rayx_devkit\"]"),
            "generated iOS entry should expose app and direct DevKit dependencies"
        );
        assert!(
            manifest_text.contains("rayx_devkit = { path = ")
                && manifest_text.contains("optional = true }"),
            "generated iOS entry should include a direct optional rayx_devkit dependency"
        );
        assert!(
            manifest_text.contains("gpui_mobile_example = { package = \"gpui-mobile-example\"")
        );
        let source = std::fs::read_to_string(
            manifest
                .parent()
                .expect("generated manifest should have parent")
                .join("src")
                .join("lib.rs"),
        )?;
        assert!(source.contains("apps/examples_mobile/src/main.rs"));

        let project = std::fs::read_to_string(app.ios_dir().join("project.yml"))?;
        assert!(project.contains("RAYX_IOS_RUST_MANIFEST"));
        assert!(project.contains("--manifest-path \"${RAYX_IOS_RUST_MANIFEST}\""));
        assert!(!project.contains("--manifest-path Cargo.toml"));
        assert!(project.contains("$(PROJECT_DIR)/target/aarch64-apple-ios-sim"));
        assert!(project.contains("$(PROJECT_DIR)/target/aarch64-apple-ios"));

        std::fs::remove_dir_all(app.temp_root()?.join("ios")).ok();
        Ok(())
    }

    #[test]
    fn examples_mobile_shared_source_has_no_manual_android_startup()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_root = repo_root_for_test().join("apps").join("examples_mobile");
        let shared_source = app_root.join("src").join("lib.rs");
        let binary_source = app_root.join("src").join("main.rs");

        let source = format!(
            "{}\n{}",
            std::fs::read_to_string(shared_source)?,
            std::fs::read_to_string(binary_source)?
        );
        for forbidden in [
            "fn android_main(app: android_activity::AndroidApp)",
            "gpui_mobile::android::jni",
            "jni::init_platform",
            "jni::shared_platform",
            "gpui_mobile::android::init_logger",
            "jni::install_panic_hook",
            "Application::with_platform",
        ] {
            assert!(
                !source.contains(forbidden),
                "shared examples_mobile source should not contain manual Android startup call `{forbidden}`"
            );
        }
        Ok(())
    }

    #[test]
    fn examples_mobile_shared_source_has_no_manual_ios_startup()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let app_root = repo_root_for_test().join("apps").join("examples_mobile");
        let shared_source = app_root.join("src").join("lib.rs");
        let binary_source = app_root.join("src").join("main.rs");

        let source = format!(
            "{}\n{}",
            std::fs::read_to_string(shared_source)?,
            std::fs::read_to_string(binary_source)?
        );
        for forbidden in [
            "pub extern \"C\" fn gpui_ios_register_app()",
            "gpui_mobile::ios::ffi::set_app_callback",
            "gpui_mobile::ios::ffi::run_app",
            "fn ios_main()",
            "fn open_main_window",
            "cx.open_window",
        ] {
            assert!(
                !source.contains(forbidden),
                "shared examples_mobile source should not contain manual iOS startup call `{forbidden}`"
            );
        }
        Ok(())
    }

    #[test]
    fn examples_mobile_docs_describe_unified_generated_startup()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let repo = repo_root_for_test();
        let readme =
            std::fs::read_to_string(repo.join("apps").join("examples_mobile").join("README.md"))?;
        let about = std::fs::read_to_string(
            repo.join("apps")
                .join("examples_mobile")
                .join("src")
                .join("screens")
                .join("about.rs"),
        )?;

        assert!(readme.contains("`#[rayx::main]`"));
        assert!(readme.contains("RayXApp::builder()"));
        assert!(readme.contains("with_assets"));
        assert!(readme.contains("with_devkit"));
        assert!(readme.contains("artifacts-temp/apps/gpui-mobile-example"));
        assert!(readme.contains("RAYX_IOS_RUST_MANIFEST"));
        for forbidden in [
            "this crate's `android_main",
            "Application::with_platform",
            "`ios_main`",
            "Shared app code plus `android_main` and `ios_main` entrypoints",
            "platform/android/Cargo.toml",
            "platform/ios/Cargo.toml",
            "wrapper static library",
            "wrapper native library",
        ] {
            assert!(
                !readme.contains(forbidden),
                "examples_mobile README should not describe obsolete startup wording `{forbidden}`"
            );
        }

        assert!(!about.contains("\"ios_main()\""));
        assert!(!about.contains("\"android_main()\""));
        assert!(about.contains("\"RayX generated entry\""));
        Ok(())
    }
}
