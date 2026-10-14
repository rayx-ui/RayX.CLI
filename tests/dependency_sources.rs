//! Dependency sources: anyone who clones the public repository can build it.
//!
//! Allowed: crates.io, public `https://` git repositories pinned to a `rev`, and a `[patch]` entry
//! that follows the same source rules and has a comment above it linking the upstream issue or pull
//! request. Forbidden: `path` dependencies, git dependencies on a `branch`, a `tag` or nothing,
//! anything from the private `rayx-ui/RayX` repository, and `[replace]`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The only registry a clean clone can always reach.
const CRATES_IO_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The private repository nothing may depend on (exactly this one; `RayX.CLI`, `Gpux` and other
/// public repositories of the organization are fine).
const PRIVATE_REPOSITORY: &str = "rayx-ui/rayx";

/// Why `url` (a git URL or a `git+` source) names a repository nobody may depend on, if it does.
fn forbidden_git_url(url: &str) -> Option<String> {
    let url = url.strip_prefix("git+").unwrap_or(url);
    let Some(rest) = url.strip_prefix("https://") else {
        return Some(format!("git url {url} must use https://"));
    };
    let location = rest.split(['?', '#']).next().unwrap_or(rest);
    let location = location.trim_end_matches('/');
    let location = location.strip_suffix(".git").unwrap_or(location);
    let mut parts = location.splitn(2, '/');
    let _host = parts.next();
    let repository = parts.next().unwrap_or("").to_ascii_lowercase();
    (repository == PRIVATE_REPOSITORY)
        .then(|| format!("git url {url} is the private rayx-ui/RayX repository"))
}

/// Violations of the source rules in one dependency declaration.
fn dependency_violations(context: &str, name: &str, spec: &toml::Value, out: &mut Vec<String>) {
    let Some(spec) = spec.as_table() else {
        return; // `name = "1.2"` is a crates.io version requirement
    };
    if spec.contains_key("path") {
        out.push(format!("{context}: {name} is a path dependency"));
    }
    for registry in ["registry", "registry-index"] {
        if spec.contains_key(registry) {
            out.push(format!(
                "{context}: {name} uses an alternative `{registry}`"
            ));
        }
    }
    let Some(git) = spec.get("git") else {
        return;
    };
    let url = git.as_str().unwrap_or("");
    if let Some(reason) = forbidden_git_url(url) {
        out.push(format!("{context}: {name}: {reason}"));
    }
    for moving in ["branch", "tag"] {
        if spec.contains_key(moving) {
            out.push(format!(
                "{context}: {name} is pinned by `{moving}`, not `rev`"
            ));
        }
    }
    match spec.get("rev").map(|rev| rev.as_str().unwrap_or("")) {
        None => out.push(format!(
            "{context}: {name} is a git dependency without a `rev`"
        )),
        Some(rev) if !is_commit(rev) => out.push(format!(
            "{context}: {name} has `rev = {rev:?}`, which is not a commit hash"
        )),
        Some(_) => {}
    }
}

/// Whether `rev` is an (abbreviated) commit hash, so it cannot move like a branch or tag name.
fn is_commit(rev: &str) -> bool {
    (7..=40).contains(&rev.len()) && rev.chars().all(|c| c.is_ascii_hexdigit())
}

/// Walks the manifest for every `*dependencies` table, including `[target.<cfg>.*]` and
/// `[workspace.dependencies]`.
fn walk_dependencies(path: &str, value: &toml::Value, out: &mut Vec<String>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        let child_path = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        if key.ends_with("dependencies") {
            for (name, spec) in child.as_table().into_iter().flatten() {
                dependency_violations(&child_path, name, spec, out);
            }
        } else if key != "patch" {
            walk_dependencies(&child_path, child, out);
        }
    }
}

/// Whether the lines directly above `line` are comments and one of them links an upstream issue or
/// pull request.
fn has_upstream_comment(lines: &[&str], line: usize) -> bool {
    let mut at = line;
    let mut linked = false;
    while at > 0 {
        let above = lines[at - 1].trim();
        if !above.starts_with('#') {
            break;
        }
        linked |= (above.contains("https://") || above.contains("http://"))
            && (above.contains("/issues/") || above.contains("/pull/"));
        at -= 1;
    }
    linked
}

/// The line holding the declaration of patched crate `name` in `[patch.<registry>]`.
fn patch_line(lines: &[&str], registry: &str, name: &str) -> Option<usize> {
    let section = format!("[patch.{registry}.{name}]");
    let quoted_section = format!("[patch.\"{registry}\".{name}]");
    let mut in_registry = false;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == section || trimmed == quoted_section {
            return Some(index);
        }
        if trimmed.starts_with('[') {
            in_registry = trimmed == format!("[patch.{registry}]")
                || trimmed == format!("[patch.\"{registry}\"]");
        } else if in_registry {
            let key = trimmed.split(['=', '.']).next().unwrap_or("").trim();
            if key == name {
                return Some(index);
            }
        }
    }
    None
}

/// Every way `manifest` (the text of a `Cargo.toml`) breaks the dependency source rules.
fn manifest_violations(manifest: &str) -> Vec<String> {
    let parsed: toml::Table = manifest.parse().expect("manifest parses");
    let mut out = Vec::new();
    walk_dependencies("", &toml::Value::Table(parsed.clone()), &mut out);

    if parsed.contains_key("replace") {
        out.push("[replace] is not allowed; use [patch] with an upstream link".to_string());
    }
    if let Some(patch) = parsed.get("patch").and_then(toml::Value::as_table) {
        let lines: Vec<&str> = manifest.lines().collect();
        for (registry, crates) in patch {
            for (name, spec) in crates.as_table().into_iter().flatten() {
                let context = format!("patch.{registry}");
                dependency_violations(&context, name, spec, &mut out);
                match patch_line(&lines, registry, name) {
                    Some(line) if has_upstream_comment(&lines, line) => {}
                    _ => out.push(format!(
                        "{context}: {name} needs a comment directly above it linking the \
                         upstream issue or pull request"
                    )),
                }
            }
        }
    }
    out
}

/// Violations in a `cargo metadata` / `Cargo.lock` source string (`registry+...`, `git+https://...`).
fn source_violation(name: &str, source: &str) -> Option<String> {
    if source == CRATES_IO_SOURCE {
        return None;
    }
    if source.starts_with("registry+") || source.starts_with("sparse+") {
        return Some(format!("{name}: {source} is not crates.io"));
    }
    if source.starts_with("git+") {
        if let Some(reason) = forbidden_git_url(source) {
            return Some(format!("{name}: {reason}"));
        }
        let rev = source
            .split_once("?rev=")
            .map(|(_, rest)| rest.split('#').next().unwrap_or(rest));
        return match rev {
            Some(rev) if is_commit(rev) => None,
            _ => Some(format!(
                "{name}: git source {source} is not pinned by a commit `rev`"
            )),
        };
    }
    Some(format!("{name}: unsupported source {source:?}"))
}

fn assert_accepted(manifest: &str) {
    let violations = manifest_violations(manifest);
    assert!(violations.is_empty(), "expected acceptance: {violations:?}");
}

fn assert_rejected(manifest: &str, needle: &str) {
    let violations = manifest_violations(manifest);
    assert!(
        violations.iter().any(|v| v.contains(needle)),
        "expected a violation containing {needle:?}, got {violations:?}"
    );
}

const HEADER: &str = "[package]\nname = \"x\"\nversion = \"1.0.0\"\n\n";

#[test]
fn crates_io_dependencies_are_accepted() {
    assert_accepted(&format!(
        "{HEADER}[dependencies]\nanyhow = \"1\"\nserde = {{ version = \"1\", features = [\"derive\"] }}\n\
         [dev-dependencies]\ntempfile = \"3\"\n\
         [target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n"
    ));
}

#[test]
fn public_https_git_dependencies_pinned_by_rev_are_accepted() {
    assert_accepted(&format!(
        "{HEADER}[dependencies]\n\
         gpux = {{ git = \"https://github.com/rayx-ui/Gpux\", rev = \"0123456789abcdef\" }}\n\
         cli = {{ git = \"https://github.com/rayx-ui/RayX.CLI.git\", rev = \"abcdef0\" }}\n\
         [target.'cfg(windows)'.dependencies]\n\
         other = {{ git = \"https://github.com/someone/else\", rev = \"abcdef0\" }}\n"
    ));
}

#[test]
fn path_dependencies_are_rejected() {
    assert_rejected(
        &format!("{HEADER}[dependencies]\ngpux = {{ path = \"../Gpux\" }}\n"),
        "path dependency",
    );
    assert_rejected(
        &format!(
            "{HEADER}[dev-dependencies]\nlocal = {{ path = \"crates/local\", version = \"1\" }}\n"
        ),
        "path dependency",
    );
}

#[test]
fn git_dependencies_must_be_pinned_by_rev() {
    for (pin, needle) in [
        ("branch = \"main\"", "`branch`"),
        ("tag = \"v1.0.0\"", "`tag`"),
        ("", "without a `rev`"),
    ] {
        assert_rejected(
            &format!(
                "{HEADER}[dependencies]\ngpux = {{ git = \"https://github.com/rayx-ui/Gpux\", {pin} }}\n"
            ),
            needle,
        );
    }
}

#[test]
fn git_rev_must_be_a_commit_hash() {
    for rev in ["main", "refs/heads/main", "v1.0.0", "abc", ""] {
        assert_rejected(
            &format!(
                "{HEADER}[dependencies]\ngpux = {{ git = \"https://github.com/rayx-ui/Gpux\", rev = \"{rev}\" }}\n"
            ),
            "not a commit hash",
        );
    }
}

#[test]
fn alternative_registries_are_rejected() {
    assert_rejected(
        &format!("{HEADER}[dependencies]\ninternal = {{ version = \"1\", registry = \"corp\" }}\n"),
        "alternative `registry`",
    );
    assert_rejected(
        &format!(
            "{HEADER}[dependencies]\ninternal = {{ version = \"1\", registry-index = \"https://corp.example/index\" }}\n"
        ),
        "alternative `registry-index`",
    );
    assert_rejected(
        &format!(
            "{HEADER}[dependencies]\nanyhow = \"1\"\n\n[patch.crates-io]\n# https://github.com/dtolnay/anyhow/pull/1\nanyhow = {{ version = \"1\", registry = \"corp\" }}\n"
        ),
        "alternative `registry`",
    );
}

#[test]
fn git_dependencies_must_use_https() {
    assert_rejected(
        &format!(
            "{HEADER}[dependencies]\ngpux = {{ git = \"git@github.com:rayx-ui/Gpux.git\", rev = \"abcdef0\" }}\n"
        ),
        "must use https://",
    );
    assert_rejected(
        &format!(
            "{HEADER}[dependencies]\ngpux = {{ git = \"ssh://git@github.com/rayx-ui/Gpux.git\", rev = \"abcdef0\" }}\n"
        ),
        "must use https://",
    );
}

#[test]
fn the_private_rayx_repository_is_rejected_but_similar_names_are_not() {
    for url in [
        "https://github.com/rayx-ui/RayX",
        "https://github.com/rayx-ui/RayX.git",
        "https://github.com/rayx-ui/RayX/",
        "https://github.com/RayX-UI/rayx",
    ] {
        assert_rejected(
            &format!("{HEADER}[dependencies]\nrayx = {{ git = \"{url}\", rev = \"abcdef0\" }}\n"),
            "private rayx-ui/RayX repository",
        );
    }
    for url in [
        "https://github.com/rayx-ui/RayX.CLI",
        "https://github.com/rayx-ui/RayXtra",
        "https://github.com/other/RayX",
    ] {
        assert_accepted(&format!(
            "{HEADER}[dependencies]\nx = {{ git = \"{url}\", rev = \"abcdef0\" }}\n"
        ));
    }
}

#[test]
fn patch_entries_need_the_source_rules_and_an_upstream_link() {
    let patch = |comment: &str, spec: &str| {
        format!(
            "{HEADER}[dependencies]\nanyhow = \"1\"\n\n[patch.crates-io]\n{comment}anyhow = {spec}\n"
        )
    };
    let pinned = "{ git = \"https://github.com/dtolnay/anyhow\", rev = \"abcdef0\" }";

    assert_accepted(&patch(
        "# Waiting for https://github.com/dtolnay/anyhow/pull/1\n",
        pinned,
    ));
    assert_accepted(&patch(
        "# Fix for the TLS backend.\n# Upstream: https://github.com/dtolnay/anyhow/issues/2\n",
        pinned,
    ));
    assert_rejected(&patch("", pinned), "needs a comment directly above it");
    assert_rejected(
        &patch("# a comment without a link\n", pinned),
        "needs a comment directly above it",
    );
    assert_rejected(
        &patch("# https://example.com/not-an-issue\n", pinned),
        "needs a comment directly above it",
    );
    assert_rejected(
        &patch(
            "# https://github.com/dtolnay/anyhow/pull/1\n",
            "{ path = \"../anyhow\" }",
        ),
        "path dependency",
    );
    assert_rejected(
        &patch(
            "# https://github.com/dtolnay/anyhow/pull/1\n",
            "{ git = \"https://github.com/dtolnay/anyhow\", branch = \"main\" }",
        ),
        "`branch`",
    );
}

#[test]
fn replace_is_rejected() {
    assert_rejected(
        &format!(
            "{HEADER}[dependencies]\nanyhow = \"1\"\n\n[replace]\n\"anyhow:1.0.0\" = {{ git = \"https://github.com/dtolnay/anyhow\", rev = \"abcdef0\" }}\n"
        ),
        "[replace]",
    );
}

#[test]
fn resolved_sources_are_registry_or_pinned_public_git() {
    assert_eq!(source_violation("a", CRATES_IO_SOURCE), None);
    for source in [
        "registry+https://corp.example/index",
        "sparse+https://corp.example/index/",
    ] {
        assert!(
            source_violation("r", source).is_some(),
            "{source} should be rejected"
        );
    }
    assert_eq!(
        source_violation(
            "g",
            "git+https://github.com/rayx-ui/Gpux?rev=abcdef0#0123456789abcdef0123456789abcdef01234567"
        ),
        None
    );
    for source in [
        "git+https://github.com/rayx-ui/RayX?rev=abcdef0#0123456789abcdef0123456789abcdef01234567",
        "git+https://github.com/rayx-ui/Gpux?branch=main#0123",
        "git+https://github.com/rayx-ui/Gpux?tag=v1#0123",
        "git+https://github.com/rayx-ui/Gpux?rev=main#0123456789abcdef0123456789abcdef01234567",
        "git+https://github.com/rayx-ui/Gpux#0123",
        "git+ssh://git@github.com/rayx-ui/Gpux?rev=abcdef0#0123456789abcdef0123456789abcdef01234567",
        "path+file:///home/x/Gpux",
    ] {
        assert!(
            source_violation("d", source).is_some(),
            "{source} should be rejected"
        );
    }
}

#[test]
fn the_real_manifest_follows_the_rules() {
    let manifest = fs::read_to_string(root().join("Cargo.toml")).expect("read Cargo.toml");
    let violations = manifest_violations(&manifest);
    assert!(violations.is_empty(), "Cargo.toml: {violations:?}");
}

#[test]
fn cargo_metadata_and_the_lockfile_resolve_to_allowed_sources() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(root())
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata emits JSON");

    let mut violations = Vec::new();
    for package in metadata["packages"].as_array().expect("packages") {
        let name = package["name"].as_str().unwrap_or("?");
        if package["source"].is_null() {
            // The workspace's own package.
            assert_eq!(name, "rayx-cli", "only rayx-cli is built from a local path");
            continue;
        }
        let source = package["source"].as_str().unwrap_or("");
        if let Some(violation) = source_violation(name, source) {
            violations.push(violation);
        }
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            assert!(
                dependency.get("path").is_none_or(|p| p.is_null()),
                "{name} depends on {} by path",
                dependency["name"]
            );
        }
    }
    assert!(violations.is_empty(), "{violations:?}");

    let lock: toml::Table = fs::read_to_string(root().join("Cargo.lock"))
        .expect("read Cargo.lock")
        .parse()
        .expect("Cargo.lock parses");
    for package in lock["package"].as_array().expect("[[package]]") {
        let name = package["name"].as_str().unwrap_or("?");
        if let Some(source) = package.get("source").and_then(toml::Value::as_str) {
            assert_eq!(source_violation(name, source), None, "Cargo.lock: {name}");
        }
    }
}
