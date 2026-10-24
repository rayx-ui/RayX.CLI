//! `rayx serve` (SERVE-1) against a running server on a free port: MIME types, `index.html`,
//! `/health`, nothing outside the site, `Cache-Control: no-store`, 127.0.0.1 only.

mod common;

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};

use assert_cmd::cargo::cargo_bin_cmd;
use common::Workspace;
use predicates::str::contains;
use rayx_cli::serve::{DEFAULT_PORT, bind, mime_type, port};

/// A site as `nsh package --target web` writes one, served on a free port.
fn serve_site() -> (Workspace, u16) {
    let site = Workspace::empty();
    site.write("index.html", "<!doctype html><title>app</title>");
    site.write("nsh.1a2b.wasm", "\0asm");
    site.write("nsh.1a2b.js", "export {}");
    site.write("app.css", "body{}");
    site.write("manifest.json", "{}");
    site.write("icons/icon.png", "png");
    site.write("fonts/inter.woff2", "woff2");
    site.write("chunks/a b.nshc", "chunk");
    site.write("chunks/index.html", "<p>chunks</p>");
    let server = bind(&site.root, 0).expect("binds");
    let port = server.port();
    std::thread::spawn(move || server.serve());
    (site, port)
}

/// The status line, the headers and the body of one request.
fn request(port: u16, method: &str, target: &str) -> (String, Vec<String>, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(
        stream,
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\n\r\n"
    )
    .expect("send");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("read");
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("headers end");
    let head = String::from_utf8(response[..split].to_vec()).expect("UTF-8 head");
    let mut lines = head.lines();
    let status = lines.next().expect("status").to_string();
    let headers = lines.map(str::to_string).collect();
    (status, headers, response[split + 4..].to_vec())
}

fn header<'a>(headers: &'a [String], name: &str) -> Option<&'a str> {
    headers.iter().find_map(|h| {
        let (key, value) = h.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

#[test]
fn the_port_is_the_flag_else_port_else_8080() {
    assert_eq!(port(Some(4000), Some("5000")).expect("port"), 4000);
    assert_eq!(port(None, Some("5000")).expect("port"), 5000);
    assert_eq!(port(None, Some(" ")).expect("port"), DEFAULT_PORT);
    assert_eq!(port(None, None).expect("port"), 8080);
    let error = port(None, Some("web")).expect_err("not a port").to_string();
    assert!(error.contains("PORT is \"web\""), "{error}");
}

#[test]
fn it_listens_on_the_loopback_address_only() {
    let site = Workspace::empty();
    let server = bind(&site.root, 0).expect("binds");
    let port = server.port();
    assert_ne!(port, 0);
    std::thread::spawn(move || server.serve());
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    assert_eq!(
        stream.peer_addr().expect("peer").ip(),
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    );
}

#[test]
fn files_are_served_with_their_mime_types() {
    let (_site, port) = serve_site();
    for (target, mime, body) in [
        ("/nsh.1a2b.wasm", "application/wasm", &b"\0asm"[..]),
        (
            "/nsh.1a2b.js",
            "text/javascript; charset=utf-8",
            b"export {}",
        ),
        (
            "/index.html",
            "text/html; charset=utf-8",
            b"<!doctype html><title>app</title>",
        ),
        ("/app.css", "text/css; charset=utf-8", b"body{}"),
        ("/manifest.json", "application/json", b"{}"),
        ("/icons/icon.png", "image/png", b"png"),
        ("/fonts/inter.woff2", "font/woff2", b"woff2"),
        ("/chunks/a%20b.nshc", "application/octet-stream", b"chunk"),
    ] {
        let (status, headers, received) = request(port, "GET", target);
        assert_eq!(status, "HTTP/1.1 200 OK", "{target}");
        assert_eq!(header(&headers, "Content-Type"), Some(mime), "{target}");
        assert_eq!(received, body, "{target}");
        assert_eq!(
            header(&headers, "Content-Length"),
            Some(body.len().to_string().as_str())
        );
    }
    for (file, mime) in [
        ("a.svg", "image/svg+xml"),
        ("a.ttf", "font/ttf"),
        ("A.WASM", "application/wasm"),
    ] {
        assert_eq!(mime_type(std::path::Path::new(file)), mime);
    }
}

#[test]
fn a_directory_serves_its_index_html_and_queries_are_ignored() {
    let (_site, port) = serve_site();
    for target in ["/", "/?sportsDataUrl=http://x&codec=json", "/#top"] {
        let (status, _, body) = request(port, "GET", target);
        assert_eq!(status, "HTTP/1.1 200 OK", "{target}");
        assert_eq!(body, b"<!doctype html><title>app</title>", "{target}");
    }
    let (_, _, body) = request(port, "GET", "/chunks/");
    assert_eq!(body, b"<p>chunks</p>");
}

#[test]
fn health_answers_200() {
    let (_site, port) = serve_site();
    let (status, headers, body) = request(port, "GET", "/health");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(body, b"ok");
    assert_eq!(header(&headers, "Cache-Control"), Some("no-store"));
}

#[test]
fn nothing_outside_the_site_is_served() {
    let outside = Workspace::empty();
    outside.write("secret.txt", "secret");
    let site = Workspace::empty();
    site.write("index.html", "<p>site</p>");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path("secret.txt"), site.path("link.txt")).expect("link");
    let server = bind(&site.root, 0).expect("binds");
    let port = server.port();
    std::thread::spawn(move || server.serve());
    let escape = format!("/{}", "../".repeat(8));
    for target in [
        format!("{escape}{}", outside.path("secret.txt").display()),
        "/../secret.txt".to_string(),
        "/%2e%2e/secret.txt".to_string(),
        "/%2E%2E%2Fsecret.txt".to_string(),
        "/a/%2e%2e/%2e%2e/secret.txt".to_string(),
        "/..%5csecret.txt".to_string(),
        "/%zz".to_string(),
        "/missing.js".to_string(),
        "/link.txt".to_string(),
    ] {
        let (status, headers, body) = request(port, "GET", &target);
        assert_eq!(status, "HTTP/1.1 404 Not Found", "{target}");
        assert_ne!(body, b"secret", "{target}");
        assert_eq!(
            header(&headers, "Cache-Control"),
            Some("no-store"),
            "{target}"
        );
    }
}

#[test]
fn every_response_carries_no_store_and_head_has_no_body() {
    let (_site, port) = serve_site();
    let (status, headers, body) = request(port, "HEAD", "/nsh.1a2b.wasm");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(header(&headers, "Cache-Control"), Some("no-store"));
    assert_eq!(header(&headers, "Content-Length"), Some("4"));
    assert!(body.is_empty());
    let (status, headers, _) = request(port, "POST", "/index.html");
    assert_eq!(status, "HTTP/1.1 405 Method Not Allowed");
    assert_eq!(header(&headers, "Cache-Control"), Some("no-store"));
}

#[test]
fn a_missing_site_or_a_taken_port_fails_with_the_reason() {
    let site = Workspace::empty();
    let error = format!(
        "{:#}",
        bind(&site.path("out/hello"), 0).err().expect("no site")
    );
    assert!(
        error.contains("no site at")
            && error.contains(&site.path("out/hello").display().to_string()),
        "{error}"
    );

    let taken = bind(&site.root, 0).expect("binds");
    let port = taken.port();
    let error = format!("{:#}", bind(&site.root, port).err().expect("taken"));
    assert!(error.contains(&format!("127.0.0.1:{port}")), "{error}");

    cargo_bin_cmd!("rayx")
        .args(["serve", "--port", &port.to_string(), "--dir"])
        .arg(&site.root)
        .assert()
        .code(1)
        .stderr(contains(format!("cannot listen on 127.0.0.1:{port}")));
    cargo_bin_cmd!("rayx")
        .args(["serve", "--port", "0", "--dir"])
        .arg(site.path("missing"))
        .assert()
        .code(1)
        .stderr(contains("no site at"));
}
