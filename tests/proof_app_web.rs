//! An app made by `rayx new` on the web (PROOF-1), ignored by default: it installs rayx's default
//! nsh into a temporary `RAYX_HOME` (a source build of several minutes while the fork has no
//! release), builds the app and runs it in Chrome, which needs the network.
//!
//! ```text
//! cargo test --test proof_app_web -- --ignored --test-threads=1
//! ```

mod common;

use std::time::Duration;

use common::Workspace;
use common::proof::{Running, free_port, get, rayx, rayx_ok, wait_for_ok};

#[test]
#[ignore = "installs nsh and builds an app for the web (network, many minutes)"]
fn a_new_mixed_app_builds_runs_tests_and_packages_for_the_web() {
    let home = Workspace::empty();
    let work = Workspace::empty();
    rayx_ok(&home.root, &work.root, &["new", "hello"]);
    let app = work.path("hello");
    assert!(
        app.join("Cargo.lock").is_file(),
        "the pin is in the app's lock"
    );

    rayx_ok(&home.root, &app, &["app", "build"]);

    // The dev server answers on the port it was given, or the command fails.
    let port = free_port();
    let port_text = port.to_string();
    let mut server = Running::start(
        rayx(
            &home.root,
            &app,
            &[
                "app",
                "run",
                "--target",
                "web",
                "--strict-port",
                "--port",
                &port_text,
            ],
        ),
        "rayx app run --target web",
    );
    let version = wait_for_ok(&mut server, port, "/nsh.version", Duration::from_secs(900));
    assert!(
        !version.text().trim().is_empty(),
        "nsh.version names the runtime"
    );
    drop(server);

    rayx_ok(&home.root, &app, &["app", "test", "--target", "web"]);

    let out = Workspace::empty();
    let out_text = out.root.display().to_string();
    rayx_ok(
        &home.root,
        &app,
        &["app", "package", "--target", "web", "--out", &out_text],
    );
    let site = out.path("hello");
    assert!(site.join("index.html").is_file(), "{}", site.display());

    let port = free_port();
    let port_text = port.to_string();
    let site_text = site.display().to_string();
    let mut serve = Running::start(
        rayx(
            &home.root,
            &work.root,
            &["serve", "--dir", &site_text, "--port", &port_text],
        ),
        "rayx serve",
    );
    let health = wait_for_ok(&mut serve, port, "/health", Duration::from_secs(30));
    assert_eq!(health.status, 200);
    let page = get(port, "/").expect("the page");
    assert_eq!(page.status, 200);
    assert_eq!(
        page.body,
        std::fs::read(site.join("index.html")).expect("index.html"),
        "/ is index.html"
    );
    assert!(
        page.header("content-type")
            .is_some_and(|t| t.starts_with("text/html")),
        "{:?}",
        page.headers
    );
}
