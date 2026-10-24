//! Gamify through rayx (PROOF-3), ignored by default. It reads Gamify's frontend
//! (`RAYX_PROOF_GAMIFY`, default `../../Six/Six.Gamify/src/frontend` from this repository) and
//! changes none of its tracked files: rayx shows the pin from Gamify's `Cargo.lock`, installs it
//! into a temporary `RAYX_HOME` (a source build of several minutes while the fork has no release),
//! type checks the app, packages it for the web and serves the package.
//!
//! Gamify's app imports `config/runtime.json`, which its `cargo xtask config` writes; the proof
//! needs that file and never writes it.
//!
//! ```text
//! cargo test --test proof_gamify -- --ignored --test-threads=1
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use common::Workspace;
use common::proof::{Running, free_port, get, rayx, rayx_ok, rayx_output, wait_for_ok};

fn frontend() -> PathBuf {
    let dir = std::env::var_os("RAYX_PROOF_GAMIFY").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Six/Six.Gamify/src/frontend"),
        PathBuf::from,
    );
    std::fs::canonicalize(&dir).unwrap_or_else(|_| {
        panic!(
            "no Gamify frontend at {}: set RAYX_PROOF_GAMIFY to its src/frontend",
            dir.display()
        )
    })
}

/// Gamify's tracked files as git sees them: the status of tracked paths and the diff against
/// `HEAD`, so work in progress there is compared, not required to be committed.
fn tracked_state(dir: &Path) -> (String, Vec<u8>) {
    let git = |args: &[&str]| {
        // `--no-optional-locks`: a status refreshes Gamify's index otherwise, under the owner's
        // feet.
        let output = Command::new("git")
            .arg("--no-optional-locks")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?} in {}", dir.display());
        output.stdout
    };
    let status = String::from_utf8(git(&["status", "--porcelain=v1", "--untracked-files=no"]))
        .expect("UTF-8");
    (status, git(&["diff", "--binary", "HEAD"]))
}

/// The first file under `dir` whose name ends with `suffix`, relative to `dir`.
fn find(dir: &Path, suffix: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.to_string_lossy().ends_with(suffix) {
                return path.strip_prefix(dir).ok().map(Path::to_path_buf);
            }
        }
    }
    None
}

#[test]
#[ignore = "installs nsh and packages Gamify for the web (network, many minutes)"]
fn gamify_shows_its_pin_checks_packages_and_serves_through_rayx_unchanged() {
    let gamify = frontend();
    assert!(
        gamify.join("config/runtime.json").is_file(),
        "Gamify has no config/runtime.json: run `cargo xtask config` in {} first",
        gamify.display()
    );
    let before = tracked_state(&gamify);
    let home = Workspace::empty();

    let status = rayx_output(&home.root, &gamify, &["nsh", "status"]);
    let status_text = String::from_utf8_lossy(&status.stdout).into_owned();
    assert!(status.status.success(), "{status_text}");
    assert!(
        status_text.contains("fork-0.2.0-4 (327bb9e)") && status_text.contains("project"),
        "{status_text}"
    );

    rayx_ok(&home.root, &gamify, &["app", "check", "."]);

    let out = Workspace::empty();
    let out_text = out.root.display().to_string();
    rayx_ok(
        &home.root,
        &gamify,
        &["app", "package", ".", "--target", "web", "--out", &out_text],
    );
    let site = out.path("Gamify");
    assert!(site.join("index.html").is_file(), "{}", site.display());
    let wasm = find(&site, ".wasm").expect("the package has a .wasm");

    let port = free_port();
    let port_text = port.to_string();
    let site_text = site.display().to_string();
    let mut serve = Running::start(
        rayx(
            &home.root,
            &out.root,
            &["serve", "--dir", &site_text, "--port", &port_text],
        ),
        "rayx serve",
    );
    wait_for_ok(&mut serve, port, "/health", Duration::from_secs(30));
    let page = get(port, "/").expect("the page");
    assert_eq!(page.status, 200);
    assert_eq!(
        page.body,
        std::fs::read(site.join("index.html")).expect("index.html")
    );
    let url = format!(
        "/{}",
        wasm.components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    );
    let module = get(port, &url).expect("the module");
    assert_eq!(module.status, 200, "{url}");
    assert_eq!(
        module.header("content-type"),
        Some("application/wasm"),
        "{url}"
    );
    assert_eq!(
        module.body.len() as u64,
        std::fs::metadata(site.join(&wasm)).expect("wasm").len()
    );
    drop(serve);

    let after = tracked_state(&gamify);
    assert_eq!(before.0, after.0, "the status of Gamify's tracked files");
    assert!(before.1 == after.1, "Gamify's tracked files changed");
}
