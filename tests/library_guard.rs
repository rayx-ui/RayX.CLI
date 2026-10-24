//! rayx names NSH, its toolchain, and nothing above it (D14). It runs the `nsh` CLI a project pins
//! and depends on no NSH crate, and it knows no library or app built on NSH: no RayX crate, no gpux
//! or GPUI library, no app.
//!
//! The guard fails, listing `file:line: token`, when
//! - the CLI's `Cargo.toml` or `Cargo.lock` names a RayX, gpux, GPUI or NSH crate (`rayx_app*`,
//!   `rayx_runtime`, `rayx`, `gpux*`, `gpui*`, `nsh-*`);
//! - non-test source under `src/` names `rayx_app`, `gpux`, `gpui`, `GameX` or `Gamify`, or
//!   reads `env!("CARGO_MANIFEST_DIR")`;
//! - an allowlist entry matches nothing any more.
//!
//! Names compare case-insensitively with `-` and `_` equal. Each rule has a failing fixture below,
//! built in a temporary directory and scanned by the same code that scans this repository.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use common::Workspace;

/// Crates the CLI must not depend on, by exact name.
const DENIED_CRATES: &[&str] = &["rayx", "rayx_runtime"];

/// Crate families the CLI must not depend on.
const DENIED_CRATE_PREFIXES: &[&str] = &["rayx_app", "gpux", "gpui", "nsh_"];

/// Name families non-test source must not contain: RayX's app crates, the gpux and GPUI libraries
/// and the apps built on NSH.
const DENIED_SOURCE_PREFIXES: &[&str] = &["rayx_app", "gpux", "gpui", "gamex", "gamify"];

/// The one compile-time path non-test source must not read: it ties rayx to its own checkout.
const MANIFEST_DIR: &str = "env!(\"CARGO_MANIFEST_DIR\")";

/// The argument that makes an `env!` call [`MANIFEST_DIR`].
const MANIFEST_DIR_ARGUMENT: &str = "\"CARGO_MANIFEST_DIR\"";

/// Uses of a denied name that are nsh's own vocabulary: `(file, token, reason)`. Every entry must
/// still match something.
const ALLOWLIST: &[Allowed] = &[(
    "src/nsh/pin.rs",
    "gpui",
    "NSH's own crate nsh-platform-gpui, in NSH's crates/gpui workspace",
)];

/// An allowlist entry: `(file, token, reason)`.
type Allowed = (&'static str, &'static str, &'static str);

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn normalized(token: &str) -> String {
    token.to_ascii_lowercase().replace('-', "_")
}

/// Whether `name` is a crate the CLI must not depend on.
fn is_denied_crate(name: &str) -> bool {
    let name = normalized(name);
    DENIED_CRATES.contains(&name.as_str())
        || DENIED_CRATE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// Whether `token` is a name non-test source must not contain.
fn is_denied_in_source(token: &str) -> bool {
    let token = normalized(token);
    DENIED_SOURCE_PREFIXES
        .iter()
        .any(|prefix| token.starts_with(prefix))
}

/// `text` without its `#[cfg(test)] mod <name> { ... }` blocks (string literals, raw strings,
/// char literals and comments do not count as braces).
fn strip_test_modules(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = find_test_module(rest) {
        out.push_str(&rest[..start]);
        let open = start + rest[start..].find('{').expect("the module's brace");
        let end = matching_brace(rest, open);
        // Keep the line count so reported lines stay true.
        out.extend(rest[start..end].chars().filter(|ch| *ch == '\n'));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The offset of the next `#[cfg(test)]` that is followed by `mod <name> {`.
fn find_test_module(text: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(found) = text[offset..].find("#[cfg(test)]") {
        let start = offset + found;
        let after = text[start + "#[cfg(test)]".len()..].trim_start();
        let after = after.strip_prefix("pub(crate) ").unwrap_or(after);
        let after = after.strip_prefix("pub ").unwrap_or(after);
        if let Some(module) = after.strip_prefix("mod ") {
            let name_end = module
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .unwrap_or(module.len());
            if module[name_end..].trim_start().starts_with('{') {
                return Some(start);
            }
        }
        offset = start + 1;
    }
    None
}

/// The offset just past the brace that closes the one at `open`.
fn matching_brace(text: &str, open: usize) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b'r' if matches!(bytes.get(i + 1), Some(b'#' | b'"'))
                && !bytes
                    .get(i.wrapping_sub(1))
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') =>
            {
                let hashes = bytes[i + 1..].iter().take_while(|b| **b == b'#').count();
                if bytes.get(i + 1 + hashes) == Some(&b'"') {
                    let closing = format!("\"{}", "#".repeat(hashes));
                    let body = i + 2 + hashes;
                    i = body
                        + text[body..].find(&closing).expect("closed raw string")
                        + closing.len()
                        - 1;
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'\'' => {
                // A char literal (`'{'`, `'\''`); a lifetime has no closing quote nearby.
                if bytes.get(i + 1) == Some(&b'\\') {
                    if let Some(close) = text[i + 2..].find('\'') {
                        i += 2 + close;
                    }
                } else if bytes.get(i + 2) == Some(&b'\'') {
                    i += 2;
                }
            }
            _ => {}
        }
        i += 1;
    }
    panic!("unbalanced braces after offset {open}");
}

/// The denied `(line, token)` pairs of a source text, test modules left out.
fn scan_source(text: &str) -> Vec<(usize, String)> {
    let text = strip_test_modules(text);
    let mut found = Vec::new();
    // The text without whitespace, with the line each kept character is on, so a macro call
    // spread over several lines is found too.
    let (compact, lines): (String, Vec<usize>) = text
        .lines()
        .enumerate()
        .flat_map(|(index, line)| line.chars().map(move |ch| (ch, index + 1)))
        .filter(|(ch, _)| !ch.is_whitespace())
        .unzip();
    let mut offset = 0;
    while let Some(at) = compact[offset..].find(MANIFEST_DIR_ARGUMENT) {
        let start = offset + at;
        let call = compact[..start].trim_end_matches(['(', '[', '{']);
        if compact[..start].len() == call.len() + 1 && call.ends_with("env!") {
            let line = lines[compact[..start].chars().count()];
            found.push((line, MANIFEST_DIR.to_string()));
        }
        offset = start + MANIFEST_DIR_ARGUMENT.len();
    }
    for (index, line) in text.lines().enumerate() {
        for token in line
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
            .filter(|token| !token.is_empty())
        {
            if is_denied_in_source(token) {
                found.push((index + 1, token.to_string()));
            }
        }
    }
    found.sort();
    found
}

/// Files of `src` compiled only for tests: declared under `#[cfg(test)] mod <name>;`.
fn test_only_files(files: &[PathBuf]) -> BTreeSet<PathBuf> {
    let mut skipped = BTreeSet::new();
    for file in files {
        let text = fs::read_to_string(file).expect("read");
        let dir = module_dir(file);
        let mut lines = text.lines().peekable();
        while let Some(line) = lines.next() {
            if line.trim() != "#[cfg(test)]" {
                continue;
            }
            if let Some(next) = lines.peek()
                && let Some(name) = next
                    .trim()
                    .strip_prefix("mod ")
                    .and_then(|rest| rest.strip_suffix(';'))
            {
                skipped.insert(dir.join(format!("{name}.rs")));
                skipped.insert(dir.join(name).join("mod.rs"));
            }
        }
    }
    skipped
}

/// The directory the child modules of `file` live in: beside `lib.rs`, `main.rs` and `mod.rs`,
/// in `<name>/` for any other `<name>.rs`.
fn module_dir(file: &Path) -> PathBuf {
    let parent = file.parent().expect("parent");
    match file.file_name().and_then(|name| name.to_str()) {
        Some("lib.rs" | "main.rs" | "mod.rs") => parent.to_path_buf(),
        _ => parent.join(file.file_stem().expect("a file name")),
    }
}

fn source_files(src: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    if src.is_dir() {
        walk(src, &mut files);
    }
    files.sort();
    files
}

/// Every dependency key and `package` name of a `Cargo.toml`, in every dependency table.
fn manifest_dependency_names(manifest: &toml::Table) -> Vec<String> {
    let mut tables = Vec::new();
    for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = manifest.get(kind).and_then(toml::Value::as_table) {
            tables.push(table);
        }
        if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
            tables.extend(
                targets
                    .values()
                    .filter_map(|target| target.get(kind).and_then(toml::Value::as_table)),
            );
        }
    }
    if let Some(workspace) = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        tables.push(workspace);
    }
    let mut names = Vec::new();
    for table in tables {
        for (key, value) in table {
            names.push(key.clone());
            if let Some(package) = value.get("package").and_then(toml::Value::as_str) {
                names.push(package.to_string());
            }
        }
    }
    names
}

/// The package names a `Cargo.lock` locks.
fn lock_package_names(lock: &toml::Table) -> Vec<String> {
    lock.get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|package| package.get("name").and_then(toml::Value::as_str))
        .map(str::to_string)
        .collect()
}

/// What the guard found in one project directory.
#[derive(Debug, Default)]
struct Report {
    /// `file:line: token` for source, `file: crate` for the manifest and the lock.
    violations: Vec<String>,
    /// Allowlist entries that matched nothing.
    stale: Vec<String>,
}

/// Runs the guard over `root` (its `Cargo.toml`, `Cargo.lock` and `src/`) with `allowlist`.
fn guard(root: &Path, allowlist: &[Allowed]) -> Report {
    let mut report = Report::default();
    for (file, names) in [
        (
            "Cargo.toml",
            read_toml(&root.join("Cargo.toml")).map(|m| manifest_dependency_names(&m)),
        ),
        (
            "Cargo.lock",
            read_toml(&root.join("Cargo.lock")).map(|l| lock_package_names(&l)),
        ),
    ] {
        for name in names.unwrap_or_default() {
            if is_denied_crate(&name) {
                report.violations.push(format!("{file}: {name}"));
            }
        }
    }

    let files = source_files(&root.join("src"));
    let skipped = test_only_files(&files);
    let mut used = BTreeSet::new();
    for file in &files {
        if skipped.contains(file) {
            continue;
        }
        let path = file
            .strip_prefix(root)
            .expect("under the project")
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(file).expect("read source");
        for (line, token) in scan_source(&text) {
            let allowed = allowlist
                .iter()
                .any(|(entry, allowed, _)| *entry == path && *allowed == token);
            if allowed {
                used.insert((path.clone(), token));
            } else {
                report.violations.push(format!("{path}:{line}: {token}"));
            }
        }
    }
    report.stale = allowlist
        .iter()
        .filter(|(path, token, _)| !used.contains(&(path.to_string(), token.to_string())))
        .map(|(path, token, reason)| format!("{path}: {token} ({reason})"))
        .collect();
    report
}

fn read_toml(path: &Path) -> Option<toml::Table> {
    let text = fs::read_to_string(path).ok()?;
    Some(
        text.parse()
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display())),
    )
}

/// A fixture project: a clean manifest, lock and library, changed by `edit`.
fn fixture(edit: impl FnOnce(&Workspace)) -> Workspace {
    let project = Workspace::empty();
    project.write(
        "Cargo.toml",
        "[package]\nname = \"rayx-cli\"\nversion = \"1.0.0\"\n\n[dependencies]\nanyhow = \"1\"\n",
    );
    project.write(
        "Cargo.lock",
        "version = 4\n\n[[package]]\nname = \"anyhow\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"rayx-cli\"\nversion = \"1.0.0\"\n",
    );
    project.write(
        "src/lib.rs",
        "//! Runs the nsh a project pins, in its `nsh-*` crates' directory.\npub fn run() {}\n",
    );
    edit(&project);
    project
}

#[test]
fn a_clean_project_passes() {
    let report = guard(&fixture(|_| {}).root, &[]);
    assert!(report.violations.is_empty(), "{report:?}");
    assert!(report.stale.is_empty(), "{report:?}");
}

#[test]
fn a_manifest_naming_a_rayx_gpux_gpui_or_nsh_crate_fails() {
    for (dependency, expected) in [
        ("rayx_app = \"0.4\"", "rayx_app"),
        ("rayx-app-di = \"0.4\"", "rayx-app-di"),
        ("rayx_runtime = \"0.1\"", "rayx_runtime"),
        ("rayx = \"0.4\"", "rayx"),
        ("gpux-fonts = \"0.1\"", "gpux-fonts"),
        ("gpui = \"0.2\"", "gpui"),
        ("nsh-core = \"0.2\"", "nsh-core"),
        (
            "core = { package = \"nsh-core\", git = \"https://github.com/dXnSix/nsh\", rev = \"327bb9e\" }",
            "nsh-core",
        ),
    ] {
        let project = fixture(|p| p.append("Cargo.toml", &format!("{dependency}\n")));
        let report = guard(&project.root, &[]);
        assert_eq!(
            report.violations,
            [format!("Cargo.toml: {expected}")],
            "{dependency}"
        );
    }
    // Every dependency table counts, the target-specific and the development ones included.
    let project = fixture(|p| {
        p.append(
            "Cargo.toml",
            "\n[dev-dependencies]\nnsh_cli = \"0.2\"\n\n[target.'cfg(windows)'.dependencies]\ngpux = \"0.1\"\n",
        )
    });
    let mut violations = guard(&project.root, &[]).violations;
    violations.sort();
    assert_eq!(violations, ["Cargo.toml: gpux", "Cargo.toml: nsh_cli"]);
}

#[test]
fn a_lock_naming_a_rayx_gpux_gpui_or_nsh_crate_fails() {
    let project = fixture(|p| {
        p.append(
            "Cargo.lock",
            "\n[[package]]\nname = \"nsh-platform-web\"\nversion = \"0.2.0\"\n\n[[package]]\nname = \"rayx_app_macros\"\nversion = \"0.4.0\"\n",
        )
    });
    let report = guard(&project.root, &[]);
    assert_eq!(
        report.violations,
        [
            "Cargo.lock: nsh-platform-web",
            "Cargo.lock: rayx_app_macros"
        ]
    );
    // The CLI's own package is not a library, and names that merely start alike are fine.
    for name in ["rayx-cli", "nshell", "rayxtra"] {
        assert!(!is_denied_crate(name), "{name}");
    }
}

#[test]
fn source_naming_a_library_or_an_app_fails_with_its_line() {
    for (planted, token) in [
        ("use rayx_app::App;", "rayx_app"),
        ("const X: &str = \"rayx-app-di\";", "rayx-app-di"),
        ("// Draws with gpux.", "gpux"),
        ("const T: &str = \"gpui\";", "gpui"),
        ("/// The GameX assets.", "GameX"),
        ("const APP: &str = \"Gamify\";", "Gamify"),
    ] {
        let project = fixture(|p| p.append("src/lib.rs", &format!("{planted}\n")));
        let report = guard(&project.root, &[]);
        assert_eq!(
            report.violations,
            [format!("src/lib.rs:3: {token}")],
            "{planted}"
        );
    }
}

#[test]
fn source_reading_the_manifest_directory_fails() {
    for planted in [
        "const ROOT: &str = env!(\"CARGO_MANIFEST_DIR\");",
        "fn root() -> &'static str { env!( \"CARGO_MANIFEST_DIR\" ) }",
        "const ROOT: &str = env![\"CARGO_MANIFEST_DIR\"];",
    ] {
        let project = fixture(|p| p.write("src/project.rs", &format!("{planted}\n")));
        let report = guard(&project.root, &[]);
        assert_eq!(
            report.violations,
            [format!("src/project.rs:1: {MANIFEST_DIR}")],
            "{planted}"
        );
    }
    // A call spread over lines is found on the line of its argument; the name alone is not a use.
    let project = fixture(|p| {
        p.write(
            "src/project.rs",
            "const ROOT: &str = env!(\n    \"CARGO_MANIFEST_DIR\"\n);\nconst NAME: &str = \"CARGO_MANIFEST_DIR\";\n",
        )
    });
    let report = guard(&project.root, &[]);
    assert_eq!(
        report.violations,
        [format!("src/project.rs:2: {MANIFEST_DIR}")]
    );
}

#[test]
fn test_modules_and_test_only_files_are_not_scanned() {
    let project = fixture(|p| {
        p.append(
            "src/lib.rs",
            r#"
#[cfg(test)]
mod support;

#[cfg(test)]
mod tests {
    #[test]
    fn inside() {
        let _ = ("gpux-fonts rayx_app Gamify { '{' }", env!("CARGO_MANIFEST_DIR"));
    }
}

fn after() { let _ = "gpui"; }
"#,
        );
        p.write(
            "src/support.rs",
            "pub const ROOT: &str = env!(\"CARGO_MANIFEST_DIR\"); // Gamify fixtures\n",
        );
    });
    let report = guard(&project.root, &[]);
    assert_eq!(
        report.violations,
        ["src/lib.rs:15: gpui"],
        "lines after a test module stay true"
    );
}

#[test]
fn a_test_only_module_of_a_named_file_skips_its_own_file_only() {
    let project = fixture(|p| {
        p.append("src/lib.rs", "pub mod app;\npub mod store;\n");
        p.write("src/app.rs", "#[cfg(test)]\nmod store;\n");
        // The test-only module of `app.rs` lives in `src/app/`.
        p.write("src/app/store.rs", "const FIXTURE: &str = \"Gamify\";\n");
        // `src/store.rs` is a real module of the library and is still scanned.
        p.write("src/store.rs", "const APP: &str = \"GameX\";\n");
    });
    let report = guard(&project.root, &[]);
    assert_eq!(report.violations, ["src/store.rs:1: GameX"]);
}

#[test]
fn an_allowlisted_use_passes_and_a_stale_entry_fails() {
    let project = fixture(|p| {
        p.write(
            "src/app.rs",
            "pub const DESKTOP: &[&str] = &[\"macos\", \"gpui\"];\n",
        )
    });
    let allowlist: &[Allowed] = &[
        ("src/app.rs", "gpui", "nsh's --target gpui"),
        ("src/setup.rs", "gpux", "an entry whose use is gone"),
    ];
    let report = guard(&project.root, allowlist);
    assert!(report.violations.is_empty(), "{report:?}");
    assert_eq!(
        report.stale,
        ["src/setup.rs: gpux (an entry whose use is gone)"]
    );
    // The allowlist names a token in one file only.
    let report = guard(&project.root, &[("src/lib.rs", "gpui", "elsewhere")]);
    assert_eq!(report.violations, ["src/app.rs:1: gpui"]);
}

#[test]
fn this_repository_names_no_library_and_every_allowlist_entry_still_matches() {
    let report = guard(&root(), ALLOWLIST);
    assert!(
        report.violations.is_empty(),
        "rayx names a library, an app or its own checkout:\n{}",
        report.violations.join("\n")
    );
    assert!(
        report.stale.is_empty(),
        "allowlist entries that match nothing:\n{}",
        report.stale.join("\n")
    );
    // The scan sees this repository's manifest and lock.
    let manifest = read_toml(&root().join("Cargo.toml")).expect("Cargo.toml");
    assert!(manifest_dependency_names(&manifest).contains(&"toml".to_string()));
    let lock = read_toml(&root().join("Cargo.lock")).expect("Cargo.lock");
    assert!(lock_package_names(&lock).contains(&"rayx-cli".to_string()));
}
