//! `rayx self update`: the version comparison, `--check`, install-source detection and the
//! instruction for builds the release installers did not make.

use std::path::{Path, PathBuf};

use rayx_cli::update::{Backend, InstallSource, Standing, classify, compare, run_with};
use semver::Version;

fn v(text: &str) -> Version {
    Version::parse(text).expect("version")
}

struct Fake {
    latest: Result<Version, String>,
    receipt: bool,
    exe: PathBuf,
    installed: Option<Version>,
    install_result: Result<Version, String>,
    installs: usize,
}

impl Fake {
    fn new(latest: &str, receipt: bool, exe: &str) -> Self {
        Self {
            latest: Ok(v(latest)),
            receipt,
            exe: PathBuf::from(exe),
            installed: None,
            install_result: Ok(v(latest)),
            installs: 0,
        }
    }
}

impl Backend for Fake {
    fn latest(&mut self) -> Result<Version, String> {
        self.latest.clone()
    }
    fn receipt_matches(&mut self) -> bool {
        self.receipt
    }
    fn executable(&self) -> PathBuf {
        self.exe.clone()
    }
    fn install(&mut self) -> Result<Version, String> {
        self.installs += 1;
        let result = self.install_result.clone();
        self.installed = result.clone().ok();
        result
    }
}

fn run(check: bool, installed: &str, backend: &mut Fake) -> (u8, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_with(check, &v(installed), backend, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).expect("UTF-8"),
        String::from_utf8(err).expect("UTF-8"),
    )
}

#[test]
fn versions_compare_by_semver_not_by_text() {
    assert_eq!(
        compare(&v("0.9.0"), &v("0.10.0")),
        Standing::UpdateAvailable
    );
    assert_eq!(compare(&v("1.2.3"), &v("1.2.3")), Standing::UpToDate);
    assert_eq!(compare(&v("2.0.0"), &v("1.9.9")), Standing::Ahead);
    assert_eq!(
        compare(&v("1.0.0-rc.1"), &v("1.0.0")),
        Standing::UpdateAvailable,
        "a prerelease is older than its release"
    );
}

#[test]
fn check_reports_both_versions_and_never_installs() {
    for (installed, latest, words) in [
        ("0.1.0", "0.2.0", "an update is available"),
        ("0.2.0", "0.2.0", "up to date"),
        ("0.3.0", "0.2.0", "newer than the latest release"),
    ] {
        let mut backend = Fake::new(latest, true, "/home/dev/.cargo/bin/rayx");
        let (code, out, err) = run(true, installed, &mut backend);

        assert_eq!(code, 0, "{err}");
        assert!(
            out.contains(&format!("installed {installed}, latest release {latest}")),
            "{out}"
        );
        assert!(out.contains(words), "{out}");
        assert_eq!(backend.installs, 0, "--check never installs");
    }
}

#[test]
fn check_works_for_a_rayx_the_installers_did_not_make() {
    let mut backend = Fake::new("0.2.0", false, "/work/RayX.CLI/target/release/rayx");

    let (code, out, _) = run(true, "0.1.0", &mut backend);

    assert_eq!(code, 0);
    assert!(out.contains("an update is available"), "{out}");
}

#[test]
fn an_installer_made_rayx_is_replaced_when_a_newer_release_exists() {
    let mut backend = Fake::new("0.2.0", true, "/home/dev/.cargo/bin/rayx");

    let (code, out, err) = run(false, "0.1.0", &mut backend);

    assert_eq!(code, 0, "{err}");
    assert_eq!(backend.installs, 1);
    assert_eq!(out.trim(), "Updated rayx 0.1.0 -> 0.2.0.");
}

#[test]
fn nothing_is_installed_when_the_release_is_not_newer() {
    for (installed, latest) in [("0.2.0", "0.2.0"), ("0.3.0", "0.2.0")] {
        let mut backend = Fake::new(latest, true, "/home/dev/.cargo/bin/rayx");
        let (code, out, _) = run(false, installed, &mut backend);

        assert_eq!(code, 0);
        assert!(out.contains("is the latest release"), "{out}");
        assert_eq!(backend.installs, 0);
    }
}

#[test]
fn a_rayx_without_a_receipt_is_told_how_to_update_and_left_alone() {
    for (exe, words) in [
        ("/work/RayX.CLI/target/release/rayx", "development build"),
        (r"C:\work\rayx\target\debug\rayx.exe", "development build"),
        ("/home/dev/.cargo/bin/rayx", "installed with cargo"),
        ("/opt/tools/rayx", "not installed by the release installers"),
    ] {
        let mut backend = Fake::new("0.2.0", false, exe);

        let (code, out, err) = run(false, "0.1.0", &mut backend);

        assert_eq!(code, 1, "{exe}");
        assert_eq!(backend.installs, 0, "{exe} is not overwritten");
        assert!(out.is_empty(), "{out}");
        assert!(err.contains(words), "{exe}: {err}");
        assert!(
            err.contains("rayx-cli-installer.sh") && err.contains("rayx-cli-installer.ps1"),
            "the install commands are named: {err}"
        );
    }
}

#[test]
fn the_install_source_follows_the_receipt_before_the_path() {
    let target = Path::new("/work/RayX.CLI/target/release/rayx");
    assert_eq!(classify(target, true), InstallSource::Installer);
    assert_eq!(classify(target, false), InstallSource::DevelopmentBuild);
    assert_eq!(
        classify(
            Path::new("/work/x/target/x86_64-pc-windows-msvc/release/rayx.exe"),
            false
        ),
        InstallSource::DevelopmentBuild
    );
    assert_eq!(
        classify(Path::new(r"C:\Users\dev\.cargo\bin\rayx.exe"), false),
        InstallSource::Cargo
    );
    assert_eq!(
        classify(Path::new("/usr/local/bin/rayx"), false),
        InstallSource::Unknown
    );
    assert!(InstallSource::Installer.instructions().is_empty());
}

#[test]
fn a_failure_to_reach_the_releases_or_to_install_is_reported_with_exit_code_1() {
    let mut offline = Fake::new("0.2.0", true, "/home/dev/.cargo/bin/rayx");
    offline.latest = Err("connection refused".into());
    let (code, _, err) = run(false, "0.1.0", &mut offline);
    assert_eq!(code, 1);
    assert!(
        err.contains("cannot find the latest release: connection refused"),
        "{err}"
    );

    let mut broken = Fake::new("0.2.0", true, "/home/dev/.cargo/bin/rayx");
    broken.install_result = Err("checksum mismatch".into());
    let (code, _, err) = run(false, "0.1.0", &mut broken);
    assert_eq!(code, 1);
    assert!(
        err.contains("the update failed: checksum mismatch"),
        "{err}"
    );
}
