//! Helpers of the real proofs (`proof_*` groups): the `rayx` binary run with a store of its own,
//! long-running commands stopped with their children, and plain HTTP/1.1 requests.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `rayx <args>` in `dir` with the store in `home`, as a user would run it: no `CARGO` from the
/// test harness.
pub fn rayx(home: &Path, dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rayx"));
    command
        .args(args)
        .current_dir(dir)
        .env("RAYX_HOME", home)
        .env_remove("CARGO");
    command
}

/// Runs `rayx <args>` to the end, showing its output as it comes, and fails the test when it does.
pub fn rayx_ok(home: &Path, dir: &Path, args: &[&str]) {
    let status = rayx(home, dir, args)
        .stdin(Stdio::null())
        .status()
        .expect("rayx");
    assert!(
        status.success(),
        "rayx {args:?} in {} failed: {status}",
        dir.display()
    );
}

/// Runs `rayx <args>` to the end and returns its output, whatever its status.
pub fn rayx_output(home: &Path, dir: &Path, args: &[&str]) -> Output {
    rayx(home, dir, args)
        .stdin(Stdio::null())
        .output()
        .expect("rayx")
}

/// A port nothing listens on right now.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .expect("a free port")
}

/// A command that runs until the test stops it: in a process group of its own on Unix, and as a
/// process tree on Windows, so stopping it stops what it started (nsh's dev server and its
/// builds).
pub struct Running {
    child: Child,
    what: String,
}

impl Running {
    pub fn start(mut command: Command, what: &str) -> Self {
        command.stdin(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command
            .spawn()
            .unwrap_or_else(|error| panic!("{what}: {error}"));
        Self {
            child,
            what: what.to_string(),
        }
    }

    /// Fails the test when the command has exited.
    pub fn assert_running(&mut self) {
        if let Ok(Some(status)) = self.child.try_wait() {
            panic!("{} exited early: {status}", self.what);
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // The group may be gone already; `kill` need not say so.
            let group = format!("-{}", self.child.id());
            let kill = |signal: &str| {
                let _ = Command::new("kill")
                    .args([signal, "--", &group])
                    .stderr(Stdio::null())
                    .status();
            };
            kill("-TERM");
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            kill("-KILL");
        }
        // On Windows rayx waits for nsh as a child of its own: stop the whole tree.
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &self.child.id().to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An HTTP response: the status code, the headers (names in lower case) and the body.
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// `GET <path>` on 127.0.0.1:`port`; `None` when nothing answers.
pub fn get(port: u16, path: &str) -> Option<Response> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let mut body = raw[end + 4..].to_vec();
    if headers
        .iter()
        .any(|(key, value)| key == "transfer-encoding" && value.eq_ignore_ascii_case("chunked"))
    {
        body = unchunk(&body);
    }
    Some(Response {
        status,
        headers,
        body,
    })
}

fn unchunk(mut data: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    while let Some(line_end) = data.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&data[..line_end])
            .ok()
            .and_then(|size| usize::from_str_radix(size.split(';').next()?.trim(), 16).ok())
            .unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = line_end + 2;
        let Some(chunk) = data.get(start..start + size) else {
            break;
        };
        body.extend_from_slice(chunk);
        data = data.get(start + size + 2..).unwrap_or_default();
    }
    body
}

/// Waits until `GET <path>` answers 200 on `port`, checking that `running` is still up; fails the
/// test after `timeout`.
pub fn wait_for_ok(running: &mut Running, port: u16, path: &str, timeout: Duration) -> Response {
    let deadline = Instant::now() + timeout;
    loop {
        running.assert_running();
        if let Some(response) = get(port, path)
            && response.status == 200
        {
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "nothing answered 200 on 127.0.0.1:{port}{path} within {timeout:?}"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
}
