use crate::app::args::{ensure_empty, take_flag_value};
use crate::app::package_assets::{DiscoveredAssetPackage, discover_package_assets};
use crate::app::{AppDescriptor, AppFeatureSelection, CargoMetadataContext};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn run(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    mut args: Vec<String>,
) -> Result<()> {
    if args.is_empty() {
        print_help(app);
        bail!("assets command requires generate, validate, or list");
    }

    let command = args.remove(0);
    match command.as_str() {
        "generate" => generate_command(app, features, args),
        "validate" => validate_command(app, features, args),
        "list" => list_command(app, features, args),
        other => bail!("unknown assets command: {other}"),
    }
}

#[cfg(test)]
pub fn package_app_assets(app: &AppDescriptor, destination: &Path) -> Result<()> {
    package_app_assets_with_selection(app, &AppFeatureSelection::default(), destination)
}

pub fn package_app_assets_with_selection(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    destination: &Path,
) -> Result<()> {
    package_app_assets_with_additional_roots(app, features, destination, &[])
}

pub fn package_app_content(app: &AppDescriptor, destination: &Path) -> Result<Vec<PathBuf>> {
    let config = AssetBuildConfig::for_app(app)?;
    fs::create_dir_all(destination)
        .with_context(|| format!("creating app content root {}", destination.display()))?;
    let catalog_paths = read_catalog_paths(destination)?;
    let mut copied = Vec::with_capacity(config.app_content.len());
    for content in config.app_content {
        if catalog_paths
            .iter()
            .any(|path| path.eq_ignore_ascii_case(&content.logical_path))
        {
            bail!(
                "app content path {} collides with an AssetService catalog entry",
                content.logical_path
            );
        }
        let destination_path = destination.join(&content.relative_path);
        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating app content directory {}", parent.display()))?;
        }
        fs::copy(&content.source_path, &destination_path).with_context(|| {
            format!(
                "copying declared app content {} to {}",
                content.source_path.display(),
                destination_path.display()
            )
        })?;
        let source_bytes = fs::read(&content.source_path)
            .with_context(|| format!("reading app content {}", content.source_path.display()))?;
        let copied_bytes = fs::read(&destination_path)
            .with_context(|| format!("verifying app content {}", destination_path.display()))?;
        if source_bytes != copied_bytes {
            bail!(
                "copied app content {} does not match declared source bytes",
                content.logical_path
            );
        }
        copied.push(destination_path);
    }
    Ok(copied)
}

fn read_catalog_paths(root: &Path) -> Result<Vec<String>> {
    let index_path = root.join("index.json");
    if !index_path.is_file() {
        return Ok(Vec::new());
    }
    let index = serde_json::from_str::<AssetToolIndex>(
        &fs::read_to_string(&index_path)
            .with_context(|| format!("reading staged catalog {}", index_path.display()))?,
    )?;
    index.paths()
}

pub fn package_app_assets_with_additional_roots(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    destination: &Path,
    additional_roots: &[PathBuf],
) -> Result<()> {
    features.validate_for(&app.manifest())?;
    let graph = AssetBuildGraph::for_app(app, &features.metadata_context())?;
    let source_manifest = app.asset_manifest_path()?;
    let mut entries = graph.collect_entries(&source_manifest)?;
    let mut source_roots = graph
        .app
        .roots
        .iter()
        .chain(
            graph
                .packages
                .iter()
                .filter(|package| package.selected)
                .flat_map(|package| package.config.roots.iter()),
        )
        .cloned()
        .collect::<Vec<_>>();

    if !additional_roots.is_empty() {
        let additional_config = graph.app.clone().with_roots(additional_roots.to_vec());
        for (logical_path, entry) in collect_asset_entries(&additional_config, &source_manifest)? {
            insert_asset_entry(&mut entries, logical_path, entry)?;
        }
        source_roots.extend(additional_config.roots);
    }

    ensure_safe_destination(destination, &source_roots)?;
    let manifest = manifest_from_entries(&graph.app.package, &entries);
    stage_directory_atomically(destination, |staging| {
        for (logical_path, entry) in &entries {
            let staged_path = staging.join(logical_path);
            if let Some(parent) = staged_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::copy(&entry.source_path, &staged_path).with_context(|| {
                format!(
                    "copying asset {} to {}",
                    entry.source_path.display(),
                    staged_path.display()
                )
            })?;
        }
        let manifest_path = staging.join("index.json");
        write_manifest(&manifest_path, &manifest)?;
        let report = validate_manifest_against_graph(
            vec![AssetValidationRoot::direct(staging.to_path_buf())],
            &manifest_path,
            &entries,
        )?;
        if !report.ok {
            bail!(
                "staged asset manifest validation failed for {}: {}",
                manifest_path.display(),
                serde_json::to_string(&report)?
            );
        }
        Ok(())
    })
}

fn stage_directory_atomically(
    destination: &Path,
    populate: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let parent = destination.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "asset staging destination has no parent: {}",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "invalid asset staging destination {}",
                destination.display()
            )
        })?;
    let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let suffix = format!("{}-{sequence}", std::process::id());
    let staging = parent.join(format!(".{name}.staging-{suffix}"));
    let backup = parent.join(format!(".{name}.backup-{suffix}"));
    fs::create_dir(&staging).with_context(|| format!("creating {}", staging.display()))?;

    if let Err(error) = populate(&staging) {
        fs::remove_dir_all(&staging).ok();
        return Err(error);
    }

    let had_destination = destination.exists();
    if had_destination {
        fs::rename(destination, &backup).with_context(|| {
            format!(
                "moving verified asset destination {} to {}",
                destination.display(),
                backup.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(&staging, destination) {
        if had_destination {
            fs::rename(&backup, destination).with_context(|| {
                format!(
                    "restoring verified asset destination {} after activation failure: {error}",
                    destination.display()
                )
            })?;
        }
        fs::remove_dir_all(&staging).ok();
        return Err(error).with_context(|| {
            format!(
                "activating staged asset destination {}",
                destination.display()
            )
        });
    }
    if had_destination {
        fs::remove_dir_all(&backup)
            .with_context(|| format!("removing replaced asset destination {}", backup.display()))?;
    }
    Ok(())
}

fn generate_command(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    mut args: Vec<String>,
) -> Result<()> {
    let output = take_path_flag(&mut args, "--output")?.unwrap_or(app.asset_manifest_path()?);
    ensure_empty(&args)?;
    let graph = AssetBuildGraph::for_app(app, &features.metadata_context())?;
    let manifest = graph.build_manifest(&output)?;
    write_manifest(&output, &manifest)?;
    println!("Generated asset manifest {}", output.display());
    Ok(())
}

fn validate_command(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    mut args: Vec<String>,
) -> Result<()> {
    let manifest = take_path_flag(&mut args, "--manifest")?.unwrap_or(app.asset_manifest_path()?);
    ensure_empty(&args)?;
    let graph = AssetBuildGraph::for_app(app, &features.metadata_context())?;
    let entries = graph.collect_entries(&manifest)?;
    let report =
        validate_manifest_against_graph(graph.selected_validation_roots(), &manifest, &entries)?;
    print_json(&report)?;
    if !report.ok {
        bail!(
            "asset manifest validation failed for {}",
            manifest.display()
        );
    }
    Ok(())
}

fn list_command(
    app: &AppDescriptor,
    features: &AppFeatureSelection,
    mut args: Vec<String>,
) -> Result<()> {
    let manifest = take_path_flag(&mut args, "--manifest")?.unwrap_or(app.asset_manifest_path()?);
    ensure_empty(&args)?;
    let graph = AssetBuildGraph::for_app(app, &features.metadata_context())?;
    let report = graph.list_report(&manifest)?;
    print_json(&report)?;
    Ok(())
}

fn take_path_flag(args: &mut Vec<String>, flag: &str) -> Result<Option<PathBuf>> {
    Ok(take_flag_value(args, flag)?.map(PathBuf::from))
}

#[cfg(test)]
fn build_manifest(app: &AppDescriptor, output: &Path) -> Result<AssetToolManifest> {
    AssetBuildGraph::for_app(app, &CargoMetadataContext::default())?.build_manifest(output)
}

fn collect_asset_entries(
    config: &AssetBuildConfig,
    output: &Path,
) -> Result<BTreeMap<String, CollectedAssetEntry>> {
    let mut entries = BTreeMap::<String, CollectedAssetEntry>::new();
    for root in &config.roots {
        collect_root(root, &root.source_root, output, config, &mut entries)?;
    }
    Ok(entries)
}

fn manifest_from_entries(
    package: &AssetToolPackage,
    entries: &BTreeMap<String, CollectedAssetEntry>,
) -> AssetToolManifest {
    let entries = entries
        .values()
        .map(|entry| entry.manifest_entry.clone())
        .collect::<Vec<_>>();
    let assets = entries.iter().map(|entry| entry.path.clone()).collect();
    AssetToolManifest {
        package: package.clone(),
        assets,
        entries,
    }
}

#[derive(Clone, Debug)]
struct AssetBuildGraph {
    app: AssetBuildConfig,
    packages: Vec<PackageAssetBuildConfig>,
}

#[derive(Clone, Debug)]
struct PackageAssetBuildConfig {
    discovery: DiscoveredAssetPackage,
    config: AssetBuildConfig,
    selected: bool,
}

impl AssetBuildGraph {
    fn for_app(app: &AppDescriptor, context: &CargoMetadataContext) -> Result<Self> {
        let app_config = AssetBuildConfig::for_app(app)?;
        let discovered = discover_package_assets(app, context)?;
        let selected = app_config.package_assets.select(
            discovered
                .iter()
                .map(|package| package.asset_package_id.clone()),
        )?;
        let mut packages = Vec::with_capacity(discovered.len());
        for discovery in discovered {
            let package_root = discovery.cargo_manifest_path.parent().ok_or_else(|| {
                anyhow::anyhow!(
                    "Cargo package {} has no manifest parent",
                    discovery.cargo_package_name
                )
            })?;
            let package = AppDescriptor::resolve(package_root.to_str().ok_or_else(|| {
                anyhow::anyhow!("non-UTF-8 Cargo package path {}", package_root.display())
            })?)?;
            let config = AssetBuildConfig::for_app(&package).with_context(|| {
                format!(
                    "loading asset package {} from {}",
                    discovery.asset_package_id,
                    discovery.asset_manifest_path.display()
                )
            })?;
            if config.package.id != discovery.asset_package_id {
                bail!(
                    "asset package id changed between discovery ({}) and compilation ({}) in {}",
                    discovery.asset_package_id,
                    config.package.id,
                    discovery.asset_manifest_path.display()
                );
            }
            if fs::canonicalize(&config.manifest_path)? != discovery.asset_manifest_path {
                bail!(
                    "asset package {} compiled unexpected manifest {} instead of {}",
                    discovery.asset_package_id,
                    config.manifest_path.display(),
                    discovery.asset_manifest_path.display()
                );
            }
            packages.push(PackageAssetBuildConfig {
                selected: selected.contains(&discovery.asset_package_id),
                discovery,
                config,
            });
        }
        packages.sort_by(|left, right| {
            left.discovery
                .asset_package_id
                .cmp(&right.discovery.asset_package_id)
        });
        Ok(Self {
            app: app_config,
            packages,
        })
    }

    fn collect_entries(&self, output: &Path) -> Result<BTreeMap<String, CollectedAssetEntry>> {
        let mut entries = collect_asset_entries(&self.app, output)?;
        for package in self.packages.iter().filter(|package| package.selected) {
            for (logical_path, entry) in collect_asset_entries(&package.config, output)? {
                insert_asset_entry(&mut entries, logical_path, entry)?;
            }
        }
        Ok(entries)
    }

    fn build_manifest(&self, output: &Path) -> Result<AssetToolManifest> {
        let entries = self.collect_entries(output)?;
        Ok(manifest_from_entries(&self.app.package, &entries))
    }

    fn selected_validation_roots(&self) -> Vec<AssetValidationRoot> {
        self.app
            .roots
            .iter()
            .chain(
                self.packages
                    .iter()
                    .filter(|package| package.selected)
                    .flat_map(|package| package.config.roots.iter()),
            )
            .map(AssetValidationRoot::from_build_root)
            .collect()
    }

    fn list_report(&self, output: &Path) -> Result<AssetGraphListReport> {
        let entries = self.collect_entries(output)?;
        let mut packages = vec![AssetGraphPackageReport::from_config(&self.app, true, None)];
        packages.extend(self.packages.iter().map(|package| {
            AssetGraphPackageReport::from_config(
                &package.config,
                package.selected,
                Some(&package.discovery),
            )
        }));
        let entries = entries
            .into_iter()
            .map(|(logical_path, entry)| AssetGraphEntryReport {
                package_id: entry.package_id,
                asset_manifest: entry.asset_manifest,
                namespace: entry.namespace,
                mount: entry.mount,
                source_root: entry.source_root,
                source_path: entry.source_path,
                logical_path,
            })
            .collect();
        Ok(AssetGraphListReport { packages, entries })
    }
}

fn collect_root(
    root: &AssetBuildRoot,
    current: &Path,
    output: &Path,
    config: &AssetBuildConfig,
    entries: &mut BTreeMap<String, CollectedAssetEntry>,
) -> Result<()> {
    let read_dir = match fs::read_dir(current) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading asset directory {}", current.display()));
        }
    };
    for entry in read_dir {
        let entry =
            entry.with_context(|| format!("reading asset directory {}", current.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading file type {}", path.display()))?;
        if file_type.is_dir() {
            collect_root(root, &path, output, config, entries)?;
            continue;
        }
        if !file_type.is_file()
            || same_path(&path, output)
            || same_path(&path, &entries_manifest_path(output))
        {
            continue;
        }

        let relative = path
            .strip_prefix(&root.source_root)
            .with_context(|| format!("computing asset path for {}", path.display()))?;
        let relative_display = normalize_logical_path(relative)?;
        let logical_path = root.logical_path(relative)?;
        let transforms = run_transform_hooks(config, &root.source_root, &path, &logical_path)?;
        let metadata = entry
            .metadata()
            .with_context(|| format!("reading metadata {}", path.display()))?;
        let sniff_bytes = read_sniff_bytes(&path)?;
        let kind = infer_kind_from_bytes(&logical_path, &sniff_bytes)
            .unwrap_or_else(|| infer_kind(&logical_path))
            .to_string();
        let mime_type = infer_mime_type_from_bytes(&sniff_bytes)
            .or_else(|| infer_mime_type(&logical_path))
            .map(str::to_string);
        let mut tool_entry = AssetToolEntry {
            path: logical_path.clone(),
            package_id: Some(config.package.id.clone()),
            kind,
            mime_type,
            platform: None,
            tags: Vec::new(),
            size_bytes: Some(metadata.len()),
            custom: serde_json::Value::Null,
        };
        apply_asset_config(&logical_path, &mut tool_entry, config, transforms)?;
        insert_asset_entry(
            entries,
            logical_path,
            CollectedAssetEntry {
                manifest_entry: tool_entry,
                package_id: config.package.id.clone(),
                asset_manifest: config.manifest_path.clone(),
                namespace: root.namespace.clone(),
                mount: root.mount.clone(),
                source_root: root.source_root.clone(),
                source_path: path,
                display_path: format!("{}/{}", root.diagnostic_root, relative_display),
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
fn validate_manifest(roots: Vec<PathBuf>, manifest_path: &Path) -> Result<AssetValidationReport> {
    validate_manifest_with_roots(
        roots.into_iter().map(AssetValidationRoot::direct).collect(),
        manifest_path,
    )
}

fn validate_manifest_with_roots(
    roots: Vec<AssetValidationRoot>,
    manifest_path: &Path,
) -> Result<AssetValidationReport> {
    let manifest = read_manifest(manifest_path)?;
    let mut diagnostics = Vec::new();
    let mut asset_paths = BTreeSet::new();
    let mut entry_paths = BTreeSet::new();
    validate_manifest_paths(
        &manifest.assets,
        "assets",
        "asset.manifest.duplicate-asset",
        &mut asset_paths,
        &mut diagnostics,
    );
    validate_manifest_paths(
        &manifest
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>(),
        "entries",
        "asset.manifest.duplicate-entry",
        &mut entry_paths,
        &mut diagnostics,
    );

    for path in asset_paths.difference(&entry_paths) {
        diagnostics.push(AssetValidationDiagnostic::error(
            "asset.manifest.missing-entry",
            path,
            "index path is missing from index.entries.json",
        ));
    }
    for path in entry_paths.difference(&asset_paths) {
        diagnostics.push(AssetValidationDiagnostic::error(
            "asset.manifest.unindexed-entry",
            path,
            "entry metadata path is missing from index.json",
        ));
    }

    for path in asset_paths.union(&entry_paths) {
        if !roots.iter().any(|root| root.contains(path)) {
            diagnostics.push(AssetValidationDiagnostic::error(
                "asset.manifest.missing-file",
                path,
                "manifest path does not exist in any configured asset root",
            ));
        }
    }
    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.code.cmp(right.code))
    });
    Ok(AssetValidationReport {
        ok: diagnostics.is_empty(),
        asset_count: manifest.assets.len(),
        entry_count: manifest.entries.len(),
        diagnostics,
    })
}

fn validate_manifest_against_graph(
    roots: Vec<AssetValidationRoot>,
    manifest_path: &Path,
    expected_entries: &BTreeMap<String, CollectedAssetEntry>,
) -> Result<AssetValidationReport> {
    let mut report = validate_manifest_with_roots(roots, manifest_path)?;
    let manifest = read_manifest(manifest_path)?;
    let actual = manifest.assets.into_iter().collect::<BTreeSet<_>>();
    let expected = expected_entries.keys().cloned().collect::<BTreeSet<_>>();
    for path in expected.difference(&actual) {
        report.diagnostics.push(AssetValidationDiagnostic::error(
            "asset.manifest.missing-graph-entry",
            path,
            "selected app/package asset graph path is missing from the generated manifest",
        ));
    }
    for path in actual.difference(&expected) {
        report.diagnostics.push(AssetValidationDiagnostic::error(
            "asset.manifest.unexpected-graph-entry",
            path,
            "generated manifest path is not present in the selected app/package asset graph",
        ));
    }
    let actual_packages = manifest
        .entries
        .iter()
        .map(|entry| {
            (
                entry.path.as_str(),
                entry
                    .package_id
                    .as_deref()
                    .unwrap_or(manifest.package.id.as_str()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for (path, expected_entry) in expected_entries {
        if let Some(actual_package_id) = actual_packages.get(path.as_str())
            && *actual_package_id != expected_entry.package_id
        {
            report.diagnostics.push(AssetValidationDiagnostic::error(
                "asset.manifest.package-mismatch",
                path,
                format!(
                    "manifest package owner {actual_package_id} does not match selected graph owner {}",
                    expected_entry.package_id
                ),
            ));
        }
    }
    report.diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.code.cmp(right.code))
    });
    report.ok = report.diagnostics.is_empty();
    Ok(report)
}

#[derive(Clone, Debug)]
struct AssetValidationRoot {
    source_root: PathBuf,
    logical_prefix: String,
}

impl AssetValidationRoot {
    fn direct(source_root: PathBuf) -> Self {
        Self {
            source_root,
            logical_prefix: String::new(),
        }
    }

    fn from_build_root(root: &AssetBuildRoot) -> Self {
        Self {
            source_root: root.source_root.clone(),
            logical_prefix: root.logical_prefix.clone(),
        }
    }

    fn contains(&self, logical_path: &str) -> bool {
        if self.logical_prefix.is_empty() {
            return self.source_root.join(logical_path).is_file();
        }
        let prefix = format!("{}/", self.logical_prefix);
        logical_path
            .strip_prefix(&prefix)
            .is_some_and(|relative| self.source_root.join(relative).is_file())
    }
}

fn validate_manifest_paths(
    paths: &[String],
    section: &'static str,
    duplicate_code: &'static str,
    valid_paths: &mut BTreeSet<String>,
    diagnostics: &mut Vec<AssetValidationDiagnostic>,
) {
    let mut seen = BTreeSet::new();
    for path in paths {
        if let Err(reason) = validate_logical_path(path) {
            diagnostics.push(AssetValidationDiagnostic::error(
                "asset.manifest.invalid-path",
                path,
                format!("{section}: {reason}"),
            ));
            continue;
        }
        if !seen.insert(path.clone()) {
            diagnostics.push(AssetValidationDiagnostic::error(
                duplicate_code,
                path,
                format!("path appears more than once in {section}"),
            ));
        }
        valid_paths.insert(path.clone());
    }
}

fn read_manifest(path: &Path) -> Result<AssetToolManifest> {
    let index_text =
        fs::read_to_string(path).with_context(|| format!("reading manifest {}", path.display()))?;
    let index = serde_json::from_str::<AssetToolIndex>(&index_text)
        .with_context(|| format!("parsing manifest {}", path.display()))?;
    let entries_path = entries_manifest_path(path);
    let entries_text = fs::read_to_string(&entries_path)
        .with_context(|| format!("reading manifest {}", entries_path.display()))?;
    let entries = serde_json::from_str::<AssetToolEntriesFile>(&entries_text)
        .with_context(|| format!("parsing manifest {}", entries_path.display()))?;
    Ok(AssetToolManifest {
        package: entries.package(),
        assets: index.paths()?,
        entries: entries.entries(&index)?,
    })
}

fn write_manifest(path: &Path, manifest: &AssetToolManifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let index = AssetToolIndex::from_paths(manifest.entries.iter().map(|entry| &entry.path))?;
    let entries = AssetToolEntriesFile::from_manifest(manifest, &index)?;
    let index_text = serde_json::to_string(&index)?;
    let entries_text = serde_json::to_string(&entries)?;
    fs::write(path, index_text).with_context(|| format!("writing manifest {}", path.display()))?;
    let entries_path = entries_manifest_path(path);
    fs::write(&entries_path, entries_text)
        .with_context(|| format!("writing manifest {}", entries_path.display()))
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn normalize_logical_path(path: &Path) -> Result<String> {
    let path = path.to_string_lossy().replace('\\', "/");
    validate_logical_path(&path).map_err(|reason| anyhow::anyhow!(reason))?;
    Ok(path)
}

fn validate_logical_path(path: &str) -> std::result::Result<(), &'static str> {
    if path.trim().is_empty() {
        return Err("path is empty");
    }
    if path.starts_with('/') || path.contains(':') {
        return Err("path must be relative");
    }
    if path.split('/').any(|part| part == ".." || part.is_empty()) {
        return Err("path must not contain empty or parent segments");
    }
    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    let Ok(left) = fs::canonicalize(left) else {
        return false;
    };
    let Ok(right) = fs::canonicalize(right) else {
        return false;
    };
    left == right
}

fn insert_asset_entry(
    entries: &mut BTreeMap<String, CollectedAssetEntry>,
    logical_path: String,
    entry: CollectedAssetEntry,
) -> Result<()> {
    if let Some(existing) = entries.get(&logical_path) {
        bail!(
            "asset logical path collision for {} between {} and {}",
            logical_path,
            existing.display_path,
            entry.display_path
        );
    }
    entries.insert(logical_path, entry);
    Ok(())
}

fn ensure_safe_destination(destination: &Path, roots: &[AssetBuildRoot]) -> Result<()> {
    let destination = normalize_path_for_compare(destination)?;
    for root in roots {
        let source_root = normalize_path_for_compare(&root.source_root)?;
        if path_contains(&destination, &source_root) || path_contains(&source_root, &destination) {
            bail!(
                "asset staging destination {} overlaps source root {}",
                destination.display(),
                source_root.display()
            );
        }
    }
    Ok(())
}

fn normalize_path_for_compare(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path)
            .with_context(|| format!("canonicalizing {}", path.display()));
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .with_context(|| format!("resolving {}", path.display()))?
            .join(path)
    };
    Ok(normalize_absolute_path(&absolute))
}

fn normalize_absolute_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        use std::path::Component;

        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn path_contains(path: &Path, prefix: &Path) -> bool {
    path == prefix || path.starts_with(prefix)
}

fn entries_manifest_path(index_path: &Path) -> PathBuf {
    let file_name = index_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("index.json");
    let sidecar = file_name
        .strip_suffix(".json")
        .map(|stem| format!("{stem}.entries.json"))
        .unwrap_or_else(|| format!("{file_name}.entries.json"));
    index_path.with_file_name(sidecar)
}

fn split_index_path(path: &str) -> Result<(String, String, String)> {
    validate_logical_path(path).map_err(|reason| anyhow::anyhow!(reason))?;
    let (directory, file_name) = path
        .rsplit_once('/')
        .map(|(directory, file_name)| (directory.to_string(), file_name.to_string()))
        .unwrap_or_else(|| (String::new(), path.to_string()));
    let (stem, extension) = file_name
        .rsplit_once('.')
        .filter(|(stem, extension)| !stem.is_empty() && !extension.is_empty())
        .map(|(stem, extension)| (stem.to_string(), format!(".{extension}")))
        .unwrap_or_else(|| (file_name, String::new()));
    Ok((directory, extension, stem))
}

fn intern(dictionary: &mut Vec<String>, value: &str) -> usize {
    if let Some(index) = dictionary.iter().position(|candidate| candidate == value) {
        index
    } else {
        dictionary.push(value.to_string());
        dictionary.len() - 1
    }
}

fn apply_compact_column<T: Clone>(
    column: &CompactColumn<T>,
    entries: &mut [AssetToolEntry],
    mut apply: impl FnMut(&mut AssetToolEntry, T),
) -> Result<()> {
    if !column.values.is_empty() && column.default.is_some() {
        bail!("entry column cannot combine dense values with a default");
    }
    if !column.values.is_empty() {
        if column.values.len() != entries.len() {
            bail!("entry dense column must align with index asset count");
        }
        for (entry, value) in entries.iter_mut().zip(column.values.iter()) {
            apply(entry, value.clone());
        }
    } else if let Some(value) = &column.default {
        for entry in entries.iter_mut() {
            apply(entry, value.clone());
        }
    }
    for SparseValue(row, value) in &column.overrides {
        let entry = entries
            .get_mut(*row)
            .ok_or_else(|| anyhow::anyhow!("entry sparse row {row} is out of bounds"))?;
        apply(entry, value.clone());
    }
    Ok(())
}

fn apply_compact_dictionary(
    column: &CompactColumn<usize>,
    dictionary: &[String],
    entries: &mut [AssetToolEntry],
    mut apply: impl FnMut(&mut AssetToolEntry, &str),
) -> Result<()> {
    validate_compact_dictionary_column(column, dictionary)?;
    apply_compact_column(column, entries, |entry, index| {
        if let Some(value) = dictionary.get(index) {
            apply(entry, value);
        }
    })
}

fn apply_compact_u64(
    column: &CompactColumn<u64>,
    entries: &mut [AssetToolEntry],
    mut apply: impl FnMut(&mut AssetToolEntry, u64),
) -> Result<()> {
    apply_compact_column(column, entries, |entry, value| apply(entry, value))
}

fn apply_compact_tags(
    column: &CompactColumn<Vec<String>>,
    entries: &mut [AssetToolEntry],
) -> Result<()> {
    apply_compact_column(column, entries, |entry, value| entry.tags = value)
}

fn apply_compact_json(
    column: &CompactColumn<serde_json::Value>,
    entries: &mut [AssetToolEntry],
) -> Result<()> {
    apply_compact_column(column, entries, |entry, value| entry.custom = value)
}

fn apply_bool_column(
    column: &BoolColumn,
    entries: &mut [AssetToolEntry],
    mut apply: impl FnMut(&mut AssetToolEntry, bool),
) -> Result<()> {
    if column.default {
        for entry in entries.iter_mut() {
            apply(entry, true);
        }
    }
    for row in &column.rows {
        let entry = entries
            .get_mut(*row)
            .ok_or_else(|| anyhow::anyhow!("entry sparse row {row} is out of bounds"))?;
        apply(entry, !column.default);
    }
    Ok(())
}

fn validate_compact_dictionary_column<T>(
    column: &CompactColumn<usize>,
    dictionary: &[T],
) -> Result<()> {
    if let Some(index) = column.default
        && index >= dictionary.len()
    {
        bail!("entry dictionary index {index} is out of bounds");
    }
    for index in &column.values {
        if *index >= dictionary.len() {
            bail!("entry dictionary index {index} is out of bounds");
        }
    }
    for SparseValue(_, index) in &column.overrides {
        if *index >= dictionary.len() {
            bail!("entry dictionary index {index} is out of bounds");
        }
    }
    Ok(())
}

fn insert_custom_string(entry: &mut AssetToolEntry, key: &str, value: &str) {
    if !entry.custom.is_object() {
        entry.custom = serde_json::Value::Object(serde_json::Map::new());
    }
    if let Some(object) = entry.custom.as_object_mut() {
        object.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn insert_custom_bool(entry: &mut AssetToolEntry, key: &str, value: bool) {
    if !entry.custom.is_object() {
        entry.custom = serde_json::Value::Object(serde_json::Map::new());
    }
    if let Some(object) = entry.custom.as_object_mut() {
        object.insert(key.to_string(), serde_json::Value::Bool(value));
    }
}

fn set_custom_bool(entry: &mut AssetToolEntry, key: &str, value: bool) {
    if value {
        insert_custom_bool(entry, key, true);
        return;
    }
    if let Some(object) = entry.custom.as_object_mut() {
        object.remove(key);
        if object.is_empty() {
            entry.custom = serde_json::Value::Null;
        }
    }
}

fn compact_custom(
    custom: serde_json::Value,
) -> (Option<String>, Option<String>, bool, serde_json::Value) {
    let serde_json::Value::Object(mut object) = custom else {
        return (None, None, false, custom);
    };
    let asset_group = object
        .remove("assetGroup")
        .and_then(|value| value.as_str().map(str::to_string));
    let delivery = object
        .remove("delivery")
        .and_then(|value| value.as_str().map(str::to_string));
    let preload = match object.remove("preload") {
        Some(serde_json::Value::Bool(true)) => true,
        Some(value) => {
            object.insert("preload".to_string(), value);
            false
        }
        None => false,
    };
    object.remove("platform");
    if object.is_empty() {
        (asset_group, delivery, preload, serde_json::Value::Null)
    } else {
        (
            asset_group,
            delivery,
            preload,
            serde_json::Value::Object(object),
        )
    }
}

fn infer_kind(path: &str) -> &'static str {
    match extension(path) {
        ".svg" => "svg",
        ".png" | ".jpg" | ".jpeg" | ".webp" | ".avif" | ".gif" => "image",
        ".ttf" | ".otf" | ".woff" | ".woff2" => "font",
        ".json" | ".toml" => "data",
        ".md" | ".txt" | ".ron" | ".yaml" | ".yml" => "text",
        ".mp3" | ".ogg" | ".wav" | ".flac" => "audio",
        ".mp4" | ".webm" | ".mov" => "video",
        _ => "binary",
    }
}

fn infer_kind_from_bytes(path: &str, bytes: &[u8]) -> Option<&'static str> {
    infer_mime_type_from_bytes(bytes).map(|mime_type| match mime_type {
        "image/svg+xml" => "svg",
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif" => "image",
        "font/ttf" | "font/otf" | "font/woff" | "font/woff2" => "font",
        "application/json" => match path.replace('\\', "/").to_ascii_lowercase() {
            normalized if normalized.split('/').any(|segment| segment == "locales") => "locale",
            normalized
                if normalized.split('/').any(|segment| segment == "themes")
                    || normalized.ends_with(".dtcg.json") =>
            {
                "theme"
            }
            normalized if normalized.split('/').any(|segment| segment == "fixtures") => "fixture",
            _ => "data",
        },
        "audio/mpeg" | "audio/ogg" | "audio/wav" | "audio/flac" | "audio/mp4" => "audio",
        "video/mp4" | "video/webm" | "video/quicktime" => "video",
        _ => "binary",
    })
}

fn infer_mime_type(path: &str) -> Option<&'static str> {
    match extension(path) {
        ".svg" => Some("image/svg+xml"),
        ".png" => Some("image/png"),
        ".jpg" | ".jpeg" => Some("image/jpeg"),
        ".webp" => Some("image/webp"),
        ".avif" => Some("image/avif"),
        ".gif" => Some("image/gif"),
        ".json" => Some("application/json"),
        ".toml" => Some("application/toml"),
        ".md" => Some("text/markdown; charset=utf-8"),
        ".txt" => Some("text/plain; charset=utf-8"),
        ".ttf" => Some("font/ttf"),
        ".otf" => Some("font/otf"),
        ".woff" => Some("font/woff"),
        ".woff2" => Some("font/woff2"),
        ".mp3" => Some("audio/mpeg"),
        ".ogg" => Some("audio/ogg"),
        ".wav" => Some("audio/wav"),
        ".mp4" => Some("video/mp4"),
        ".webm" => Some("video/webm"),
        _ => None,
    }
}

fn infer_mime_type_from_bytes(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if matches!(brand, b"avif" | b"avis") {
            return Some("image/avif");
        }
        if matches!(brand, b"M4A " | b"M4B " | b"M4P ") {
            return Some("audio/mp4");
        }
        if matches!(brand, b"qt  ") {
            return Some("video/quicktime");
        }
        return Some("video/mp4");
    }
    if bytes.starts_with(b"wOFF") {
        return Some("font/woff");
    }
    if bytes.starts_with(b"wOF2") {
        return Some("font/woff2");
    }
    if bytes.starts_with(b"OTTO") {
        return Some("font/otf");
    }
    if bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]) {
        return Some("font/ttf");
    }
    if bytes.starts_with(b"ID3") || bytes.first().is_some_and(|first| *first == 0xff) {
        return Some("audio/mpeg");
    }
    if bytes.starts_with(b"OggS") {
        return Some("audio/ogg");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        return Some("audio/wav");
    }
    if bytes.starts_with(b"fLaC") {
        return Some("audio/flac");
    }
    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some("video/webm");
    }
    if looks_like_svg(bytes) {
        return Some("image/svg+xml");
    }
    if looks_like_json(bytes) {
        return Some("application/json");
    }
    None
}

fn read_sniff_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut bytes = vec![0; 4096];
    let len = file
        .read(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    bytes.truncate(len);
    Ok(bytes)
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg"))
}

fn looks_like_json(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with('{') || text.starts_with('[')
}

fn extension(path: &str) -> &str {
    path.rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
        .map(|extension| {
            let start = path.len() - extension.len() - 1;
            &path[start..]
        })
        .unwrap_or_default()
}

fn print_help(app: &AppDescriptor) {
    println!("Asset commands for {}:", app.slug);
    println!(
        "  app {} assets generate [--output <path>]",
        app.root.display()
    );
    println!(
        "  app {} assets validate [--manifest <path>]",
        app.root.display()
    );
    println!(
        "  app {} assets list [--manifest <path>]",
        app.root.display()
    );
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetToolManifest {
    package: AssetToolPackage,
    assets: Vec<String>,
    entries: Vec<AssetToolEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetToolPackage {
    id: String,
    name: String,
    kind: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetGraphListReport {
    packages: Vec<AssetGraphPackageReport>,
    entries: Vec<AssetGraphEntryReport>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetGraphPackageReport {
    package_id: String,
    package_kind: String,
    cargo_package_id: Option<String>,
    cargo_manifest: Option<PathBuf>,
    asset_manifest: PathBuf,
    selected: bool,
    roots: Vec<AssetGraphRootReport>,
}

impl AssetGraphPackageReport {
    fn from_config(
        config: &AssetBuildConfig,
        selected: bool,
        discovery: Option<&DiscoveredAssetPackage>,
    ) -> Self {
        Self {
            package_id: config.package.id.clone(),
            package_kind: config.package.kind.clone(),
            cargo_package_id: discovery.map(|package| package.cargo_package_id.clone()),
            cargo_manifest: discovery.map(|package| package.cargo_manifest_path.clone()),
            asset_manifest: config.manifest_path.clone(),
            selected,
            roots: config
                .roots
                .iter()
                .map(|root| AssetGraphRootReport {
                    source_root: root.source_root.clone(),
                    namespace: root.namespace.clone(),
                    mount: root.mount.clone(),
                    logical_prefix: root.logical_prefix.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetGraphRootReport {
    source_root: PathBuf,
    namespace: String,
    mount: String,
    logical_prefix: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetGraphEntryReport {
    package_id: String,
    asset_manifest: PathBuf,
    namespace: String,
    mount: String,
    source_root: PathBuf,
    source_path: PathBuf,
    logical_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetToolEntry {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    package_id: Option<String>,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "serde_json_value_is_null")]
    custom: serde_json::Value,
}

#[derive(Clone, Debug)]
struct AssetBuildRoot {
    source_root: PathBuf,
    diagnostic_root: String,
    namespace: String,
    mount: String,
    logical_prefix: String,
}

impl AssetBuildRoot {
    fn direct(source_root: PathBuf) -> Self {
        let diagnostic_root = source_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("assets")
            .to_string();
        Self {
            source_root,
            diagnostic_root,
            namespace: String::new(),
            mount: String::new(),
            logical_prefix: String::new(),
        }
    }

    fn logical_path(&self, relative: &Path) -> Result<String> {
        let relative = normalize_logical_path(relative)?;
        if self.logical_prefix.is_empty() {
            return Ok(relative);
        }
        let logical_path = format!("{}/{relative}", self.logical_prefix);
        validate_logical_path(&logical_path).map_err(|reason| anyhow::anyhow!(reason))?;
        Ok(logical_path)
    }
}

#[derive(Clone, Debug)]
struct CollectedAssetEntry {
    manifest_entry: AssetToolEntry,
    package_id: String,
    asset_manifest: PathBuf,
    namespace: String,
    mount: String,
    source_root: PathBuf,
    source_path: PathBuf,
    display_path: String,
}

fn default_tool_entry(path: &str) -> AssetToolEntry {
    AssetToolEntry {
        path: path.to_string(),
        package_id: None,
        kind: infer_kind(path).to_string(),
        mime_type: infer_mime_type(path).map(str::to_string),
        platform: None,
        tags: Vec::new(),
        size_bytes: None,
        custom: serde_json::Value::Null,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AssetToolIndex {
    assets: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

impl AssetToolIndex {
    fn from_paths<'a>(paths: impl IntoIterator<Item = &'a String>) -> Result<Self> {
        let mut index = Self {
            assets: BTreeMap::new(),
        };
        for path in paths {
            index.insert(path)?;
        }
        Ok(index)
    }

    fn insert(&mut self, path: &str) -> Result<()> {
        let (directory, extension, stem) = split_index_path(path)?;
        let stems = self
            .assets
            .entry(directory)
            .or_default()
            .entry(extension)
            .or_default();
        if !stems.iter().any(|candidate| candidate == &stem) {
            stems.push(stem);
            stems.sort();
        }
        Ok(())
    }

    fn paths(&self) -> Result<Vec<String>> {
        let mut paths = Vec::new();
        for (directory, extensions) in &self.assets {
            for (extension, stems) in extensions {
                for stem in stems {
                    let path = if directory.is_empty() {
                        format!("{stem}{extension}")
                    } else {
                        format!("{directory}/{stem}{extension}")
                    };
                    paths.push(path);
                }
            }
        }
        Ok(paths)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetToolEntriesFile {
    package: Vec<String>,
    dict: AssetToolDictionaries,
    columns: AssetToolColumns,
}

impl AssetToolEntriesFile {
    fn from_manifest(manifest: &AssetToolManifest, index: &AssetToolIndex) -> Result<Self> {
        let mut entries_by_path = manifest
            .entries
            .iter()
            .map(|entry| (entry.path.clone(), entry.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut dict = AssetToolDictionaries::default();
        let mut columns = AssetToolColumns::default();
        let row_count = index.paths()?.len();
        let mut package_id_values = Vec::with_capacity(row_count);
        let mut size_values = Vec::with_capacity(row_count);
        let mut platform_values = Vec::with_capacity(row_count);
        let mut asset_group_values = Vec::with_capacity(row_count);
        let mut delivery_values = Vec::with_capacity(row_count);
        let mut preload_values = Vec::with_capacity(row_count);

        for (row, path) in index.paths()?.into_iter().enumerate() {
            let entry = entries_by_path
                .remove(&path)
                .unwrap_or_else(|| default_tool_entry(&path));
            package_id_values.push(
                entry
                    .package_id
                    .as_deref()
                    .filter(|package_id| *package_id != manifest.package.id)
                    .map(|package_id| dict.package_id_id(package_id)),
            );
            if entry.kind != infer_kind(&path) {
                columns
                    .kind
                    .overrides
                    .push(SparseValue(row, dict.kind_id(&entry.kind)));
            }
            if let Some(mime_type) = &entry.mime_type
                && Some(mime_type.as_str()) != infer_mime_type(&path)
            {
                columns
                    .mime_type
                    .overrides
                    .push(SparseValue(row, dict.mime_type_id(mime_type)));
            }
            size_values.push(entry.size_bytes);
            platform_values.push(
                entry
                    .platform
                    .as_ref()
                    .map(|platform| dict.platform_id(platform)),
            );
            if !entry.tags.is_empty() {
                columns
                    .tags
                    .overrides
                    .push(SparseValue(row, entry.tags.clone()));
            }
            let (asset_group, delivery, preload, custom) = compact_custom(entry.custom.clone());
            asset_group_values
                .push(asset_group.map(|asset_group| dict.asset_group_id(&asset_group)));
            delivery_values.push(delivery.map(|delivery| dict.delivery_id(&delivery)));
            preload_values.push(preload);
            if !custom.is_null() {
                columns.custom.overrides.push(SparseValue(row, custom));
            }
        }
        columns.package_id = compact_optional_column(package_id_values);
        columns.size_bytes = compact_optional_column(size_values);
        columns.platform = compact_optional_column(platform_values);
        columns.asset_group = compact_optional_column(asset_group_values);
        columns.delivery = compact_optional_column(delivery_values);
        columns.preload = BoolColumn::from_values(&preload_values);

        Ok(Self {
            package: vec![
                manifest.package.id.clone(),
                manifest.package.name.clone(),
                manifest.package.kind.clone(),
            ],
            dict,
            columns,
        })
    }

    fn package(&self) -> AssetToolPackage {
        AssetToolPackage {
            id: self.package.first().cloned().unwrap_or_default(),
            name: self.package.get(1).cloned().unwrap_or_default(),
            kind: self.package.get(2).cloned().unwrap_or_default(),
        }
    }

    fn entries(&self, index: &AssetToolIndex) -> Result<Vec<AssetToolEntry>> {
        let paths = index.paths()?;
        let package_id = self.package.first().cloned().unwrap_or_default();
        let mut entries = Vec::with_capacity(paths.len());
        for path in &paths {
            entries.push(AssetToolEntry {
                path: path.clone(),
                package_id: Some(package_id.clone()),
                kind: infer_kind(path).to_string(),
                mime_type: infer_mime_type(path).map(str::to_string),
                platform: None,
                tags: Vec::new(),
                size_bytes: None,
                custom: serde_json::Value::Null,
            });
        }
        apply_compact_dictionary(
            &self.columns.package_id,
            &self.dict.package_id,
            &mut entries,
            |entry, value| entry.package_id = Some(value.to_string()),
        )?;
        apply_compact_dictionary(
            &self.columns.kind,
            &self.dict.kind,
            &mut entries,
            |entry, value| entry.kind = value.to_string(),
        )?;
        apply_compact_dictionary(
            &self.columns.mime_type,
            &self.dict.mime_type,
            &mut entries,
            |entry, value| entry.mime_type = Some(value.to_string()),
        )?;
        apply_compact_u64(&self.columns.size_bytes, &mut entries, |entry, value| {
            entry.size_bytes = Some(value);
        })?;
        apply_compact_dictionary(
            &self.columns.platform,
            &self.dict.platform,
            &mut entries,
            |entry, value| {
                entry.platform = Some(value.to_string());
                insert_custom_string(entry, "platform", value);
            },
        )?;
        apply_compact_tags(&self.columns.tags, &mut entries)?;
        apply_compact_json(&self.columns.custom, &mut entries)?;
        apply_bool_column(&self.columns.preload, &mut entries, |entry, value| {
            set_custom_bool(entry, "preload", value);
        })?;
        apply_compact_dictionary(
            &self.columns.asset_group,
            &self.dict.asset_group,
            &mut entries,
            |entry, value| insert_custom_string(entry, "assetGroup", value),
        )?;
        apply_compact_dictionary(
            &self.columns.delivery,
            &self.dict.delivery,
            &mut entries,
            |entry, value| insert_custom_string(entry, "delivery", value),
        )?;
        Ok(entries)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetToolDictionaries {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    package_id: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    kind: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mime_type: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    asset_group: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    delivery: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    platform: Vec<String>,
}

impl AssetToolDictionaries {
    fn package_id_id(&mut self, value: &str) -> usize {
        intern(&mut self.package_id, value)
    }

    fn kind_id(&mut self, value: &str) -> usize {
        intern(&mut self.kind, value)
    }

    fn mime_type_id(&mut self, value: &str) -> usize {
        intern(&mut self.mime_type, value)
    }

    fn asset_group_id(&mut self, value: &str) -> usize {
        intern(&mut self.asset_group, value)
    }

    fn delivery_id(&mut self, value: &str) -> usize {
        intern(&mut self.delivery, value)
    }

    fn platform_id(&mut self, value: &str) -> usize {
        intern(&mut self.platform, value)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetToolColumns {
    #[serde(default, skip_serializing_if = "CompactColumn::is_empty")]
    package_id: CompactColumn<usize>,
    #[serde(default, skip_serializing_if = "CompactColumn::is_empty")]
    kind: CompactColumn<usize>,
    #[serde(default, skip_serializing_if = "CompactColumn::is_empty")]
    mime_type: CompactColumn<usize>,
    #[serde(default, skip_serializing_if = "CompactColumn::is_empty")]
    size_bytes: CompactColumn<u64>,
    #[serde(default, skip_serializing_if = "CompactColumn::is_empty")]
    platform: CompactColumn<usize>,
    #[serde(default, skip_serializing_if = "CompactColumn::is_empty")]
    tags: CompactColumn<Vec<String>>,
    #[serde(default, skip_serializing_if = "CompactColumn::is_empty")]
    asset_group: CompactColumn<usize>,
    #[serde(default, skip_serializing_if = "CompactColumn::is_empty")]
    delivery: CompactColumn<usize>,
    #[serde(default, skip_serializing_if = "BoolColumn::is_empty")]
    preload: BoolColumn,
    #[serde(default, skip_serializing_if = "CompactColumn::is_empty")]
    custom: CompactColumn<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SparseValue<T>(usize, T);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompactColumn<T> {
    #[serde(default, rename = "d", skip_serializing_if = "Option::is_none")]
    default: Option<T>,
    #[serde(default, rename = "v", skip_serializing_if = "Vec::is_empty")]
    values: Vec<T>,
    #[serde(default, rename = "o", skip_serializing_if = "Vec::is_empty")]
    overrides: Vec<SparseValue<T>>,
}

impl<T> Default for CompactColumn<T> {
    fn default() -> Self {
        Self {
            default: None,
            values: Vec::new(),
            overrides: Vec::new(),
        }
    }
}

impl<T> CompactColumn<T> {
    fn is_empty(&self) -> bool {
        self.default.is_none() && self.values.is_empty() && self.overrides.is_empty()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct BoolColumn {
    #[serde(default, rename = "d", skip_serializing_if = "is_false")]
    default: bool,
    #[serde(default, rename = "r", skip_serializing_if = "Vec::is_empty")]
    rows: Vec<usize>,
}

impl BoolColumn {
    fn is_empty(&self) -> bool {
        !self.default && self.rows.is_empty()
    }

    fn from_values(values: &[bool]) -> Self {
        let true_count = values.iter().filter(|value| **value).count();
        let false_count = values.len().saturating_sub(true_count);
        let default = true_count > false_count;
        let rows = values
            .iter()
            .enumerate()
            .filter_map(|(row, value)| (*value != default).then_some(row))
            .collect();
        Self { default, rows }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn compact_optional_column<T: Clone + Ord>(values: Vec<Option<T>>) -> CompactColumn<T> {
    if values.iter().all(Option::is_some) {
        return compact_dense_or_default_column(values.into_iter().flatten().collect());
    }
    let overrides = values
        .into_iter()
        .enumerate()
        .filter_map(|(row, value)| value.map(|value| SparseValue(row, value)))
        .collect();
    CompactColumn {
        overrides,
        ..CompactColumn::default()
    }
}

fn compact_dense_or_default_column<T: Clone + Ord>(values: Vec<T>) -> CompactColumn<T> {
    if values.is_empty() {
        return CompactColumn::default();
    }
    let mut counts = BTreeMap::<T, usize>::new();
    for value in &values {
        *counts.entry(value.clone()).or_default() += 1;
    }
    let Some((default, default_count)) = counts.into_iter().max_by_key(|(_, count)| *count) else {
        return CompactColumn::default();
    };
    if default_count <= values.len() / 2 {
        return CompactColumn {
            values,
            ..CompactColumn::default()
        };
    }
    let overrides = values
        .into_iter()
        .enumerate()
        .filter_map(|(row, value)| (value != default).then_some(SparseValue(row, value)))
        .collect();
    CompactColumn {
        default: Some(default),
        overrides,
        ..CompactColumn::default()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetValidationReport {
    ok: bool,
    asset_count: usize,
    entry_count: usize,
    diagnostics: Vec<AssetValidationDiagnostic>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetValidationDiagnostic {
    severity: &'static str,
    code: &'static str,
    path: String,
    message: String,
}

impl AssetValidationDiagnostic {
    fn error(code: &'static str, path: &str, message: impl Into<String>) -> Self {
        Self {
            severity: "error",
            code,
            path: path.to_string(),
            message: message.into(),
        }
    }
}

fn serde_json_value_is_null(value: &serde_json::Value) -> bool {
    value.is_null()
}

#[derive(Clone, Debug)]
struct AssetBuildConfig {
    package: AssetToolPackage,
    roots: Vec<AssetBuildRoot>,
    manifest_path: PathBuf,
    manifest_root: PathBuf,
    authoring_enabled: bool,
    package_assets: PackageAssetsSelection,
    default_group: String,
    groups: BTreeMap<String, AssetGroupConfig>,
    extension_policies: BTreeMap<String, ExtensionPolicyConfig>,
    platform_aliases: BTreeMap<String, String>,
    dpi_directories: BTreeSet<String>,
    transforms: Vec<TransformHookConfig>,
    app_content: Vec<AppContentFile>,
}

impl AssetBuildConfig {
    fn for_app(app: &AppDescriptor) -> Result<Self> {
        let toml_path = app.root.join("rayx.assets.toml");
        if !toml_path.is_file() {
            return Ok(Self::from_app_roots(
                &app.slug,
                app.asset_roots()?,
                app.root.clone(),
                app.manifest(),
            ));
        }

        let text = fs::read_to_string(&toml_path)
            .with_context(|| format!("reading {}", toml_path.display()))?;
        let manifest = toml::from_str::<AssetTomlConfig>(&text)
            .with_context(|| format!("parsing {}", toml_path.display()))?;
        if let Some(version) = manifest.version
            && version != 1
        {
            bail!(
                "{} declares unsupported version {version}; supported version is 1",
                toml_path.display()
            );
        }

        let package_assets = PackageAssetsSelection::resolve(manifest.package_assets)?;
        let assets = manifest.assets.unwrap_or_default();
        let package = resolve_manifest_package(app, manifest.asset_package.as_ref())?;
        let package_namespace =
            resolve_package_namespace(manifest.asset_package.as_ref(), &package.id)?;
        let roots = if assets.roots.is_empty() {
            app.asset_roots()?
                .into_iter()
                .map(AssetBuildRoot::direct)
                .collect()
        } else {
            assets
                .roots
                .into_iter()
                .map(|root| {
                    normalize_asset_root(
                        &toml_path,
                        &app.root,
                        &package_namespace,
                        manifest.asset_package.is_some(),
                        root,
                    )
                })
                .collect::<Result<Vec<_>>>()?
        };
        let default_group = assets
            .default_group
            .or(assets.default_bundle)
            .unwrap_or_else(|| "app".to_string());
        let mut groups = BTreeMap::new();
        for group in manifest.asset_group {
            let id = group.id.clone();
            if groups.insert(id.clone(), group).is_some() {
                bail!(
                    "{} declares duplicate asset group id {id}",
                    toml_path.display()
                );
            }
        }
        groups.entry(default_group.clone()).or_insert_with(|| {
            AssetGroupConfig::default_group(default_group.clone(), Some("bundled".to_string()))
        });

        let extension_policies = normalize_extension_policies(manifest.policies.by_extension)?;
        let platform_aliases =
            platform_aliases(assets.platform_layout.as_deref(), assets.platforms)?;
        let dpi_directories = dpi_directories(assets.variant_layout.as_deref(), assets.dpi)?;
        let transforms = normalize_transform_hooks(manifest.transform)?;
        let app_content =
            resolve_app_content(&toml_path, &app.app_content_root()?, manifest.app_content)?;

        Ok(Self {
            package,
            roots,
            manifest_path: toml_path,
            manifest_root: app.root.clone(),
            authoring_enabled: true,
            package_assets,
            default_group,
            groups,
            extension_policies,
            platform_aliases,
            dpi_directories,
            transforms,
            app_content,
        })
    }

    fn from_app_roots(
        package_id: &str,
        roots: Vec<PathBuf>,
        app_root: PathBuf,
        manifest_path: PathBuf,
    ) -> Self {
        let default_group = "app".to_string();
        let mut groups = BTreeMap::new();
        groups.insert(
            default_group.clone(),
            AssetGroupConfig::default_group(default_group.clone(), None),
        );
        Self {
            package: AssetToolPackage {
                id: package_id.to_string(),
                name: package_id.to_string(),
                kind: "app".to_string(),
            },
            roots: roots.into_iter().map(AssetBuildRoot::direct).collect(),
            manifest_path,
            manifest_root: app_root,
            authoring_enabled: false,
            package_assets: PackageAssetsSelection::default(),
            default_group,
            groups,
            extension_policies: BTreeMap::new(),
            platform_aliases: BTreeMap::new(),
            dpi_directories: BTreeSet::new(),
            transforms: Vec::new(),
            app_content: Vec::new(),
        }
    }

    fn with_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.roots = roots.into_iter().map(AssetBuildRoot::direct).collect();
        self
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
struct AssetTomlConfig {
    version: Option<u32>,
    asset_package: Option<AssetPackageTomlConfig>,
    assets: Option<AssetTomlAssets>,
    package_assets: Option<PackageAssetsTomlConfig>,
    #[serde(default)]
    asset_group: Vec<AssetGroupConfig>,
    #[serde(default)]
    policies: AssetPoliciesConfig,
    #[serde(default)]
    transform: Vec<TransformHookConfig>,
    #[serde(default)]
    app_content: AppContentTomlConfig,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct AppContentTomlConfig {
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Clone, Debug)]
struct AppContentFile {
    logical_path: String,
    relative_path: PathBuf,
    source_path: PathBuf,
}

fn resolve_app_content(
    manifest_path: &Path,
    content_root: &Path,
    config: AppContentTomlConfig,
) -> Result<Vec<AppContentFile>> {
    let canonical_root = fs::canonicalize(content_root)
        .with_context(|| format!("canonicalizing app content root {}", content_root.display()))?;
    let mut logical_paths = BTreeSet::new();
    let mut files = Vec::with_capacity(config.files.len());
    for declared in config.files {
        let logical_path = declared.replace('\\', "/");
        validate_logical_path(&logical_path).map_err(|reason| {
            anyhow::anyhow!(
                "{} declares invalid app_content file {:?}: {reason}",
                manifest_path.display(),
                declared
            )
        })?;
        if logical_path.split('/').any(|segment| segment == ".") {
            bail!(
                "{} declares invalid app_content file {:?}: current-directory segments are forbidden",
                manifest_path.display(),
                declared
            );
        }
        if logical_path
            .split('/')
            .next()
            .is_some_and(|segment| segment.eq_ignore_ascii_case("assets"))
            || logical_path.eq_ignore_ascii_case("index.json")
            || logical_path.eq_ignore_ascii_case("index.entries.json")
        {
            bail!(
                "{} declares app_content file {:?} that collides with the AssetService deployment namespace",
                manifest_path.display(),
                declared
            );
        }
        let collision_key = logical_path.to_ascii_lowercase();
        if !logical_paths.insert(collision_key.clone())
            || logical_paths.iter().any(|existing| {
                existing != &collision_key
                    && (existing.starts_with(&format!("{collision_key}/"))
                        || collision_key.starts_with(&format!("{existing}/")))
            })
        {
            bail!(
                "{} declares duplicate or colliding app_content path {:?}",
                manifest_path.display(),
                declared
            );
        }

        let relative_path = PathBuf::from(&logical_path);
        let mut current = content_root.to_path_buf();
        for component in relative_path.components() {
            current.push(component.as_os_str());
            let metadata = fs::symlink_metadata(&current).with_context(|| {
                format!(
                    "declared app_content file {:?} is missing at {}",
                    declared,
                    current.display()
                )
            })?;
            if metadata.file_type().is_symlink() {
                bail!(
                    "declared app_content file {:?} traverses symlink {}",
                    declared,
                    current.display()
                );
            }
        }
        let metadata = fs::metadata(&current)
            .with_context(|| format!("reading app_content file {}", current.display()))?;
        if !metadata.is_file() {
            bail!(
                "declared app_content path {} is not a regular file",
                current.display()
            );
        }
        let canonical_source = fs::canonicalize(&current)
            .with_context(|| format!("canonicalizing app_content file {}", current.display()))?;
        if !canonical_source.starts_with(&canonical_root) {
            bail!(
                "declared app_content file {} escapes app content root {}",
                canonical_source.display(),
                canonical_root.display()
            );
        }
        files.push(AppContentFile {
            logical_path,
            relative_path,
            source_path: canonical_source,
        });
    }
    Ok(files)
}

#[derive(Clone, Debug, Deserialize, Default)]
struct AssetTomlAssets {
    #[serde(default)]
    roots: Vec<AssetRootConfig>,
    default_group: Option<String>,
    default_bundle: Option<String>,
    variant_layout: Option<String>,
    platform_layout: Option<String>,
    #[serde(default)]
    platforms: BTreeMap<String, Vec<String>>,
    dpi: Option<AssetDpiConfig>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct AssetDpiConfig {
    #[serde(default)]
    directories: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct AssetPackageTomlConfig {
    id: String,
    kind: String,
    namespace: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct PackageAssetsTomlConfig {
    mode: Option<String>,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum AssetRootConfig {
    Path(String),
    Mounted { path: String, mount: String },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PackageAssetsMode {
    #[default]
    Auto,
    Explicit,
    Disabled,
}

#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
struct PackageAssetsSelection {
    mode: PackageAssetsMode,
    include: BTreeSet<String>,
    exclude: BTreeSet<String>,
}

impl PackageAssetsSelection {
    fn resolve(config: Option<PackageAssetsTomlConfig>) -> Result<Self> {
        let Some(config) = config else {
            return Ok(Self::default());
        };

        let mode = match config.mode.as_deref().unwrap_or("auto") {
            "auto" => PackageAssetsMode::Auto,
            "explicit" => PackageAssetsMode::Explicit,
            "disabled" => PackageAssetsMode::Disabled,
            other => bail!(
                "unsupported package_assets.mode {other:?}; use \"auto\", \"explicit\", or \"disabled\""
            ),
        };
        let include = normalize_package_id_set("package_assets.include", config.include)?;
        let exclude = normalize_package_id_set("package_assets.exclude", config.exclude)?;
        if include.iter().any(|id| exclude.contains(id)) {
            bail!(
                "package_assets.include and package_assets.exclude cannot contain the same package id"
            );
        }
        if mode == PackageAssetsMode::Disabled && (!include.is_empty() || !exclude.is_empty()) {
            bail!(
                "package_assets.mode = \"disabled\" cannot be combined with include or exclude lists"
            );
        }

        Ok(Self {
            mode,
            include,
            exclude,
        })
    }

    #[allow(dead_code)]
    fn select(&self, available_ids: impl IntoIterator<Item = String>) -> Result<BTreeSet<String>> {
        let available = available_ids.into_iter().collect::<BTreeSet<_>>();
        for id in &self.include {
            if !available.contains(id) {
                bail!("unknown package asset id in package_assets.include: {id}");
            }
        }
        for id in &self.exclude {
            if !available.contains(id) {
                bail!("unknown package asset id in package_assets.exclude: {id}");
            }
        }

        let mut selected = match self.mode {
            PackageAssetsMode::Auto => available.clone(),
            PackageAssetsMode::Explicit => self.include.clone(),
            PackageAssetsMode::Disabled => BTreeSet::new(),
        };
        for id in &self.exclude {
            selected.remove(id);
        }
        Ok(selected)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct AssetGroupConfig {
    id: String,
    delivery: Option<String>,
    preload: Option<bool>,
    base_url: Option<String>,
    cache: Option<String>,
}

impl AssetGroupConfig {
    fn default_group(id: String, delivery: Option<String>) -> Self {
        Self {
            id,
            delivery,
            preload: None,
            base_url: None,
            cache: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
struct AssetPoliciesConfig {
    #[serde(default)]
    by_extension: BTreeMap<String, ExtensionPolicyConfig>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct ExtensionPolicyConfig {
    asset_group: Option<String>,
    group: Option<String>,
    bundle: Option<String>,
    cache: Option<String>,
    delivery: Option<String>,
}

impl ExtensionPolicyConfig {
    fn group_id(&self) -> Option<&str> {
        self.asset_group
            .as_deref()
            .or(self.group.as_deref())
            .or(self.bundle.as_deref())
    }
}

#[derive(Clone, Debug, Deserialize)]
struct TransformHookConfig {
    id: String,
    #[serde(default)]
    extensions: Vec<String>,
    command: String,
    #[serde(default)]
    args: Vec<String>,
}

fn resolve_manifest_package(
    app: &AppDescriptor,
    asset_package: Option<&AssetPackageTomlConfig>,
) -> Result<AssetToolPackage> {
    if let Some(asset_package) = asset_package {
        validate_package_id(&asset_package.id)?;
        if asset_package.kind.trim().is_empty() {
            bail!("asset_package.kind cannot be empty");
        }
        return Ok(AssetToolPackage {
            id: asset_package.id.clone(),
            name: asset_package.id.clone(),
            kind: asset_package.kind.clone(),
        });
    }

    Ok(AssetToolPackage {
        id: app.slug.clone(),
        name: app.slug.clone(),
        kind: "app".to_string(),
    })
}

fn resolve_package_namespace(
    asset_package: Option<&AssetPackageTomlConfig>,
    package_id: &str,
) -> Result<String> {
    let Some(asset_package) = asset_package else {
        return Ok(String::new());
    };
    let namespace = asset_package
        .namespace
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| format!("packages/{package_id}"));
    normalize_logical_prefix("asset_package.namespace", &namespace)
}

fn normalize_asset_root(
    toml_path: &Path,
    manifest_root: &Path,
    package_namespace: &str,
    is_package_manifest: bool,
    root: AssetRootConfig,
) -> Result<AssetBuildRoot> {
    let (path, mount, local_only) = match root {
        AssetRootConfig::Path(path) => (path, String::new(), true),
        AssetRootConfig::Mounted { path, mount } => {
            let mount = normalize_logical_prefix("assets.roots[].mount", &mount)?;
            (path, mount, is_package_manifest)
        }
    };
    let source_root = resolve_configured_root_path(
        toml_path,
        manifest_root,
        &path,
        is_package_manifest || local_only,
    )?;
    let logical_prefix = join_logical_prefix(package_namespace, &mount)?;
    let diagnostic_root = PathBuf::from(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path.as_str())
        .to_string();
    Ok(AssetBuildRoot {
        source_root,
        diagnostic_root,
        namespace: package_namespace.to_string(),
        mount,
        logical_prefix,
    })
}

fn resolve_configured_root_path(
    toml_path: &Path,
    manifest_root: &Path,
    path: &str,
    require_local: bool,
) -> Result<PathBuf> {
    let resolved = resolve_config_path(manifest_root, path);
    let canonical = fs::canonicalize(&resolved)
        .with_context(|| format!("resolving configured asset root {}", resolved.display()))?;
    if !canonical.is_dir() {
        bail!(
            "{} configured asset root is not a directory: {}",
            toml_path.display(),
            canonical.display()
        );
    }
    if require_local {
        let manifest_root = fs::canonicalize(manifest_root)
            .with_context(|| format!("canonicalizing {}", manifest_root.display()))?;
        if !path_contains(&canonical, &manifest_root) {
            bail!(
                "{} configured asset root {} must remain beneath {}",
                toml_path.display(),
                canonical.display(),
                manifest_root.display()
            );
        }
    }
    Ok(canonical)
}

fn normalize_package_id_set(field: &str, ids: Vec<String>) -> Result<BTreeSet<String>> {
    let mut normalized = BTreeSet::new();
    for id in ids {
        validate_package_id(&id)?;
        if !normalized.insert(id.clone()) {
            bail!("{field} contains duplicate package id {id}");
        }
    }
    Ok(normalized)
}

fn validate_package_id(id: &str) -> Result<()> {
    if id.trim().is_empty() {
        bail!("asset package id cannot be empty");
    }
    if id.starts_with('-') || id.ends_with('-') || id.contains("--") {
        bail!("asset package id must be lowercase kebab-case: {id}");
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        bail!("asset package id must be lowercase kebab-case: {id}");
    }
    Ok(())
}

fn normalize_logical_prefix(field: &str, prefix: &str) -> Result<String> {
    let prefix = prefix.trim().replace('\\', "/");
    if prefix.is_empty() {
        return Ok(String::new());
    }
    validate_logical_path(&prefix).map_err(|reason| anyhow::anyhow!("{field}: {reason}"))?;
    Ok(prefix)
}

fn join_logical_prefix(namespace: &str, mount: &str) -> Result<String> {
    match (namespace.is_empty(), mount.is_empty()) {
        (true, true) => Ok(String::new()),
        (false, true) => Ok(namespace.to_string()),
        (true, false) => Ok(mount.to_string()),
        (false, false) => {
            normalize_logical_prefix("logical asset prefix", &format!("{namespace}/{mount}"))
        }
    }
}

fn resolve_config_path(app_root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        app_root.join(path)
    }
}

fn normalize_extension_policies(
    policies: BTreeMap<String, ExtensionPolicyConfig>,
) -> Result<BTreeMap<String, ExtensionPolicyConfig>> {
    let mut normalized = BTreeMap::new();
    for (extension, policy) in policies {
        normalized.insert(normalize_extension_key(&extension)?, policy);
    }
    Ok(normalized)
}

fn normalize_transform_hooks(
    mut hooks: Vec<TransformHookConfig>,
) -> Result<Vec<TransformHookConfig>> {
    for hook in &mut hooks {
        let mut extensions = Vec::new();
        for extension in hook.extensions.drain(..) {
            extensions.push(normalize_extension_key(&extension)?);
        }
        hook.extensions = extensions;
    }
    Ok(hooks)
}

fn normalize_extension_key(extension: &str) -> Result<String> {
    let extension = extension.trim().to_ascii_lowercase();
    if extension.is_empty() {
        bail!("asset extension policy cannot use an empty extension");
    }
    if extension.starts_with('.') {
        Ok(extension)
    } else {
        Ok(format!(".{extension}"))
    }
}

fn platform_aliases(
    layout: Option<&str>,
    configured: BTreeMap<String, Vec<String>>,
) -> Result<BTreeMap<String, String>> {
    match layout.unwrap_or("folders") {
        "folders" => {
            if configured.is_empty() {
                return Ok(default_platform_aliases());
            }
            let mut aliases = BTreeMap::new();
            for (platform, values) in configured {
                aliases.insert(platform.clone(), platform.clone());
                for value in values {
                    aliases.insert(value, platform.clone());
                }
            }
            Ok(aliases)
        }
        "none" => Ok(BTreeMap::new()),
        other => bail!("unsupported assets.platform_layout {other:?}; use \"folders\" or \"none\""),
    }
}

fn default_platform_aliases() -> BTreeMap<String, String> {
    [
        ("android", "android"),
        ("ios", "ios"),
        ("web", "web"),
        ("wasm", "web"),
        ("windows", "windows"),
        ("win32", "windows"),
        ("macos", "macos"),
        ("darwin", "macos"),
        ("linux", "linux"),
    ]
    .into_iter()
    .map(|(alias, platform)| (alias.to_string(), platform.to_string()))
    .collect()
}

fn dpi_directories(layout: Option<&str>, dpi: Option<AssetDpiConfig>) -> Result<BTreeSet<String>> {
    if let Some(dpi) = dpi
        && !dpi.directories.is_empty()
    {
        return Ok(dpi.directories.into_iter().collect());
    }

    match layout.unwrap_or("flutter") {
        "flutter" => Ok([
            "1x", "1.0x", "1.25x", "1.5x", "2x", "2.0x", "3x", "3.0x", "4x", "4.0x",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()),
        "none" => Ok(BTreeSet::new()),
        other => bail!("unsupported assets.variant_layout {other:?}; use \"flutter\" or \"none\""),
    }
}

fn run_transform_hooks(
    config: &AssetBuildConfig,
    root: &Path,
    path: &Path,
    logical_path: &str,
) -> Result<Vec<String>> {
    if !config.authoring_enabled || config.transforms.is_empty() {
        return Ok(Vec::new());
    }

    let extension = extension_key(logical_path);
    let mut executed = Vec::new();
    for hook in &config.transforms {
        if !hook.extensions.is_empty() && !hook.extensions.iter().any(|item| item == &extension) {
            continue;
        }
        let command = replace_transform_placeholders(&hook.command, root, path, logical_path);
        let command = resolve_transform_command(&config.manifest_root, &command);
        let args = hook
            .args
            .iter()
            .map(|arg| replace_transform_placeholders(arg, root, path, logical_path))
            .collect::<Vec<_>>();
        let mut process = Command::new(&command);
        process.args(&args);
        if !config.manifest_root.as_os_str().is_empty() {
            process.current_dir(&config.manifest_root);
        }
        let status = process
            .status()
            .with_context(|| format!("running asset transform hook {}", hook.id))?;
        if !status.success() {
            bail!(
                "asset transform hook {} failed for {} with status {}",
                hook.id,
                logical_path,
                status
            );
        }
        executed.push(hook.id.clone());
    }
    Ok(executed)
}

fn resolve_transform_command(app_root: &Path, command: &str) -> PathBuf {
    let path = PathBuf::from(command);
    if path.is_absolute() || app_root.as_os_str().is_empty() || !looks_like_command_path(command) {
        path
    } else {
        app_root.join(path)
    }
}

fn looks_like_command_path(command: &str) -> bool {
    command.starts_with('.') || command.contains('/') || command.contains('\\')
}

fn replace_transform_placeholders(
    value: &str,
    root: &Path,
    path: &Path,
    logical_path: &str,
) -> String {
    value
        .replace("{path}", &path.to_string_lossy())
        .replace("{input}", &path.to_string_lossy())
        .replace("{root}", &root.to_string_lossy())
        .replace("{logical_path}", logical_path)
}

fn apply_asset_config(
    logical_path: &str,
    entry: &mut AssetToolEntry,
    config: &AssetBuildConfig,
    transforms: Vec<String>,
) -> Result<()> {
    if !config.authoring_enabled {
        return Ok(());
    }

    let extension = extension_key(logical_path);
    let policy = config.extension_policies.get(&extension);
    let group_id = policy
        .and_then(ExtensionPolicyConfig::group_id)
        .unwrap_or(&config.default_group);
    let group = config.groups.get(group_id).ok_or_else(|| {
        anyhow::anyhow!("asset policy for {logical_path} references unknown asset group {group_id}")
    })?;
    let mut custom = serde_json::Map::new();
    custom.insert(
        "assetGroup".to_string(),
        serde_json::Value::String(group_id.to_string()),
    );
    if let Some(delivery) = policy
        .and_then(|policy| policy.delivery.as_deref())
        .or(group.delivery.as_deref())
    {
        custom.insert(
            "delivery".to_string(),
            serde_json::Value::String(delivery.to_string()),
        );
    }
    if let Some(base_url) = group.base_url.as_deref() {
        custom.insert(
            "baseUrl".to_string(),
            serde_json::Value::String(base_url.to_string()),
        );
    }
    if let Some(cache) = policy
        .and_then(|policy| policy.cache.as_deref())
        .or(group.cache.as_deref())
    {
        custom.insert(
            "cache".to_string(),
            serde_json::Value::String(cache.to_string()),
        );
    }
    if let Some(preload) = group.preload {
        custom.insert("preload".to_string(), serde_json::Value::Bool(preload));
    }
    if let Some(platform) = detect_platform(logical_path, &config.platform_aliases) {
        entry.platform = Some(platform.clone());
        custom.insert("platform".to_string(), serde_json::Value::String(platform));
    }
    if let Some((scale, variant_of)) = detect_dpi_variant(logical_path, &config.dpi_directories) {
        custom.insert("scale".to_string(), serde_json::json!(scale));
        custom.insert(
            "variantOf".to_string(),
            serde_json::Value::String(variant_of),
        );
    }
    if !transforms.is_empty() {
        custom.insert(
            "transforms".to_string(),
            serde_json::Value::Array(
                transforms
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect::<Vec<_>>(),
            ),
        );
    }
    entry.custom = serde_json::Value::Object(custom);
    Ok(())
}

fn detect_platform(
    logical_path: &str,
    platform_aliases: &BTreeMap<String, String>,
) -> Option<String> {
    let first = logical_path.split('/').next()?;
    platform_aliases.get(first).cloned()
}

fn detect_dpi_variant(
    logical_path: &str,
    dpi_directories: &BTreeSet<String>,
) -> Option<(f64, String)> {
    if dpi_directories.is_empty() {
        return None;
    }

    let segments = logical_path.split('/').collect::<Vec<_>>();
    for (index, segment) in segments.iter().enumerate() {
        if !dpi_directories.contains(*segment) {
            continue;
        }
        let Some(scale) = parse_dpi_scale(segment) else {
            continue;
        };
        let variant_of = segments
            .iter()
            .enumerate()
            .filter_map(|(segment_index, value)| (segment_index != index).then_some(*value))
            .collect::<Vec<_>>()
            .join("/");
        if !variant_of.is_empty() && variant_of != logical_path {
            return Some((scale, variant_of));
        }
    }
    None
}

fn parse_dpi_scale(segment: &str) -> Option<f64> {
    let lower = segment.to_ascii_lowercase();
    let value = lower.strip_suffix('x')?;
    let scale = value.parse::<f64>().ok()?;
    (scale > 0.0).then_some(scale)
}

fn extension_key(path: &str) -> String {
    extension(path).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_cross_target_assets_have_identical_common_manifests_and_bytes()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask should be inside the repository root")
            .to_path_buf();
        let app_root = repository_root.join("apps/lab");
        let app = AppDescriptor::resolve(app_root.to_str().expect("UTF-8 app path"))?;
        let root = temp_root("deterministic-cross-target-assets")?;
        let targets = ["host", "wasm", "android", "ios"];
        let mut baseline = None;

        for target in targets {
            let destination = root.join(target).join("app.public");
            package_app_assets(&app, &destination)?;
            let first = asset_tree_snapshot(&destination)?;
            package_app_assets(&app, &destination)?;
            let second = asset_tree_snapshot(&destination)?;
            assert_eq!(
                first, second,
                "{target} staging changed across identical runs"
            );

            for manifest in ["index.json", "index.entries.json"] {
                let text = String::from_utf8(first[manifest].clone())?;
                assert!(
                    !text.contains(root.to_string_lossy().as_ref())
                        && !text.contains(repository_root.to_string_lossy().as_ref()),
                    "{target} {manifest} leaked a physical source path"
                );
            }

            if let Some(expected) = &baseline {
                assert_eq!(
                    expected, &first,
                    "{target} staging differs from the shared host graph"
                );
            } else {
                baseline = Some(first);
            }
        }

        let snapshot = baseline.expect("at least one target snapshot");
        assert!(snapshot.contains_key("icons/bot.svg"));
        let sidecar = String::from_utf8(snapshot["index.entries.json"].clone())?;
        assert!(sidecar.contains("rayx-components-core"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn generated_manifest_writes_compact_index_and_columnar_entries()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-assets-generate")?;
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-assets-generate"),
        )?;
        fs::write(root.join("assets/icons/app.svg"), "<svg/>")?;
        fs::write(root.join("assets/readme.md"), "# Hello")?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let manifest_path = root.join("assets/index.json");
        let manifest = build_manifest(&app, &manifest_path)?;
        write_manifest(&manifest_path, &manifest)?;

        assert_eq!(manifest.assets, vec!["icons/app.svg", "readme.md"]);
        assert_eq!(manifest.entries.len(), 2);
        assert_eq!(manifest.entries[0].kind, "svg");
        assert_eq!(
            manifest.entries[0].mime_type.as_deref(),
            Some("image/svg+xml")
        );
        let index_json = fs::read_to_string(&manifest_path)?;
        let index_value: serde_json::Value = serde_json::from_str(&index_json)?;
        assert!(index_value.get("version").is_none());
        assert!(index_value.get("entries").is_none());
        assert_eq!(index_value["assets"]["icons"][".svg"][0], "app");
        assert_eq!(index_value["assets"][""][".md"][0], "readme");

        let entries_json = fs::read_to_string(entries_manifest_path(&manifest_path))?;
        let entries_value: serde_json::Value = serde_json::from_str(&entries_json)?;
        assert!(entries_value.get("version").is_none());
        assert!(entries_value["columns"].get("path").is_none());
        assert_eq!(
            entries_value["columns"]["sizeBytes"]["v"]
                .as_array()
                .expect("dense byte-size column")
                .len(),
            2
        );
        assert!(entries_value["columns"].get("mimeType").is_none());

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn generated_manifest_sniffs_kind_and_mime_from_file_bytes()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-assets-sniff")?;
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-assets-sniff"),
        )?;
        fs::write(
            root.join("assets/icons/misleading.svg"),
            [137, 80, 78, 71, 13, 10, 26, 10],
        )?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let manifest = build_manifest(&app, &root.join("assets/index.json"))?;

        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].kind, "image");
        assert_eq!(manifest.entries[0].mime_type.as_deref(), Some("image/png"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn generated_manifest_uses_rayx_assets_toml_groups_variants_and_platforms()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-assets-toml")?;
        fs::create_dir_all(root.join("assets/brand/2x"))?;
        fs::create_dir_all(root.join("assets/android/brand"))?;
        fs::create_dir_all(root.join("assets/videos"))?;
        fs::create_dir_all(root.join("content/legal"))?;
        fs::write(root.join("Cargo.toml"), cargo_manifest("xtask-assets-toml"))?;
        fs::write(root.join("assets/brand/logo.png"), png_header())?;
        fs::write(root.join("assets/brand/2x/logo.png"), png_header())?;
        fs::write(root.join("assets/android/brand/logo.webp"), b"RIFF0000WEBP")?;
        fs::write(root.join("assets/videos/intro.mp4"), mp4_header())?;
        fs::write(root.join("content/legal/privacy.md"), "# Privacy")?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[assets]
roots = ["assets", "content"]
default_group = "app"
variant_layout = "flutter"
platform_layout = "folders"

[assets.platforms]
android = ["android"]

[assets.dpi]
directories = ["2x"]

[[asset_group]]
id = "app"
delivery = "bundled"

[[asset_group]]
id = "media"
delivery = "dynamic"
base_url = "https://cdn.example.test/media"
cache = "persistent"

[policies.by_extension]
".mp4" = { asset_group = "media" }
"#,
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let manifest = build_manifest(&app, &root.join("assets/index.json"))?;

        let intro = entry(&manifest, "videos/intro.mp4");
        assert_eq!(intro.custom["assetGroup"], "media");
        assert_eq!(intro.custom["delivery"], "dynamic");
        assert_eq!(intro.custom["baseUrl"], "https://cdn.example.test/media");
        assert_eq!(intro.custom["cache"], "persistent");

        let hdpi = entry(&manifest, "brand/2x/logo.png");
        assert_eq!(hdpi.custom["scale"], 2.0);
        assert_eq!(hdpi.custom["variantOf"], "brand/logo.png");

        let android = entry(&manifest, "android/brand/logo.webp");
        assert_eq!(android.platform.as_deref(), Some("android"));
        assert_eq!(android.custom["platform"], "android");

        assert!(manifest.assets.contains(&"legal/privacy.md".to_string()));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn asset_toml_preserves_multiple_app_string_roots_without_mounts()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-assets-string-roots")?;
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::create_dir_all(root.join("content/legal"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-assets-string-roots"),
        )?;
        fs::write(root.join("assets/icons/app.svg"), "<svg/>")?;
        fs::write(root.join("content/legal/privacy.md"), "# Privacy")?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[assets]
roots = ["assets", "content"]

[[asset_group]]
id = "app"
delivery = "bundled"
"#,
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let manifest = build_manifest(&app, &root.join("assets/index.json"))?;

        assert_eq!(manifest.assets, vec!["icons/app.svg", "legal/privacy.md"]);

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_authoring_deterministically_orders_current_app_entries()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-ordering")?;
        fs::create_dir_all(root.join("assets/zeta"))?;
        fs::create_dir_all(root.join("content/alpha"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-ordering"),
        )?;
        fs::write(root.join("assets/zeta/icon.svg"), "<svg/>")?;
        fs::write(root.join("content/alpha/readme.md"), "# Alpha")?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[assets]
roots = ["assets", "content"]

[[asset_group]]
id = "app"
delivery = "bundled"
"#,
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let manifest = build_manifest(&app, &root.join("assets/index.json"))?;

        assert_eq!(manifest.assets, vec!["alpha/readme.md", "zeta/icon.svg"]);

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    // These target-state tests intentionally fail until package asset authoring exists.
    #[test]
    fn package_asset_authoring_supports_mounted_roots_and_empty_namespace_exports()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-mounted-root")?;
        fs::create_dir_all(root.join("package-icons"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-mounted-root"),
        )?;
        fs::write(root.join("package-icons/bot.svg"), "<svg/>")?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[asset_package]
id = "rayx-components-core"
kind = "component"
namespace = ""

[assets]
roots = [{ path = "package-icons", mount = "icons" }]

[[asset_group]]
id = "app"
delivery = "bundled"
"#,
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let manifest = build_manifest(&app, &root.join("assets/index.json"))?;

        assert_eq!(manifest.assets, vec!["icons/bot.svg"]);

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_authoring_defaults_package_namespace_when_unspecified()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-default-namespace")?;
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-default-namespace"),
        )?;
        fs::write(root.join("assets/icons/bot.svg"), "<svg/>")?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[asset_package]
id = "rayx-components-core"
kind = "component"

[assets]
roots = [{ path = "assets", mount = "icons" }]

[[asset_group]]
id = "app"
delivery = "bundled"
"#,
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let manifest = build_manifest(&app, &root.join("assets/index.json"))?;

        assert_eq!(
            manifest.assets,
            vec!["packages/rayx-components-core/icons/icons/bot.svg"]
        );

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_authoring_supports_mounted_external_app_roots()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-external-app-root")?;
        let external = root
            .parent()
            .expect("temp root should have a parent")
            .join("shared-external-assets");
        fs::create_dir_all(&external)?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-external-app-root"),
        )?;
        fs::write(external.join("bot.svg"), "<svg/>")?;
        fs::write(
            root.join("rayx.assets.toml"),
            format!(
                r#"
version = 1

[assets]
roots = [{{ path = "{}", mount = "icons" }}]

[[asset_group]]
id = "app"
delivery = "bundled"
"#,
                external.display().to_string().replace('\\', "\\\\")
            ),
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let manifest = build_manifest(&app, &root.join("assets/index.json"))?;

        assert_eq!(manifest.assets, vec!["icons/bot.svg"]);

        fs::remove_dir_all(root).ok();
        fs::remove_dir_all(external).ok();
        Ok(())
    }

    #[test]
    fn package_asset_authoring_selects_package_assets_by_mode_and_filters() -> Result<()> {
        let available = [
            "rayx-components-core".to_string(),
            "rayx-icons".to_string(),
            "rayx-fixtures".to_string(),
        ];

        let auto = PackageAssetsSelection::resolve(None)?;
        assert_eq!(
            auto.select(available.clone())?,
            available.iter().cloned().collect::<BTreeSet<_>>()
        );

        let explicit = PackageAssetsSelection::resolve(Some(PackageAssetsTomlConfig {
            mode: Some("explicit".to_string()),
            include: vec!["rayx-components-core".to_string(), "rayx-icons".to_string()],
            exclude: vec!["rayx-fixtures".to_string()],
        }))?;
        assert_eq!(
            explicit.select(available.clone())?,
            ["rayx-components-core".to_string(), "rayx-icons".to_string()]
                .into_iter()
                .collect::<BTreeSet<_>>()
        );

        let disabled = PackageAssetsSelection::resolve(Some(PackageAssetsTomlConfig {
            mode: Some("disabled".to_string()),
            include: Vec::new(),
            exclude: Vec::new(),
        }))?;
        assert!(disabled.select(available)?.is_empty());

        Ok(())
    }

    #[test]
    fn package_asset_authoring_rejects_unknown_package_asset_ids() {
        let selection = PackageAssetsSelection::resolve(Some(PackageAssetsTomlConfig {
            mode: Some("explicit".to_string()),
            include: vec![
                "rayx-components-core".to_string(),
                "missing-package".to_string(),
            ],
            exclude: Vec::new(),
        }))
        .expect("selection should parse");
        let error = selection
            .select(["rayx-components-core".to_string()])
            .expect_err("unknown explicit package ids must fail");

        assert!(
            error
                .to_string()
                .contains("unknown package asset id in package_assets.include")
        );
    }

    #[test]
    fn package_asset_authoring_rejects_package_roots_outside_manifest_directory()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-root-escape")?;
        let escaped_root = root
            .parent()
            .expect("temp root should have a parent")
            .join("shared");
        fs::create_dir_all(&escaped_root)?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-root-escape"),
        )?;
        fs::write(escaped_root.join("secret.svg"), "<svg/>")?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[asset_package]
id = "rayx-components-core"
kind = "component"

[assets]
roots = ["../shared"]

[[asset_group]]
id = "app"
delivery = "bundled"
"#,
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let error = build_manifest(&app, &root.join("assets/index.json"))
            .expect_err("package-authored roots must stay inside the manifest directory");

        assert!(error.to_string().contains("rayx.assets.toml"));

        fs::remove_dir_all(root).ok();
        fs::remove_dir_all(escaped_root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_authoring_reports_strict_collision_diagnostics_before_staging()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-collision")?;
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::create_dir_all(root.join("theme-assets/icons"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-collision"),
        )?;
        fs::write(root.join("assets/icons/bot.svg"), "<svg/>")?;
        fs::write(
            root.join("theme-assets/icons/bot.svg"),
            "<svg class=\"theme\"/>",
        )?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[assets]
roots = ["assets", "theme-assets"]

[[asset_group]]
id = "app"
delivery = "bundled"
"#,
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let error = build_manifest(&app, &root.join("assets/index.json"))
            .expect_err("duplicate logical paths should fail before staging");

        let message = error.to_string();
        assert!(message.contains("icons/bot.svg"));
        assert!(message.contains("assets/icons/bot.svg"));
        assert!(message.contains("theme-assets/icons/bot.svg"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_collision_preserves_existing_destination_on_preflight_failure()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-collision-destination")?;
        let destination = root.join("staged-assets");
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::create_dir_all(root.join("theme-assets/icons"))?;
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("sentinel.txt"), "keep me")?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-collision-destination"),
        )?;
        fs::write(root.join("assets/icons/bot.svg"), "<svg/>")?;
        fs::write(
            root.join("theme-assets/icons/bot.svg"),
            "<svg class=\"theme\"/>",
        )?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[assets]
roots = ["assets", "theme-assets"]

[[asset_group]]
id = "app"
delivery = "bundled"
"#,
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        package_app_assets(&app, &destination)
            .expect_err("collisions must fail before destination mutation");

        assert_eq!(
            fs::read_to_string(destination.join("sentinel.txt"))?,
            "keep me"
        );

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_path_safety_rejects_roots_outside_the_manifest_directory()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-safety-source")?;
        let escaped_root = root
            .parent()
            .expect("temp root should have a parent")
            .join("external");
        fs::create_dir_all(&escaped_root)?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-safety-source"),
        )?;
        fs::write(escaped_root.join("secret.svg"), "<svg/>")?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[assets]
roots = ["../external"]

[[asset_group]]
id = "app"
delivery = "bundled"
"#,
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        build_manifest(&app, &root.join("assets/index.json"))
            .expect_err("unsafe source roots should fail before enumeration");

        fs::remove_dir_all(root).ok();
        fs::remove_dir_all(escaped_root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_path_safety_rejects_staging_destination_that_overlaps_source_roots()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-safety-destination")?;
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-safety-destination"),
        )?;
        fs::write(root.join("assets/icons/app.svg"), "<svg/>")?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;

        package_app_assets(&app, &root.join("assets"))
            .expect_err("staging must reject destinations that overlap source roots");

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn rayx_assets_toml_transform_hooks_execute_for_matching_extensions()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-assets-transform")?;
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-assets-transform"),
        )?;
        fs::write(root.join("assets/icons/app.svg"), "<svg/>")?;
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
        fs::write(
            root.join("rayx.assets.toml"),
            format!(
                r#"
version = 1

[assets]
roots = ["assets"]

[[asset_group]]
id = "app"
delivery = "bundled"

[[transform]]
id = "svg.version"
extensions = [".svg"]
command = "{}"
args = ["--version"]
"#,
                rustc.replace('\\', "\\\\")
            ),
        )?;

        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let manifest = build_manifest(&app, &root.join("assets/index.json"))?;

        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].custom["transforms"][0], "svg.version");

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_app_assets_preserves_rayx_assets_toml_metadata_in_staged_manifest()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-assets-package")?;
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-assets-package"),
        )?;
        fs::write(root.join("assets/icons/app.svg"), "<svg/>")?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[assets]
roots = ["assets"]
default_group = "app"

[[asset_group]]
id = "app"
delivery = "bundled"

[[asset_group]]
id = "startup"
delivery = "bundled"
preload = true

[policies.by_extension]
".svg" = { asset_group = "startup" }
"#,
        )?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let destination = root.join("staged-assets");

        package_app_assets(&app, &destination)?;
        let manifest = read_manifest(&destination.join("index.json"))?;
        let icon = entry(&manifest, "icons/app.svg");

        assert_eq!(icon.custom["assetGroup"], "startup");
        assert_eq!(icon.custom["preload"], true);
        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn package_asset_graph_validation_rejects_a_manifest_missing_selected_entries()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-package-assets-graph-validation")?;
        fs::create_dir_all(root.join("assets/icons"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-package-assets-graph-validation"),
        )?;
        fs::write(root.join("assets/icons/app.svg"), "<svg/>")?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let graph = AssetBuildGraph::for_app(&app, &CargoMetadataContext::default())?;
        let manifest_path = root.join("assets/index.json");
        let expected = graph.collect_entries(&manifest_path)?;
        let empty_entries = BTreeMap::new();
        write_manifest(
            &manifest_path,
            &manifest_from_entries(&graph.app.package, &empty_entries),
        )?;

        let report = validate_manifest_against_graph(
            graph.selected_validation_roots(),
            &manifest_path,
            &expected,
        )?;

        assert!(!report.ok);
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "asset.manifest.missing-graph-entry"
                && diagnostic.path == "icons/app.svg"
        }));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn mixed_package_manifest_preserves_graph_and_list_ownership()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-mixed-package-manifest")?;
        let public = root.join("public");
        fs::create_dir_all(public.join("icons"))?;
        fs::write(public.join("icons/app.svg"), "<svg id=\"app\"/>")?;
        fs::write(public.join("icons/theme.svg"), "<svg id=\"theme\"/>")?;
        let entries = mixed_package_entries(&root, &public);
        let manifest_path = public.join("index.json");
        let manifest = manifest_from_entries(&test_app_package(), &entries);

        write_manifest(&manifest_path, &manifest)?;

        let sidecar: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(entries_manifest_path(&manifest_path))?)?;
        assert_eq!(sidecar["package"][0], "test-app");
        assert_eq!(
            sidecar["dict"]["packageId"],
            serde_json::json!(["test-theme"])
        );
        assert_eq!(
            sidecar["columns"]["packageId"]["o"],
            serde_json::json!([[1, 0]])
        );

        let reparsed = read_manifest(&manifest_path)?;
        let reparsed_owners = reparsed
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.package_id.as_deref()))
            .collect::<BTreeMap<_, _>>();
        let list_owners = entries
            .iter()
            .map(|(path, entry)| (path.as_str(), Some(entry.package_id.as_str())))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(reparsed_owners, list_owners);

        let report = validate_manifest_against_graph(
            vec![AssetValidationRoot::direct(public.clone())],
            &manifest_path,
            &entries,
        )?;
        assert!(report.ok, "mixed manifest should validate: {report:?}");

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn deterministic_package_manifest_is_byte_identical_across_generations()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-deterministic-package-manifest")?;
        let public = root.join("public");
        fs::create_dir_all(public.join("icons"))?;
        fs::write(public.join("icons/app.svg"), "<svg id=\"app\"/>")?;
        fs::write(public.join("icons/theme.svg"), "<svg id=\"theme\"/>")?;
        let entries = mixed_package_entries(&root, &public);
        let manifest_path = public.join("index.json");
        let manifest = manifest_from_entries(&test_app_package(), &entries);

        write_manifest(&manifest_path, &manifest)?;
        let first_index = fs::read(&manifest_path)?;
        let first_entries = fs::read(entries_manifest_path(&manifest_path))?;
        write_manifest(&manifest_path, &manifest)?;

        assert_eq!(fs::read(&manifest_path)?, first_index);
        assert_eq!(
            fs::read(entries_manifest_path(&manifest_path))?,
            first_entries
        );

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn atomic_asset_staging_preserves_verified_destination_and_removes_stale_files()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-atomic-asset-staging")?;
        let destination = root.join("assets");
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("verified.txt"), "verified")?;

        let error = stage_directory_atomically(&destination, |staging| {
            fs::write(staging.join("unverified.txt"), "unverified")?;
            bail!("simulated validation failure")
        })
        .expect_err("failed staging must not activate");
        assert!(error.to_string().contains("simulated validation failure"));
        assert_eq!(
            fs::read_to_string(destination.join("verified.txt"))?,
            "verified"
        );
        assert!(!destination.join("unverified.txt").exists());

        stage_directory_atomically(&destination, |staging| {
            fs::write(staging.join("current.txt"), "current")?;
            Ok(())
        })?;
        assert!(!destination.join("verified.txt").exists());
        assert_eq!(
            fs::read_to_string(destination.join("current.txt"))?,
            "current"
        );

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn validation_reports_missing_manifest_files()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-assets-validate")?;
        fs::create_dir_all(root.join("assets"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-assets-validate"),
        )?;
        fs::write(
            root.join("assets/index.json"),
            r#"{"assets":{"":{".svg":["missing"]}}}"#,
        )?;
        fs::write(
            root.join("assets/index.entries.json"),
            r#"{"package":["test","test","app"],"dict":{},"columns":{}}"#,
        )?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let report = validate_manifest(app.asset_roots()?, &root.join("assets/index.json"))?;

        assert!(!report.ok);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].code, "asset.manifest.missing-file");

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn validation_reports_invalid_and_duplicate_manifest_paths()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-assets-invalid")?;
        fs::create_dir_all(root.join("assets"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-assets-invalid"),
        )?;
        fs::write(root.join("assets/app.svg"), "<svg/>")?;
        fs::write(
            root.join("assets/index.json"),
            r#"{"assets":{"":{".svg":["app","app"]},"..":{".svg":["secret"]}}}"#,
        )?;
        fs::write(
            root.join("assets/index.entries.json"),
            r#"{"package":["test","test","app"],"dict":{},"columns":{}}"#,
        )?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let report = validate_manifest(app.asset_roots()?, &root.join("assets/index.json"))?;
        let codes = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();

        assert!(codes.contains(&"asset.manifest.duplicate-asset"));
        assert!(codes.contains(&"asset.manifest.invalid-path"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn validation_rejects_misaligned_entries_sidecar()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-assets-mismatch")?;
        fs::create_dir_all(root.join("assets"))?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-assets-mismatch"),
        )?;
        fs::write(root.join("assets/app.svg"), "<svg/>")?;
        fs::write(
            root.join("assets/index.json"),
            r#"{"assets":{"":{".svg":["app"]}}}"#,
        )?;
        fs::write(
            root.join("assets/index.entries.json"),
            r#"{"package":["test","test","app"],"dict":{},"columns":{"sizeBytes":{"v":[1,2]}}}"#,
        )?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let error = validate_manifest(app.asset_roots()?, &root.join("assets/index.json"))
            .expect_err("misaligned sidecar should fail validation");
        assert!(error.to_string().contains("align with index asset count"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn app_content_copies_declared_bytes_without_cataloging_them()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-app-content")?;
        fs::create_dir_all(root.join("assets"))?;
        fs::write(root.join("Cargo.toml"), cargo_manifest("xtask-app-content"))?;
        fs::write(root.join("fixture.rs"), "pub fn fixture() {}")?;
        fs::write(root.join("assets/icon.svg"), "<svg/>")?;
        fs::write(root.join("app_settings.json"), br#"{"enabled":true}"#)?;
        fs::write(
            root.join("rayx.assets.toml"),
            r#"
version = 1

[assets]
roots = ["assets"]

[app_content]
files = ["app_settings.json"]
"#,
        )?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let deployment = root.join("deployment");

        package_app_assets(&app, &deployment.join("assets"))?;
        package_app_content(&app, &deployment)?;

        assert_eq!(
            fs::read(deployment.join("app_settings.json"))?,
            fs::read(root.join("app_settings.json"))?
        );
        assert!(!deployment.join("assets/app_settings.json").exists());
        let index = fs::read_to_string(deployment.join("assets/index.json"))?;
        let entries = fs::read_to_string(deployment.join("assets/index.entries.json"))?;
        assert!(!index.contains("app_settings"));
        assert!(!entries.contains("app_settings"));

        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn app_content_root_metadata_sources_content_from_another_directory()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-app-content-root")?;
        let shared = root.join("shared");
        let app = root.join("app");
        fs::create_dir_all(&shared)?;
        fs::create_dir_all(&app)?;
        fs::write(shared.join("app_settings.json"), br#"{"shared":true}"#)?;
        fs::write(
            app.join("Cargo.toml"),
            format!(
                "{}
[package.metadata.rayx]
app_content_root = \"../shared\"
",
                cargo_manifest("xtask-app-content-root")
            ),
        )?;
        fs::write(app.join("fixture.rs"), "pub fn fixture() {}")?;
        fs::write(
            app.join("rayx.assets.toml"),
            "version = 1
[app_content]
files = [\"app_settings.json\"]
",
        )?;
        let descriptor = AppDescriptor::resolve(app.to_str().expect("utf-8 path"))?;
        let deployment = root.join("out");
        package_app_content(&descriptor, &deployment)?;
        assert_eq!(
            fs::read(deployment.join("app_settings.json"))?,
            br#"{"shared":true}"#
        );
        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[test]
    fn app_content_rejects_unsafe_missing_duplicate_and_catalog_paths()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root = temp_root("xtask-app-content-invalid")?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-app-content-invalid"),
        )?;
        fs::write(root.join("fixture.rs"), "pub fn fixture() {}")?;
        fs::write(root.join("settings.json"), "{}")?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        for (files, expected) in [
            (r#"["../settings.json"]"#, "invalid app_content"),
            (r#"["C:/settings.json"]"#, "invalid app_content"),
            (r#"["missing.json"]"#, "missing"),
            (
                r#"["settings.json", "settings.json"]"#,
                "duplicate or colliding",
            ),
            (r#"["assets/settings.json"]"#, "AssetService deployment"),
            (r#"["index.json"]"#, "AssetService deployment"),
        ] {
            fs::write(
                root.join("rayx.assets.toml"),
                format!("version = 1\n[app_content]\nfiles = {files}\n"),
            )?;
            let error = package_app_content(&app, &root.join("out"))
                .expect_err("unsafe app content declaration must fail");
            assert!(
                error.to_string().contains(expected),
                "expected {expected:?} in {error}"
            );
        }
        fs::remove_dir_all(root).ok();
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn app_content_rejects_symlinked_files() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        use std::os::unix::fs::symlink;

        let root = temp_root("xtask-app-content-symlink")?;
        fs::write(
            root.join("Cargo.toml"),
            cargo_manifest("xtask-app-content-symlink"),
        )?;
        fs::write(root.join("fixture.rs"), "pub fn fixture() {}")?;
        fs::write(root.join("actual.json"), "{}")?;
        symlink(root.join("actual.json"), root.join("linked.json"))?;
        fs::write(
            root.join("rayx.assets.toml"),
            "version = 1\n[app_content]\nfiles = [\"linked.json\"]\n",
        )?;
        let app = AppDescriptor::resolve(root.to_str().expect("utf-8 path"))?;
        let error =
            package_app_content(&app, &root.join("out")).expect_err("symlinked content must fail");
        assert!(error.to_string().contains("symlink"));
        fs::remove_dir_all(root).ok();
        Ok(())
    }

    fn cargo_manifest(name: &str) -> String {
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[lib]
path = "fixture.rs"
"#
        )
    }

    fn entry<'a>(manifest: &'a AssetToolManifest, path: &str) -> &'a AssetToolEntry {
        manifest
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .unwrap_or_else(|| panic!("missing manifest entry {path}"))
    }

    fn test_app_package() -> AssetToolPackage {
        AssetToolPackage {
            id: "test-app".to_string(),
            name: "Test App".to_string(),
            kind: "app".to_string(),
        }
    }

    fn mixed_package_entries(root: &Path, public: &Path) -> BTreeMap<String, CollectedAssetEntry> {
        [
            ("icons/app.svg", "test-app", "app"),
            ("icons/theme.svg", "test-theme", "theme"),
        ]
        .into_iter()
        .map(|(path, package_id, manifest_name)| {
            let mut manifest_entry = default_tool_entry(path);
            manifest_entry.package_id = Some(package_id.to_string());
            (
                path.to_string(),
                CollectedAssetEntry {
                    manifest_entry,
                    package_id: package_id.to_string(),
                    asset_manifest: root.join(format!("{manifest_name}.rayx.assets.toml")),
                    namespace: String::new(),
                    mount: "icons".to_string(),
                    source_root: public.to_path_buf(),
                    source_path: public.join(path),
                    display_path: path.to_string(),
                },
            )
        })
        .collect()
    }

    fn png_header() -> [u8; 8] {
        [137, 80, 78, 71, 13, 10, 26, 10]
    }

    fn mp4_header() -> [u8; 12] {
        [0, 0, 0, 0, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm']
    }

    fn temp_root(name: &str) -> std::io::Result<PathBuf> {
        let root = std::env::temp_dir().join(format!(
            "rayx-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;
        Ok(root)
    }

    fn asset_tree_snapshot(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
        fn visit(
            root: &Path,
            directory: &Path,
            snapshot: &mut BTreeMap<String, Vec<u8>>,
        ) -> Result<()> {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    visit(root, &path, snapshot)?;
                } else {
                    let logical_path = path
                        .strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/");
                    snapshot.insert(logical_path, fs::read(path)?);
                }
            }
            Ok(())
        }

        let mut snapshot = BTreeMap::new();
        visit(root, root, &mut snapshot)?;
        Ok(snapshot)
    }
}
