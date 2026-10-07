use crate::app::{AppDescriptor, CargoMetadataContext};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredAssetPackage {
    pub cargo_package_id: String,
    pub cargo_package_name: String,
    pub cargo_manifest_path: PathBuf,
    pub asset_manifest_path: PathBuf,
    pub asset_package_id: String,
}

pub fn discover_package_assets(
    app: &AppDescriptor,
    context: &CargoMetadataContext,
) -> Result<Vec<DiscoveredAssetPackage>> {
    let mut command = Command::new("cargo");
    command
        .arg("metadata")
        .args(["--format-version", "1", "--manifest-path"])
        .arg(app.manifest());
    context.apply_to(&mut command)?;
    let output = command
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running Cargo metadata for {}", app.manifest().display()))?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed for {}: {}",
            app.manifest().display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let metadata = serde_json::from_slice::<CargoMetadata>(&output.stdout)
        .with_context(|| format!("parsing Cargo metadata for {}", app.manifest().display()))?;
    discover_from_metadata(app, metadata)
}

fn discover_from_metadata(
    app: &AppDescriptor,
    metadata: CargoMetadata,
) -> Result<Vec<DiscoveredAssetPackage>> {
    let app_manifest = fs::canonicalize(app.manifest())
        .with_context(|| format!("canonicalizing {}", app.manifest().display()))?;
    let packages = metadata
        .packages
        .into_iter()
        .map(|package| (package.id.clone(), package))
        .collect::<BTreeMap<_, _>>();
    let mut app_package_id = None;
    for package in packages.values() {
        if canonical_manifest_path(&package.manifest_path)? == app_manifest {
            app_package_id = Some(package.id.clone());
            break;
        }
    }
    let app_package_id = app_package_id.ok_or_else(|| {
        anyhow::anyhow!(
            "Cargo metadata did not contain selected app manifest {}",
            app_manifest.display()
        )
    })?;
    let resolve = metadata
        .resolve
        .ok_or_else(|| anyhow::anyhow!("Cargo metadata did not include a resolve graph"))?;
    if let Some(root) = resolve.root.as_deref()
        && root != app_package_id
    {
        bail!(
            "Cargo metadata resolve.root {root} does not match selected app package {app_package_id}"
        );
    }
    let nodes = resolve
        .nodes
        .into_iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    if !nodes.contains_key(&app_package_id) {
        bail!("Cargo metadata resolve graph has no node for selected app {app_package_id}");
    }

    let mut queue = VecDeque::from([app_package_id.clone()]);
    let mut visited = BTreeSet::new();
    let mut discovered = Vec::new();
    let mut asset_ids = BTreeMap::<String, PathBuf>::new();
    while let Some(package_id) = queue.pop_front() {
        if !visited.insert(package_id.clone()) {
            continue;
        }
        let node = nodes.get(&package_id).ok_or_else(|| {
            anyhow::anyhow!("Cargo metadata resolve graph references missing node {package_id}")
        })?;
        for dependency in &node.deps {
            if dependency.dep_kinds.iter().any(|kind| kind.kind.is_none()) {
                queue.push_back(dependency.pkg.clone());
            }
        }
        if package_id == app_package_id {
            continue;
        }
        let package = packages.get(&package_id).ok_or_else(|| {
            anyhow::anyhow!("Cargo metadata resolve graph references missing package {package_id}")
        })?;
        let Some(asset_manifest_path) = declared_asset_manifest(package)? else {
            continue;
        };
        let asset_package_id = read_asset_package_id(&asset_manifest_path)?;
        if let Some(first_manifest) =
            asset_ids.insert(asset_package_id.clone(), asset_manifest_path.clone())
        {
            bail!(
                "duplicate asset package id {asset_package_id} declared by {} and {}",
                first_manifest.display(),
                asset_manifest_path.display()
            );
        }
        discovered.push(DiscoveredAssetPackage {
            cargo_package_id: package.id.clone(),
            cargo_package_name: package.name.clone(),
            cargo_manifest_path: canonical_manifest_path(&package.manifest_path)?,
            asset_manifest_path,
            asset_package_id,
        });
    }
    discovered.sort_by(|left, right| {
        left.asset_package_id
            .cmp(&right.asset_package_id)
            .then_with(|| left.cargo_package_id.cmp(&right.cargo_package_id))
    });
    Ok(discovered)
}

fn declared_asset_manifest(package: &CargoPackage) -> Result<Option<PathBuf>> {
    let Some(rayx) = package.metadata.get("rayx") else {
        return Ok(None);
    };
    let rayx = rayx
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("package {} metadata.rayx must be a table", package.name))?;
    let Some(assets) = rayx.get("assets") else {
        return Ok(None);
    };
    let assets = assets.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "package {} metadata.rayx.assets must be a table",
            package.name
        )
    })?;
    let manifest = assets
        .get("manifest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "package {} metadata.rayx.assets.manifest must be a string",
                package.name
            )
        })?;
    let manifest = PathBuf::from(manifest);
    if manifest.is_absolute() {
        bail!(
            "package {} asset manifest path must be relative: {}",
            package.name,
            manifest.display()
        );
    }
    let cargo_manifest = canonical_manifest_path(&package.manifest_path)?;
    let package_root = cargo_manifest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("package {} has no manifest parent", package.name))?;
    let asset_manifest = fs::canonicalize(package_root.join(&manifest)).with_context(|| {
        format!(
            "resolving package {} asset manifest {}",
            package.name,
            package_root.join(&manifest).display()
        )
    })?;
    if !asset_manifest.starts_with(package_root) {
        bail!(
            "package {} asset manifest {} must remain beneath {}",
            package.name,
            asset_manifest.display(),
            package_root.display()
        );
    }
    if !asset_manifest.is_file() {
        bail!(
            "package {} asset manifest is not a file: {}",
            package.name,
            asset_manifest.display()
        );
    }
    Ok(Some(asset_manifest))
}

fn canonical_manifest_path(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path)
        .with_context(|| format!("canonicalizing Cargo manifest {}", path.display()))
}

fn read_asset_package_id(manifest_path: &Path) -> Result<String> {
    let text = fs::read_to_string(manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest = toml::from_str::<AssetManifestIdentity>(&text)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
    let package = manifest.asset_package.ok_or_else(|| {
        anyhow::anyhow!(
            "package asset manifest {} must declare [asset_package]",
            manifest_path.display()
        )
    })?;
    validate_asset_package_id(&package.id)
        .with_context(|| format!("validating {}", manifest_path.display()))?;
    Ok(package.id)
}

fn validate_asset_package_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.starts_with('-')
        || id.ends_with('-')
        || id.contains("--")
        || !id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        bail!("asset package id must be lowercase kebab-case: {id}");
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    resolve: Option<CargoResolve>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Deserialize)]
struct CargoResolve {
    root: Option<String>,
    nodes: Vec<CargoNode>,
}

#[derive(Debug, Deserialize)]
struct CargoNode {
    id: String,
    #[serde(default)]
    deps: Vec<CargoNodeDependency>,
}

#[derive(Debug, Deserialize)]
struct CargoNodeDependency {
    pkg: String,
    #[serde(default)]
    dep_kinds: Vec<CargoDependencyKind>,
}

#[derive(Debug, Deserialize)]
struct CargoDependencyKind {
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AssetManifestIdentity {
    asset_package: Option<AssetPackageIdentity>,
}

#[derive(Debug, Deserialize)]
struct AssetPackageIdentity {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::CargoMetadataContext;
    use crate::app::assets;
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn package_asset_discovery_finds_normal_transitive_and_feature_enabled_packages()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root("package-asset-discovery")?;
        write_fixture_workspace(&root)?;
        let app = AppDescriptor::resolve(root.join("app").to_str().expect("utf-8 fixture"))?;

        let packages = discover_package_assets(
            &app,
            &CargoMetadataContext {
                target: Some("x86_64-unknown-linux-gnu".to_string()),
                features: vec!["package-assets".to_string()],
                ..CargoMetadataContext::default()
            },
        )?;
        let ids = packages
            .iter()
            .map(|package| package.asset_package_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, ["feature-assets", "transitive-assets"]);
        assert!(
            packages
                .iter()
                .all(|package| package.asset_manifest_path.is_absolute())
        );
        assert!(
            packages
                .iter()
                .all(|package| package.cargo_manifest_path.is_absolute())
        );

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_dependency_kinds_exclude_dev_build_and_inactive_target_packages()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root("package-asset-dependency-kinds")?;
        write_fixture_workspace(&root)?;
        let app = AppDescriptor::resolve(root.join("app").to_str().expect("utf-8 fixture"))?;

        let packages = discover_package_assets(
            &app,
            &CargoMetadataContext {
                target: Some("x86_64-unknown-linux-gnu".to_string()),
                ..CargoMetadataContext::default()
            },
        )?;
        let ids = packages
            .iter()
            .map(|package| package.asset_package_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, ["transitive-assets"]);
        assert!(!ids.contains(&"dev-assets"));
        assert!(!ids.contains(&"build-assets"));
        assert!(!ids.contains(&"windows-assets"));
        assert!(!ids.contains(&"feature-assets"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_discovery_rejects_duplicate_asset_package_ids()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root("package-asset-discovery-duplicate")?;
        write_fixture_workspace(&root)?;
        write_asset_package(&root, "duplicate-assets", "transitive-assets")?;
        let app_manifest = root.join("app/Cargo.toml");
        let mut text = fs::read_to_string(&app_manifest)?;
        text.push_str("\nduplicate-assets = { path = \"../duplicate-assets\" }\n");
        fs::write(&app_manifest, text)?;
        let app = AppDescriptor::resolve(root.join("app").to_str().expect("utf-8 fixture"))?;

        let error = discover_package_assets(&app, &CargoMetadataContext::default())
            .expect_err("duplicate asset package ids must fail");

        assert!(error.to_string().contains("transitive-assets"));
        assert!(error.to_string().contains("duplicate"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_portability_resolves_copied_crate_from_its_manifest_directory()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root("package asset portability with spaces")?;
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask should be inside the repository root")
            .to_path_buf();
        let source_package = repository_root.join("crates/rayx_components/rayx_components_core");
        let copied_package = root.join("vendor/rayx_components_core");
        fs::create_dir_all(copied_package.join("src"))?;
        fs::write(copied_package.join("src/lib.rs"), "pub fn marker() {}\n")?;
        fs::write(
            copied_package.join("Cargo.toml"),
            "[package]\nname = \"portable-rayx-components-core\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[package.metadata.rayx.assets]\nmanifest = \"rayx.assets.toml\"\n",
        )?;
        fs::copy(
            source_package.join("rayx.assets.toml"),
            copied_package.join("rayx.assets.toml"),
        )?;
        copy_directory(
            &source_package.join("assets"),
            &copied_package.join("assets"),
        )?;

        let app_root = root.join("portable app");
        fs::create_dir_all(app_root.join("src"))?;
        fs::create_dir_all(app_root.join("assets"))?;
        fs::write(app_root.join("src/main.rs"), "fn main() {}\n")?;
        fs::write(
            app_root.join("Cargo.toml"),
            "[package]\nname = \"portable-package-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nportable-rayx-components-core = { path = \"../vendor/rayx_components_core\" }\n",
        )?;
        fs::write(
            app_root.join("rayx.assets.toml"),
            "version = 1\n\n[package_assets]\nmode = \"explicit\"\ninclude = [\"rayx-components-core\"]\n\n[assets]\nroots = [\"assets\"]\n",
        )?;

        let app = AppDescriptor::resolve(app_root.to_str().expect("UTF-8 fixture path"))?;
        let packages = discover_package_assets(&app, &CargoMetadataContext::default())?;
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].asset_package_id, "rayx-components-core");
        assert!(
            packages[0]
                .cargo_manifest_path
                .starts_with(fs::canonicalize(&copied_package)?)
        );

        let staged = root.join("staged app.public");
        assets::package_app_assets(&app, &staged)?;
        assert_eq!(
            fs::read(staged.join("icons/bot.svg"))?,
            fs::read(copied_package.join("assets/icons/bot.svg"))?
        );
        assert_eq!(
            fs::read(staged.join("locales/en/rayx_components_core.json"))?,
            fs::read(copied_package.join("assets/locales/en/rayx_components_core.json"))?
        );
        let index = fs::read_to_string(staged.join("index.json"))?;
        assert!(index.contains("\"locales/en\""));
        assert!(index.contains("\"rayx_components_core\""));

        let sidecar = fs::read_to_string(staged.join("index.entries.json"))?;
        assert!(sidecar.contains("rayx-components-core"));
        let physical_root = root.to_string_lossy();
        assert!(!index.contains(physical_root.as_ref()));
        assert!(!sidecar.contains(physical_root.as_ref()));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    fn write_fixture_workspace(root: &Path) -> Result<()> {
        fs::create_dir_all(root)?;
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\"app\", \"normal\", \"transitive-assets\", \"feature-assets\", \"dev-assets\", \"build-assets\", \"windows-assets\"]\n",
        )?;
        write_plain_package(
            root,
            "normal",
            "transitive-assets = { path = \"../transitive-assets\" }\n",
        )?;
        for (name, id) in [
            ("transitive-assets", "transitive-assets"),
            ("feature-assets", "feature-assets"),
            ("dev-assets", "dev-assets"),
            ("build-assets", "build-assets"),
            ("windows-assets", "windows-assets"),
        ] {
            write_asset_package(root, name, id)?;
        }
        let app = root.join("app");
        fs::create_dir_all(app.join("src"))?;
        fs::write(app.join("src/main.rs"), "fn main() {}\n")?;
        fs::write(
            app.join("Cargo.toml"),
            "[package]\nname = \"fixture-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[features]\npackage-assets = [\"dep:feature-assets\"]\n\n[dependencies]\nnormal = { path = \"../normal\" }\nfeature-assets = { path = \"../feature-assets\", optional = true }\n\n[dev-dependencies]\ndev-assets = { path = \"../dev-assets\" }\n\n[build-dependencies]\nbuild-assets = { path = \"../build-assets\" }\n\n[target.'cfg(target_os = \"windows\")'.dependencies]\nwindows-assets = { path = \"../windows-assets\" }\n",
        )?;
        Ok(())
    }

    fn write_plain_package(root: &Path, name: &str, dependencies: &str) -> Result<()> {
        let package = root.join(name);
        fs::create_dir_all(package.join("src"))?;
        fs::write(package.join("src/lib.rs"), "pub fn marker() {}\n")?;
        fs::write(
            package.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependencies}"
            ),
        )?;
        Ok(())
    }

    fn write_asset_package(root: &Path, name: &str, asset_id: &str) -> Result<()> {
        let package = root.join(name);
        fs::create_dir_all(package.join("src"))?;
        fs::create_dir_all(package.join("assets"))?;
        fs::write(package.join("src/lib.rs"), "pub fn marker() {}\n")?;
        fs::write(package.join("assets/icon.svg"), "<svg/>\n")?;
        fs::write(
            package.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[package.metadata.rayx.assets]\nmanifest = \"rayx.assets.toml\"\n"
            ),
        )?;
        fs::write(
            package.join("rayx.assets.toml"),
            format!(
                "version = 1\n\n[asset_package]\nid = \"{asset_id}\"\nkind = \"test\"\n\n[assets]\nroots = [\"assets\"]\n"
            ),
        )?;
        Ok(())
    }

    fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_directory(&source_path, &destination_path)?;
            } else {
                fs::copy(source_path, destination_path)?;
            }
        }
        Ok(())
    }

    fn fixture_root(label: &str) -> std::io::Result<PathBuf> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rayx-{label}-{nonce}"));
        fs::create_dir_all(&root)?;
        Ok(root)
    }
}
