//! `release`: what a release is called, how releases compare, where a release's installer lives
//! and whether a release is published.

use rayx_cli::host::{Outcome, Runner};
use rayx_cli::release::{
    Direction, Existence, MIN_REFRESH_RELEASE, Release, exists, installer_url_for, parse,
};
use semver::Version;

fn release(text: &str) -> Release {
    parse(text).expect("a release name")
}

#[test]
fn a_release_name_is_x_y_z_with_an_optional_leading_v() {
    assert_eq!(release("1.0.1").to_string(), "1.0.1");
    assert_eq!(release("v1.2.3").to_string(), "1.2.3");
    assert_eq!(release("10.20.30"), release("v10.20.30"));
}

#[test]
fn everything_else_is_refused_with_an_example() {
    for text in [
        "",
        "latest",
        "1",
        "1.0",
        "1.0.0-rc.1",
        "1.0.0+build.5",
        "^1.0.0",
        ">=1.0.0",
        "vv1.0.0",
        "1.0.0 ",
        "1.x.0",
    ] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains("1.0.1"), "{text:?}: {error}");
        assert!(error.contains(&format!("`{text}`")), "{text:?}: {error}");
    }
}

#[test]
fn releases_order_by_number_not_by_text() {
    assert!(release("1.10.0") > release("1.9.0"));
    assert!(release("2.0.0") > release("1.99.99"));
    assert_eq!(release("1.0.0"), release("v1.0.0"));
}

#[test]
fn a_requested_release_is_an_upgrade_a_downgrade_or_the_same() {
    let installed = Version::parse("1.1.0").expect("version");
    assert_eq!(
        release("1.2.0").direction_from(&installed),
        Direction::Upgrade
    );
    assert_eq!(release("1.1.0").direction_from(&installed), Direction::Same);
    assert_eq!(
        release("1.0.0").direction_from(&installed),
        Direction::Downgrade
    );
    assert_eq!(Direction::Downgrade.word(), "downgrade");
    assert_eq!(Direction::Upgrade.word(), "upgrade");
    assert_eq!(Direction::Same.word(), "same");
}

#[test]
fn a_development_version_has_no_release_name() {
    assert!(Release::from_version(&Version::parse("1.2.0").expect("version")).is_some());
    assert!(Release::from_version(&Version::parse("1.2.0-dev").expect("version")).is_none());
    assert!(Release::from_version(&Version::parse("1.2.0+local").expect("version")).is_none());
}

#[test]
fn the_installer_lives_under_the_release_tag() {
    assert_eq!(
        release("1.0.1").installer_url(),
        "https://github.com/rayx-ui/RayX.CLI/releases/download/v1.0.1/rayx-cli-installer.sh"
    );
    assert_eq!(
        installer_url_for("1.1.0"),
        release("1.1.0").installer_url(),
        "the guest installer and `--release` build the same URL"
    );
}

#[test]
fn the_refresh_starts_with_1_1_0() {
    assert_eq!(MIN_REFRESH_RELEASE.to_string(), "1.1.0");
    assert!(release("1.0.0") < MIN_REFRESH_RELEASE);
    assert!(release("1.1.0") >= MIN_REFRESH_RELEASE);
}

/// What `curl -sIL -w "\nstatus=%{http_code}\n"` prints: the headers, then the final status.
fn status(code: u16) -> Outcome {
    Outcome::success().with_stdout(format!(
        "HTTP/2 {code}\r\ncontent-length: 9\r\n\r\n\nstatus={code}\n"
    ))
}

fn probe(outcome: Outcome) -> (Existence, Vec<String>) {
    let mut runner = Runner::record().respond(|spec| spec.program == "curl", outcome);
    let found = exists(&mut runner, &release("1.0.1"));
    (found, runner.lines().to_vec())
}

#[test]
fn existence_asks_for_the_headers_of_the_installer_only() {
    let (_, lines) = probe(status(200));
    assert_eq!(
        lines,
        [
            "curl --proto =https --tlsv1.2 -sIL -w '\nstatus=%{http_code}\n' \
             https://github.com/rayx-ui/RayX.CLI/releases/download/v1.0.1/rayx-cli-installer.sh"
        ]
    );
}

#[test]
fn a_published_release_exists() {
    assert_eq!(probe(status(200)).0, Existence::Exists);
}

#[test]
fn an_http_404_is_a_missing_release() {
    assert_eq!(probe(status(404)).0, Existence::Missing);
    assert_eq!(probe(status(410)).0, Existence::Missing);
}

#[test]
fn a_404_is_missing_whatever_exit_code_curl_ends_with() {
    // macOS curl 8.7.1 ends a HEAD request answered with a 404 and a body length with exit 56.
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
    for code in [403, 429, 500, 503] {
        match probe(status(code)).0 {
            Existence::Unknown(reason) => assert!(reason.contains(&code.to_string()), "{reason}"),
            other => panic!("HTTP {code}: expected Unknown, got {other:?}"),
        }
    }
}

#[test]
fn any_other_failure_is_unknown_with_its_reason() {
    match probe(Outcome::failure(6)).0 {
        Existence::Unknown(reason) => assert!(reason.contains('6'), "{reason}"),
        other => panic!("expected Unknown, got {other:?}"),
    }
}
