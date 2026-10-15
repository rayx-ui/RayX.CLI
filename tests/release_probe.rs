//! `release::exists`: whether a release is published, read from the HTTP status of a headers-only
//! request for its installer, not from curl's exit code (macOS curl ends a 404 HEAD with exit 56).

use rayx_cli::host::{CommandSpec, Outcome, Runner};
use rayx_cli::release::{Existence, Release, exists, parse};

fn release(text: &str) -> Release {
    parse(text).expect("a release name")
}

/// What `curl -sIL -w "\nstatus=%{http_code}\n"` prints: the headers, then the final status.
fn status(code: u16) -> Outcome {
    Outcome::success().with_stdout(format!(
        "HTTP/2 {code}\r\ncontent-length: 9\r\n\r\n\nstatus={code}\n"
    ))
}

fn probe(outcome: Outcome) -> (Existence, Vec<CommandSpec>) {
    let mut runner = Runner::record().respond(|spec| spec.program == "curl", outcome);
    let found = exists(&mut runner, &release("1.0.1"));
    (found, runner.specs().to_vec())
}

fn unknown(outcome: Outcome) -> String {
    match probe(outcome).0 {
        Existence::Unknown(reason) => reason,
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn existence_asks_for_the_headers_of_the_installer_only() {
    // The program and its arguments, not the rendered line: the line quotes an argument with a
    // newline differently on Windows and elsewhere.
    let (_, specs) = probe(status(200));
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].program, "curl");
    assert_eq!(
        specs[0].args,
        [
            "--proto",
            "=https",
            "--tlsv1.2",
            "-sIL",
            "-w",
            "\nstatus=%{http_code}\n",
            "https://github.com/rayx-ui/RayX.CLI/releases/download/v1.0.1/rayx-cli-installer.sh"
        ]
    );
}

#[test]
fn the_probe_never_relies_on_curls_fail_flag() {
    let (_, specs) = probe(status(200));
    assert!(
        specs[0]
            .args
            .iter()
            .all(|arg| arg != "-f" && !arg.starts_with("-sf")),
        "{:?}",
        specs[0].args
    );
}

#[test]
fn a_published_release_exists() {
    assert_eq!(probe(status(200)).0, Existence::Exists);
    assert_eq!(probe(status(204)).0, Existence::Exists);
}

#[test]
fn an_http_404_or_410_is_a_missing_release() {
    assert_eq!(probe(status(404)).0, Existence::Missing);
    assert_eq!(probe(status(410)).0, Existence::Missing);
}

#[test]
fn a_404_is_missing_whatever_exit_code_curl_ends_with() {
    let answer = status(404);
    for code in [0, 22, 56] {
        let outcome = Outcome {
            code: Some(code),
            ..answer.clone()
        };
        assert_eq!(probe(outcome).0, Existence::Missing, "exit {code}");
    }
}

#[test]
fn another_http_status_is_unknown_and_not_a_missing_release() {
    for code in [301, 403, 429, 500, 503] {
        let reason = unknown(status(code));
        assert!(reason.contains(&code.to_string()), "{code}: {reason}");
    }
}

#[test]
fn another_http_status_is_reported_as_the_status_whatever_exit_code_curl_ends_with() {
    for exit in [0, 22, 56] {
        for code in [403, 503] {
            let outcome = Outcome {
                code: Some(exit),
                ..status(code)
            };
            let reason = unknown(outcome);
            assert!(
                reason.contains(&format!("HTTP {code}")),
                "exit {exit}, {code}: {reason}"
            );
        }
    }
}

#[test]
fn a_curl_failure_without_a_status_is_unknown_with_its_exit_code() {
    assert!(unknown(Outcome::failure(6)).contains("exited with 6"));
    let signalled = Outcome {
        code: None,
        ..Outcome::default()
    };
    assert!(unknown(signalled).contains("signal"));
}

#[test]
fn a_success_status_does_not_count_when_curl_itself_failed() {
    let answer = status(200);
    let outcome = Outcome {
        code: Some(56),
        ..answer
    };
    assert!(unknown(outcome).contains("56"));
}

#[test]
fn a_status_line_inside_the_headers_is_not_the_answer() {
    let outcome =
        Outcome::success().with_stdout("HTTP/2 404\r\nx-note: status=200\r\n\r\n\nstatus=404\n");
    assert_eq!(probe(outcome).0, Existence::Missing);
}
