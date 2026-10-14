//! `rayx self update` offers to refresh this machine's setup and guests with the new binary after
//! it installed a newer release, and only then.

use std::path::{Path, PathBuf};

use clap::Parser;
use rayx_cli::cli::{Cli, Command, SelfCommand};
use rayx_cli::update::{Backend, Refresher, run_with_refresh};
use semver::Version;

fn v(text: &str) -> Version {
    Version::parse(text).expect("version")
}

/// A backend whose executable path changes at install time, as Windows' renamed binary does.
struct Fake {
    latest: Version,
    install: Result<Version, String>,
    exe_before: PathBuf,
    installed: bool,
}

impl Fake {
    fn new(latest: &str) -> Self {
        Self {
            latest: v(latest),
            install: Ok(v(latest)),
            exe_before: PathBuf::from("/home/dev/.rayx/bin/rayx"),
            installed: false,
        }
    }
}

impl Backend for Fake {
    fn latest(&mut self) -> Result<Version, String> {
        Ok(self.latest.clone())
    }
    fn receipt_matches(&mut self) -> bool {
        true
    }
    fn executable(&self) -> PathBuf {
        if self.installed {
            PathBuf::from("/home/dev/.rayx/bin/rayx.old")
        } else {
            self.exe_before.clone()
        }
    }
    fn install(&mut self) -> Result<Version, String> {
        self.installed = true;
        self.install.clone()
    }
    fn release_exists(
        &mut self,
        _release: &rayx_cli::release::Release,
    ) -> rayx_cli::release::Existence {
        unreachable!("the plain update never asks for a named release")
    }
    fn install_release(
        &mut self,
        _release: &rayx_cli::release::Release,
    ) -> Result<Version, String> {
        unreachable!("the plain update never installs a named release")
    }
}

#[derive(Default)]
struct Spy {
    records: bool,
    /// `None`: no terminal.
    answer: Option<bool>,
    code: u8,
    questions: Vec<String>,
    refreshes: Vec<(PathBuf, bool)>,
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

fn run(
    check: bool,
    yes: bool,
    installed: &str,
    backend: &mut Fake,
    spy: &mut Spy,
) -> (u8, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_with_refresh(
        check,
        yes,
        &v(installed),
        backend,
        Some(spy),
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8(out).expect("UTF-8"),
        String::from_utf8(err).expect("UTF-8"),
    )
}

#[test]
fn yes_in_a_terminal_refreshes_with_the_new_binary_and_its_exit_code_is_ours() {
    let mut spy = Spy {
        records: true,
        answer: Some(true),
        code: 3,
        ..Spy::default()
    };

    let (code, out, _) = run(false, false, "1.0.0", &mut Fake::new("1.1.0"), &mut spy);

    assert_eq!(
        spy.questions,
        ["Refresh this machine's setup and guests with rayx 1.1.0?"]
    );
    assert_eq!(
        spy.refreshes,
        [(PathBuf::from("/home/dev/.rayx/bin/rayx"), false)]
    );
    assert_eq!(code, 3, "the refresh's exit code becomes the command's");
    assert!(out.contains("Updated rayx 1.0.0 -> 1.1.0."), "{out}");
}

#[test]
fn the_executable_is_the_one_captured_before_the_install() {
    let mut spy = Spy {
        records: true,
        answer: Some(true),
        ..Spy::default()
    };
    let mut backend = Fake::new("1.1.0");

    run(false, false, "1.0.0", &mut backend, &mut spy);

    assert!(backend.installed);
    assert_eq!(
        spy.refreshes[0].0,
        PathBuf::from("/home/dev/.rayx/bin/rayx"),
        "never the renamed old binary the backend reports after the install"
    );
}

#[test]
fn yes_refreshes_without_asking_and_passes_yes_on() {
    let mut spy = Spy {
        records: true,
        answer: Some(false),
        ..Spy::default()
    };

    let (code, _, _) = run(false, true, "1.0.0", &mut Fake::new("1.1.0"), &mut spy);

    assert!(spy.questions.is_empty());
    assert_eq!(spy.refreshes.len(), 1);
    assert!(spy.refreshes[0].1, "`--yes` is passed to `setup --update`");
    assert_eq!(code, 0);
}

#[test]
fn no_or_no_terminal_prints_the_command_and_exits_0() {
    for answer in [Some(false), None] {
        let mut spy = Spy {
            records: true,
            answer,
            code: 1,
            ..Spy::default()
        };

        let (code, out, _) = run(false, false, "1.0.0", &mut Fake::new("1.1.0"), &mut spy);

        assert_eq!(code, 0, "{answer:?}");
        assert!(spy.refreshes.is_empty());
        assert!(
            out.contains("run `rayx setup --update` to refresh the setup and guests"),
            "{out}"
        );
    }
}

#[test]
fn nothing_is_offered_without_records() {
    let mut spy = Spy {
        records: false,
        answer: Some(true),
        ..Spy::default()
    };

    let (code, out, _) = run(false, true, "1.0.0", &mut Fake::new("1.1.0"), &mut spy);

    assert_eq!(code, 0);
    assert!(spy.questions.is_empty() && spy.refreshes.is_empty());
    assert!(!out.contains("setup --update"), "{out}");
}

#[test]
fn check_up_to_date_and_a_failed_install_never_refresh() {
    let mut spy = Spy {
        records: true,
        answer: Some(true),
        ..Spy::default()
    };

    let (code, _, _) = run(true, false, "1.0.0", &mut Fake::new("1.1.0"), &mut spy);
    assert_eq!(code, 0);

    let (code, _, _) = run(false, true, "1.1.0", &mut Fake::new("1.1.0"), &mut spy);
    assert_eq!(code, 0, "already the latest");

    let mut failing = Fake::new("1.1.0");
    failing.install = Err("network".into());
    let (code, _, err) = run(false, true, "1.0.0", &mut failing, &mut spy);
    assert_eq!(code, 1);
    assert!(err.contains("the update failed"), "{err}");

    assert!(spy.questions.is_empty() && spy.refreshes.is_empty());
}

#[test]
fn the_command_line_takes_yes_but_not_with_check() {
    let parse = |args: &[&str]| {
        let mut argv = vec!["rayx", "self", "update"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv)
    };
    match parse(&["--yes"]).expect("--yes").command {
        Command::SelfCommand {
            command: SelfCommand::Update { check, yes, .. },
        } => {
            assert!(yes && !check);
        }
        other => panic!("{other:?}"),
    }
    assert!(parse(&["--check"]).is_ok());
    assert!(
        parse(&["--check", "--yes"]).is_err(),
        "nothing is installed by --check"
    );
}
