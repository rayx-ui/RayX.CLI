//! An app made by `rayx new` on macOS and the iOS simulator (PROOF-2), ignored by default: each
//! test installs rayx's default nsh into a temporary `RAYX_HOME` of its own (a source build of
//! several minutes while the fork has no release) and runs the app's tests in the AppKit host or
//! in a simulator, which nsh boots when none is booted.
//!
//! ```text
//! cargo test --test proof_app_apple -- --ignored --test-threads=1
//! ```

mod common;

use common::Workspace;
use common::proof::rayx_ok;

/// `rayx new hello` in a temporary directory with its own store, then `rayx app test --target
/// <target>` in it.
fn new_app_passes_its_tests_on(target: &str) {
    if !cfg!(target_os = "macos") {
        eprintln!("skipped: the Apple proof runs on a Mac");
        return;
    }
    let home = Workspace::empty();
    let work = Workspace::empty();
    rayx_ok(&home.root, &work.root, &["new", "hello"]);
    rayx_ok(
        &home.root,
        &work.path("hello"),
        &["app", "test", "--target", target],
    );
}

#[test]
#[ignore = "installs nsh and runs an app's tests in the macOS host (many minutes)"]
fn a_new_mixed_app_passes_its_tests_on_macos() {
    new_app_passes_its_tests_on("macos");
}

#[test]
#[ignore = "installs nsh and runs an app's tests in the iOS simulator (many minutes)"]
fn a_new_mixed_app_passes_its_tests_on_the_ios_simulator() {
    new_app_passes_its_tests_on("ios");
}
