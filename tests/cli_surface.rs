//! Top-level command surface: the ten commands, help, version, usage errors, and how `app` hands
//! nsh's arguments over unchanged.

use std::ffi::OsString;
use std::path::PathBuf;

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use clap::Parser;
use predicates::prelude::*;
use predicates::str::contains;
use rayx_cli::cli::{Cli, Command as Subcommand, NshCommand, Template};

const COMMANDS: [&str; 10] = [
    "setup", "doctor", "wsl", "lima", "self", "fmt", "app", "nsh", "new", "serve",
];

fn rayx() -> Command {
    cargo_bin_cmd!("rayx")
}

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(args).unwrap_or_else(|e| panic!("`{}` parses: {e}", args.join(" ")))
}

fn os(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn help_lists_exactly_the_ten_commands_with_one_line_each() {
    let output = rayx().arg("--help").assert().success().get_output().clone();
    let help = String::from_utf8(output.stdout).expect("help is UTF-8");

    let entries: Vec<&str> = help
        .lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .collect();
    let names: Vec<&str> = entries
        .iter()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert_eq!(names, COMMANDS, "help lists exactly the commands:\n{help}");
    for (line, command) in entries.iter().zip(COMMANDS) {
        assert!(
            line.trim_start()[command.len()..].trim().len() > 3,
            "`{command}` carries a one-line description: {line:?}"
        );
    }
    assert!(
        help.contains("rayx self update"),
        "`self update` is named in the help:\n{help}"
    );
}

#[test]
fn version_prints_rayx_1_0_0() {
    rayx()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::diff("rayx 1.0.0\n"));
    assert_eq!(env!("CARGO_PKG_VERSION"), "1.0.0");
}

#[test]
fn unknown_command_exits_2_and_names_the_problem() {
    for command in ["frobnicate", "help", "build"] {
        rayx()
            .arg(command)
            .assert()
            .code(2)
            .stderr(contains(command).and(contains("Usage:")));
    }
}

#[test]
fn unknown_flag_exits_2_and_names_the_problem() {
    for args in [
        &["setup", "--no-such-flag"][..],
        &["--no-such-flag"][..],
        &["nsh", "status", "--no-such-flag"][..],
    ] {
        rayx()
            .args(args)
            .assert()
            .code(2)
            .stderr(contains("--no-such-flag").and(contains("Usage:")));
    }
}

#[test]
fn no_command_prints_usage_and_exits_2() {
    rayx().assert().code(2).stderr(contains("Usage:"));
}

#[test]
fn setup_accepts_the_documented_flags() {
    let cli = parse(&[
        "rayx",
        "setup",
        "--web",
        "--test",
        "--android",
        "--ios",
        "--all",
        "--check",
        "--yes",
    ]);
    let Subcommand::Setup(setup) = cli.command else {
        panic!("expected setup");
    };
    assert!(setup.web && setup.test && setup.android && setup.ios);
    assert!(setup.all && setup.check && setup.yes);
    assert!(!setup.wsl && setup.clone.is_none());
}

#[test]
fn clone_requires_a_guest() {
    rayx()
        .args(["setup", "--clone"])
        .assert()
        .code(2)
        .stderr(contains("--wsl"));
}

#[test]
fn every_subcommand_parses() {
    for args in [
        &["rayx", "doctor", "--json"][..],
        &["rayx", "wsl", "status", "--json"][..],
        &["rayx", "wsl", "compact", "--clean", "--yes"][..],
        &["rayx", "wsl", "remove", "Ubuntu-24.04"][..],
        &["rayx", "lima", "remove", "rayx", "--yes"][..],
        &["rayx", "self", "update"][..],
        &["rayx", "self", "update", "--check"][..],
        &["rayx", "self", "update", "--release", "1.0.0", "--yes"][..],
        &["rayx", "fmt", "--check"][..],
        &["rayx", "app", "run"][..],
        &["rayx", "nsh", "status", "--json"][..],
        &["rayx", "nsh", "path", "--no-install"][..],
        &["rayx", "nsh", "install", "-C", "app"][..],
        &["rayx", "nsh", "prune", "--yes"][..],
        &["rayx", "new", "hello"][..],
        &["rayx", "serve", "--dir", "site", "--port", "8081"][..],
    ] {
        parse(args);
    }
}

#[test]
fn app_hands_everything_after_the_command_to_nsh_unchanged() {
    let cli = parse(&[
        "rayx",
        "app",
        "run",
        ".",
        "--target",
        "web",
        "--strict-port",
        "--port",
        "4000",
        "-C",
        "elsewhere",
        "--no-install",
        "--",
        "--app-flag",
        "value",
    ]);
    let Subcommand::App(app) = cli.command else {
        panic!("expected app");
    };
    assert_eq!(app.command(), "run");
    assert_eq!(
        app.args(),
        os(&[
            ".",
            "--target",
            "web",
            "--strict-port",
            "--port",
            "4000",
            "-C",
            "elsewhere",
            "--no-install",
            "--",
            "--app-flag",
            "value"
        ]),
        "rayx's own options after the command and `--` belong to nsh"
    );
    assert_eq!(app.dir, None);
    assert!(!app.no_install);
}

#[test]
fn app_takes_its_own_options_before_the_command() {
    let cli = parse(&[
        "rayx",
        "app",
        "-C",
        "apps/hello",
        "--no-install",
        "test",
        "--target",
        "web",
    ]);
    let Subcommand::App(app) = cli.command else {
        panic!("expected app");
    };
    assert_eq!(app.dir, Some(PathBuf::from("apps/hello")));
    assert!(app.no_install);
    assert_eq!(app.command(), "test");
    assert_eq!(app.args(), os(&["--target", "web"]));

    let cli = parse(&["rayx", "app", "build", "--", "--release"]);
    let Subcommand::App(app) = cli.command else {
        panic!("expected app");
    };
    assert_eq!(app.args(), os(&["--", "--release"]));
}

#[test]
fn app_needs_a_command() {
    rayx()
        .args(["app"])
        .assert()
        .code(2)
        .stderr(contains("Usage: rayx app"));
}

#[test]
fn new_takes_nsh_s_three_templates_and_defaults_to_mixed() {
    let cli = parse(&["rayx", "new", "hello"]);
    let Subcommand::New(new) = cli.command else {
        panic!("expected new");
    };
    assert_eq!(new.template, Template::Mixed);
    assert_eq!(new.dir, PathBuf::from("hello"));
    assert_eq!(new.name, None);

    for (name, template) in [
        ("mixed", Template::Mixed),
        ("ts", Template::Ts),
        ("rust", Template::Rust),
    ] {
        let cli = parse(&["rayx", "new", "app", "--name", "Hello", "--template", name]);
        let Subcommand::New(new) = cli.command else {
            panic!("expected new");
        };
        assert_eq!(new.template, template);
        assert_eq!(new.template.name(), name);
        assert_eq!(new.name.as_deref(), Some("Hello"));
    }

    rayx()
        .args(["new", "app", "--template", "rayx"])
        .assert()
        .code(2)
        .stderr(contains("mixed").and(contains("ts")).and(contains("rust")));
}

#[test]
fn nsh_subcommands_take_their_options() {
    let Subcommand::Nsh {
        command: NshCommand::Path { dir, no_install },
    } = parse(&["rayx", "nsh", "path", "-C", "app", "--no-install"]).command
    else {
        panic!("expected nsh path");
    };
    assert_eq!(dir, Some(PathBuf::from("app")));
    assert!(no_install);

    rayx()
        .args(["nsh"])
        .assert()
        .code(2)
        .stderr(contains("status").and(contains("prune")));
}

#[test]
fn serve_takes_a_directory_and_a_port() {
    let Subcommand::Serve(serve) = parse(&["rayx", "serve"]).command else {
        panic!("expected serve");
    };
    assert_eq!((serve.dir, serve.port), (None, None));
    rayx()
        .args(["serve", "--port", "not-a-port"])
        .assert()
        .code(2)
        .stderr(contains("--port"));
}
