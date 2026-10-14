//! `rayx self update --release`: exactly the named release, the downgrade question, the receipt
//! rule, `--check`, and the refresh offer that follows.

use std::path::{Path, PathBuf};

use rayx_cli::release::{self, Existence, Release};
use rayx_cli::update::{Backend, Refresher, run_release, run_with_refresh};
use semver::Version;

fn v(text: &str) -> Version {
    Version::parse(text).expect("version")
}

fn r(text: &str) -> Release {
    release::parse(text).expect("release")
}

struct Fake {
    latest: Version,
    existence: Existence,
    receipt: bool,
    exe: PathBuf,
    installed: bool,
    installs: Vec<String>,
    plain_installs: usize,
    fail: Option<String>,
}

impl Fake {
    fn new() -> Self {
        Self {
            latest: v("1.2.0"),
            existence: Existence::Exists,
            receipt: true,
            exe: PathBuf::from("/home/dev/.rayx/bin/rayx"),
            installed: false,
            installs: Vec::new(),
            plain_installs: 0,
            fail: None,
        }
    }
}

impl Backend for Fake {
    fn latest(&mut self) -> Result<Version, String> {
        Ok(self.latest.clone())
    }
    fn receipt_matches(&mut self) -> bool {
        self.receipt
    }
    fn executable(&self) -> PathBuf {
        self.exe.clone()
    }
    fn install(&mut self) -> Result<Version, String> {
        self.plain_installs += 1;
        self.installed = true;
        Ok(self.latest.clone())
    }
    fn release_exists(&mut self, _release: &Release) -> Existence {
        self.existence.clone()
    }
    fn install_release(&mut self, release: &Release) -> Result<Version, String> {
        if let Some(error) = &self.fail {
            return Err(error.clone());
        }
        self.installs.push(release.to_string());
        self.installed = true;
        Ok(release.version().clone())
    }
}

struct Spy {
    records: bool,
    /// What a terminal would answer; `None` is no terminal.
    answer: Option<bool>,
    code: u8,
    questions: Vec<String>,
    refreshes: Vec<(PathBuf, bool)>,
}

impl Spy {
    fn new(answer: Option<bool>) -> Self {
        Self {
            records: true,
            answer,
            code: 0,
            questions: Vec::new(),
            refreshes: Vec::new(),
        }
    }
}

impl Refresher for Spy {
    fn records_exist(&mut self) -> bool {
        self.records
    }
    fn confirm(&mut self, question: &str) -> Option<bool> {
        self.questions.push(question.to_string());
        self.answer
    }
    fn refresh(&mut self, executable: &Path, yes: bool) -> u8 {
        self.refreshes.push((executable.to_path_buf(), yes));
        self.code
    }
}

struct Run {
    code: u8,
    out: String,
    err: String,
}

fn release_run(
    requested: &str,
    check: bool,
    yes: bool,
    installed: &str,
    fake: &mut Fake,
    spy: &mut Spy,
) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_release(
        &r(requested),
        check,
        yes,
        &v(installed),
        fake,
        Some(spy),
        &mut out,
        &mut err,
    );
    Run {
        code,
        out: String::from_utf8(out).expect("UTF-8"),
        err: String::from_utf8(err).expect("UTF-8"),
    }
}

#[test]
fn the_named_release_is_installed_over_an_older_one() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(false)));
    let run = release_run("1.1.0", false, false, "1.0.0", &mut fake, &mut spy);
    assert_eq!(run.code, 0, "{}", run.err);
    assert_eq!(fake.installs, ["1.1.0"], "exactly the requested release");
    assert_eq!(fake.plain_installs, 0);
    assert!(
        run.out.contains("Updated rayx 1.0.0 -> 1.1.0."),
        "{}",
        run.out
    );
    assert!(
        spy.questions.iter().all(|q| !q.contains("older")),
        "an upgrade asks no downgrade question"
    );
}

#[test]
fn a_release_equal_to_the_installed_one_changes_nothing() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(true)));
    let run = release_run("1.1.0", false, false, "1.1.0", &mut fake, &mut spy);
    assert_eq!(run.code, 0);
    assert!(
        run.out.contains("rayx 1.1.0 is already installed."),
        "{}",
        run.out
    );
    assert!(fake.installs.is_empty());
    assert!(spy.questions.is_empty() && spy.refreshes.is_empty());
}

#[test]
fn a_release_that_does_not_exist_exits_1_before_any_change() {
    let mut fake = Fake::new();
    fake.existence = Existence::Missing;
    let mut spy = Spy::new(Some(true));
    let run = release_run("9.9.9", false, true, "1.2.0", &mut fake, &mut spy);
    assert_eq!(run.code, 1);
    assert!(
        run.err.contains("release 9.9.9 was not found"),
        "{}",
        run.err
    );
    assert!(fake.installs.is_empty() && spy.questions.is_empty());
}

#[test]
fn an_unanswerable_existence_check_names_the_reason_and_changes_nothing() {
    let mut fake = Fake::new();
    fake.existence = Existence::Unknown("curl exited with 6".to_string());
    let mut spy = Spy::new(Some(true));
    let run = release_run("1.0.0", false, true, "1.2.0", &mut fake, &mut spy);
    assert_eq!(run.code, 1);
    assert!(
        run.err
            .contains("cannot check whether release 1.0.0 exists: curl exited with 6"),
        "{}",
        run.err
    );
    assert!(fake.installs.is_empty());
}

#[test]
fn a_rayx_without_an_install_receipt_is_never_replaced() {
    for exe in [
        "/work/RayX.CLI/target/debug/rayx",
        "/home/dev/.cargo/bin/rayx",
    ] {
        let mut fake = Fake::new();
        fake.receipt = false;
        fake.exe = PathBuf::from(exe);
        let mut spy = Spy::new(Some(true));
        let run = release_run("1.0.0", false, true, "1.2.0", &mut fake, &mut spy);
        assert_eq!(run.code, 1, "{exe}");
        assert!(run.err.contains("leaves it alone"), "{}", run.err);
        assert!(fake.installs.is_empty());
    }
}

#[test]
fn check_reports_installed_requested_and_direction_and_installs_nothing() {
    for (requested, installed, direction) in [
        ("1.0.0", "1.2.0", "downgrade"),
        ("1.2.0", "1.0.0", "upgrade"),
        ("1.1.0", "1.1.0", "same"),
    ] {
        let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(true)));
        let run = release_run(requested, true, false, installed, &mut fake, &mut spy);
        assert_eq!(run.code, 0);
        let line = format!("installed {installed}, requested release {requested}: {direction}");
        assert!(run.out.contains(&line), "{}", run.out);
        assert!(fake.installs.is_empty() && spy.questions.is_empty());
    }
}

#[test]
fn check_also_refuses_a_release_that_does_not_exist() {
    let mut fake = Fake::new();
    fake.existence = Existence::Missing;
    let mut spy = Spy::new(None);
    let run = release_run("9.9.9", true, false, "1.2.0", &mut fake, &mut spy);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("release 9.9.9 was not found"));
}

#[test]
fn a_downgrade_asks_and_yes_installs_it() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(true)));
    let run = release_run("1.1.0", false, false, "1.2.0", &mut fake, &mut spy);
    assert_eq!(run.code, 0, "{}", run.err);
    assert_eq!(
        spy.questions,
        ["Install rayx 1.1.0 over 1.2.0, an older release?"]
    );
    assert!(
        run.out.contains("Updated rayx 1.2.0 -> 1.1.0."),
        "{}",
        run.out
    );
    assert_eq!(fake.installs, ["1.1.0"]);
}

#[test]
fn a_downgrade_answered_no_exits_1_and_changes_nothing() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(false)));
    let run = release_run("1.1.0", false, false, "1.2.0", &mut fake, &mut spy);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("nothing was changed"), "{}", run.err);
    assert!(fake.installs.is_empty() && spy.refreshes.is_empty());
}

#[test]
fn yes_installs_a_downgrade_without_asking() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(None));
    let run = release_run("1.1.0", false, true, "1.2.0", &mut fake, &mut spy);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(spy.questions.is_empty());
    assert_eq!(fake.installs, ["1.1.0"]);
}

#[test]
fn a_downgrade_without_a_terminal_and_without_yes_is_refused() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(None));
    let run = release_run("1.1.0", false, false, "1.2.0", &mut fake, &mut spy);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("--yes"), "{}", run.err);
    assert!(fake.installs.is_empty());
}

#[test]
fn a_failed_install_exits_1_and_offers_no_refresh() {
    let mut fake = Fake::new();
    fake.fail = Some("the installer failed".to_string());
    let mut spy = Spy::new(Some(true));
    let run = release_run("1.2.0", false, true, "1.1.0", &mut fake, &mut spy);
    assert_eq!(run.code, 1);
    assert!(run.err.contains("the installer failed"), "{}", run.err);
    assert!(spy.questions.is_empty() && spy.refreshes.is_empty());
}

#[test]
fn an_upgrade_offers_the_refresh_and_runs_the_new_executable() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(true)));
    let run = release_run("1.2.0", false, false, "1.1.0", &mut fake, &mut spy);
    assert_eq!(run.code, 0, "{}", run.err);
    assert_eq!(
        spy.questions,
        ["Refresh this machine's setup and guests with rayx 1.2.0?"]
    );
    assert_eq!(
        spy.refreshes,
        [(PathBuf::from("/home/dev/.rayx/bin/rayx"), false)]
    );
}

#[test]
fn the_refresh_exit_code_becomes_the_commands() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(true)));
    spy.code = 3;
    let run = release_run("1.2.0", false, false, "1.1.0", &mut fake, &mut spy);
    assert_eq!(run.code, 3);
}

#[test]
fn yes_accepts_the_refresh_after_an_upgrade() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(None));
    let run = release_run("1.2.0", false, true, "1.1.0", &mut fake, &mut spy);
    assert_eq!(run.code, 0);
    assert!(spy.questions.is_empty());
    assert_eq!(spy.refreshes.len(), 1);
    assert!(spy.refreshes[0].1, "--yes is passed on to setup --update");
}

#[test]
fn a_downgrade_offers_no_refresh() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(true)));
    let run = release_run("1.1.0", false, true, "1.2.0", &mut fake, &mut spy);
    assert_eq!(run.code, 0);
    assert!(spy.questions.is_empty() && spy.refreshes.is_empty());
}

#[test]
fn a_release_before_the_refresh_says_so_and_offers_none() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(true)));
    let run = release_run("1.0.0", false, true, "1.1.0", &mut fake, &mut spy);
    assert_eq!(run.code, 0);
    assert!(
        run.out
            .contains("rayx 1.0.0 predates the guest refresh: no refresh was offered"),
        "{}",
        run.out
    );
    assert!(spy.questions.is_empty() && spy.refreshes.is_empty());
}

#[test]
fn an_upgrade_to_1_0_0_offers_no_refresh() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(true)));
    let run = release_run("1.0.0", false, true, "0.9.0", &mut fake, &mut spy);
    assert_eq!(run.code, 0);
    assert!(
        run.out.contains("predates the guest refresh"),
        "{}",
        run.out
    );
    assert!(spy.refreshes.is_empty());
}

#[test]
fn nothing_is_remembered_the_plain_update_goes_to_the_latest_release() {
    let (mut fake, mut spy) = (Fake::new(), Spy::new(Some(false)));
    let down = release_run("1.1.0", false, true, "1.2.0", &mut fake, &mut spy);
    assert_eq!(down.code, 0);

    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_with_refresh(
        false,
        false,
        &v("1.1.0"),
        &mut fake,
        Some(&mut spy),
        &mut out,
        &mut err,
    );
    assert_eq!(code, 0);
    assert_eq!(
        fake.plain_installs, 1,
        "the plain update installs the latest release"
    );
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .contains("Updated rayx 1.1.0 -> 1.2.0.")
    );
}
