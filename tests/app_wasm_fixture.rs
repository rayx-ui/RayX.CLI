//! The threaded WebAssembly path on a project with only public crates: `rayx app <fixture> build
//! wasm` builds and verifies it, `run wasm` serves it cross-origin isolated.
//!
//! The runs need the pinned nightly and the locked `wasm-bindgen` (`rayx setup --web`), so they
//! are `#[ignore]`d in a plain `cargo test`; select them with
//! `cargo test --test app_wasm_fixture -- --ignored`.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

use common::Workspace;

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind")
        .local_addr()
        .expect("address")
        .port()
}

/// One HTTP/1.1 GET; the whole response, headers included.
fn get(port: u16, path: &str) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
#[ignore = "needs the web set: rayx setup --web"]
fn the_fixture_builds_a_verified_threaded_module() {
    let workspace = Workspace::fixture("threaded_wasm_app");

    rayx_cli::app::run_unchecked(vec![
        workspace.root.display().to_string(),
        "build".into(),
        "wasm".into(),
    ])
    .expect("the threaded build passes the shared-memory verification");

    let out = workspace.path("artifacts/apps/threaded-wasm-app/wasm/release");
    for file in [
        "index.html",
        "threaded_wasm_app_web.js",
        "threaded_wasm_app_web_bg.wasm",
    ] {
        assert!(out.join(file).is_file(), "{file}");
    }
    assert!(out.join("assets").is_dir());
    let module = std::fs::read(out.join("threaded_wasm_app_web_bg.wasm")).expect("module");
    assert!(
        module
            .windows(b"wasm_thread_entry_point".len())
            .any(|window| window == b"wasm_thread_entry_point"),
        "the module exports the thread entry point"
    );
    let host = std::fs::read_to_string(out.join("index.html")).expect("host page");
    assert!(host.contains("/threaded_wasm_app_web.js"));
}

#[test]
#[ignore = "needs the web set: rayx setup --web"]
fn run_wasm_serves_the_fixture_cross_origin_isolated() {
    let workspace = Workspace::fixture("threaded_wasm_app");
    let port = free_port();
    let app = workspace.root.display().to_string();
    // `run wasm` builds, then serves until the process ends.
    thread::spawn(move || {
        let _ = rayx_cli::app::run_unchecked(vec![
            app,
            "run".into(),
            "wasm".into(),
            "--port".into(),
            port.to_string(),
        ]);
    });

    let start = Instant::now();
    let response = loop {
        match get(port, "/") {
            Ok(response) => break response,
            Err(error) if start.elapsed() < Duration::from_secs(900) => {
                let _ = error;
                thread::sleep(Duration::from_millis(500));
            }
            Err(error) => panic!("the fixture was not served on port {port}: {error}"),
        }
    };

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains("Cross-Origin-Opener-Policy: same-origin"),
        "{response}"
    );
    assert!(
        response.contains("Cross-Origin-Embedder-Policy: require-corp"),
        "{response}"
    );
    assert!(response.contains("threaded_wasm_app_web.js"), "{response}");

    let module = get(port, "/threaded_wasm_app_web_bg.wasm").expect("module request");
    assert!(
        module.contains("Content-Type: application/wasm"),
        "{module}"
    );
    assert!(
        module.contains("Cross-Origin-Embedder-Policy: require-corp"),
        "the module is isolated too"
    );
}
