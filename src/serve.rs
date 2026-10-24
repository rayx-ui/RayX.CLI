//! `rayx serve [--dir <site>] [--port <port>]`: a static host for what `nsh package --target web`
//! writes (design §5, SERVE-1). One blocking HTTP/1.1 server on `std::net`, on 127.0.0.1 only:
//! files with their MIME types (`.wasm` as `application/wasm`), `index.html` for a directory,
//! `GET /health`, nothing outside the site, and `Cache-Control: no-store` on every response.
//!
//! It takes the place of the static host an app would otherwise carry in its own tooling.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::cli::ServeArgs;

/// The port when neither `--port` nor `PORT` names one.
pub const DEFAULT_PORT: u16 = 8080;

/// Runs `rayx serve` until the process stops; returns the exit code when it cannot start.
pub fn run(args: &ServeArgs) -> u8 {
    let dir = args
        .dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let port = match port(args.port, std::env::var("PORT").ok().as_deref()) {
        Ok(port) => port,
        Err(error) => {
            eprintln!("rayx: {error:#}");
            return 2;
        }
    };
    match bind(&dir, port) {
        Ok(server) => {
            println!(
                "rayx: serving {} on http://127.0.0.1:{}",
                server.root.display(),
                server.port()
            );
            server.serve();
            0
        }
        Err(error) => {
            eprintln!("rayx: {error:#}");
            1
        }
    }
}

/// `--port`, else `PORT`, else 8080.
pub fn port(flag: Option<u16>, env: Option<&str>) -> Result<u16> {
    if let Some(port) = flag {
        return Ok(port);
    }
    match env.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => value
            .parse()
            .with_context(|| format!("PORT is {value:?}, which is no port")),
        None => Ok(DEFAULT_PORT),
    }
}

/// A bound server: the site and the listener.
pub struct Server {
    pub root: PathBuf,
    listener: TcpListener,
}

/// Binds `127.0.0.1:<port>` (0 picks a free one) for the site `dir`.
pub fn bind(dir: &Path, port: u16) -> Result<Server> {
    let root = std::fs::canonicalize(dir).with_context(|| {
        format!(
            "no site at {} (`rayx app package --target web --out <dir>` writes one)",
            dir.display()
        )
    })?;
    if !root.is_dir() {
        anyhow::bail!("{} is not a directory", root.display());
    }
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("cannot listen on 127.0.0.1:{port} (is the port taken?)"))?;
    Ok(Server { root, listener })
}

impl Server {
    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.listener.local_addr().map_or(0, |addr| addr.port())
    }

    /// Answers requests, one thread each, until the process stops.
    pub fn serve(self) {
        for stream in self.listener.incoming() {
            let Ok(stream) = stream else { continue };
            let root = self.root.clone();
            std::thread::spawn(move || {
                let _ = respond(&root, stream);
            });
        }
    }
}

/// The MIME type of `path`, by its extension.
pub fn mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "json" | "map" => "application/json",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// `%xx` decoding of a request path; `None` for a broken escape or bytes that are no UTF-8.
fn percent_decode(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The file of `root` a request `target` (`/a/b?x`) names, if it is inside `root`: query and
/// fragment dropped, percent-decoded, `index.html` for a directory; `None` for `..` (encoded or
/// not), a backslash, or a link that leads out of the site.
pub fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_decode(path)?;
    if decoded.contains('\\') {
        return None;
    }
    let mut file = root.to_path_buf();
    for component in Path::new(decoded.trim_start_matches('/')).components() {
        match component {
            Component::Normal(part) => file.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if file.is_dir() {
        file.push("index.html");
    }
    // A symbolic link must not lead out of the site.
    let real = std::fs::canonicalize(&file).ok()?;
    real.starts_with(root).then_some(file)
}

/// Answers one request on `stream` (HTTP/1.1, `Connection: close`).
fn respond(root: &Path, mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // Skip the headers.
    let mut line = String::new();
    while reader.read_line(&mut line)? > 2 {
        line.clear();
    }
    let mut parts = request_line.split_whitespace();
    let (method, target) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or("/"),
    );
    if method != "GET" && method != "HEAD" {
        return write_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            b"",
            method,
        );
    }
    if target.split(['?', '#']).next() == Some("/health") {
        return write_response(
            &mut stream,
            "200 OK",
            "text/plain; charset=utf-8",
            b"ok",
            method,
        );
    }
    match resolve(root, target).and_then(|file| Some((std::fs::read(&file).ok()?, file))) {
        Some((body, file)) => {
            write_response(&mut stream, "200 OK", mime_type(&file), &body, method)
        }
        None => write_response(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"not found",
            method,
        ),
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    method: &str,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    if method != "HEAD" {
        stream.write_all(body)?;
    }
    stream.flush()
}
