//! `release`: what a release is called, how releases compare, where a release's installer lives
//! and the floors: 1.0.0 is the first release of this rayx, and every release has the refresh
//! (whether a release is published is `tests/release_probe.rs`).

use rayx_cli::release::{
    Direction, MIN_REFRESH_RELEASE, OLDEST_RELEASE, Release, installer_url_for, parse,
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
fn every_release_of_this_rayx_has_the_refresh() {
    assert_eq!(MIN_REFRESH_RELEASE.to_string(), "1.0.0");
    assert_eq!(MIN_REFRESH_RELEASE, OLDEST_RELEASE);
    assert!(release("1.0.0") >= MIN_REFRESH_RELEASE);
    assert!(release("1.1.0") >= MIN_REFRESH_RELEASE);
}

#[test]
fn a_release_older_than_1_0_0_cannot_be_named() {
    assert_eq!(OLDEST_RELEASE.to_string(), "1.0.0");
    assert_eq!(release("1.0.0"), OLDEST_RELEASE);
    for text in ["0.9.0", "v0.1.2", "0.99.99"] {
        let error = parse(text).expect_err(text).to_string();
        assert!(error.contains(&format!("`{text}`")), "{error}");
        assert!(
            error.contains("older than 1.0.0, the first release of this rayx"),
            "{error}"
        );
        assert!(error.contains("rayx-ui/RayX.CLI.Old"), "{error}");
    }
}
