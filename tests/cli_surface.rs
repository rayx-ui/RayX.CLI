//! Top-level command surface: help, version, usage errors and pass-through of `app` arguments.

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use clap::Parser;
use predicates::prelude::*;
use predicates::str::contains;
use rayx_cli::cli::{Cli, Command as Subcommand};

fn rayx() -> Command {
    cargo_bin_cmd!("rayx")
}

#[test]
fn help_lists_every_command_with_one_line() {
    let output = rayx().arg("--help").assert().success().get_output().clone();
    let help = String::from_utf8(output.stdout).expect("help is UTF-8");

    let entries: Vec<&str> = help
        .lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .collect();

    for command in ["setup", "doctor", "wsl", "self", "app", "fmt"] {
        let line = entries
            .iter()
            .find(|line| line.trim_start().starts_with(command))
            .unwrap_or_else(|| panic!("`{command}` is listed under Commands:\n{help}"));
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
fn version_prints_the_package_version() {
    let expected = format!("rayx {}", env!("CARGO_PKG_VERSION"));
    rayx()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::diff(format!("{expected}\n")));
}

#[test]
fn unknown_command_exits_2_and_names_the_problem() {
    rayx()
        .arg("frobnicate")
        .assert()
        .code(2)
        .stderr(contains("frobnicate").and(contains("Usage:")));
}

#[test]
fn unknown_flag_exits_2_and_names_the_problem() {
    rayx()
        .args(["setup", "--no-such-flag"])
        .assert()
        .code(2)
        .stderr(contains("--no-such-flag").and(contains("Usage:")));
}

#[test]
fn no_command_prints_usage_and_exits_2() {
    rayx().assert().code(2).stderr(contains("Usage:"));
}

#[test]
fn setup_accepts_the_documented_flags() {
    let cli = Cli::try_parse_from([
        "rayx",
        "setup",
        "--web",
        "--test",
        "--android",
        "--ios",
        "--gpu",
        "--all",
        "--check",
        "--yes",
    ])
    .expect("setup flags parse");
    let Subcommand::Setup(setup) = cli.command else {
        panic!("expected setup");
    };
    assert!(setup.web && setup.test && setup.android && setup.ios && setup.gpu);
    assert!(setup.all && setup.check && setup.yes);
    assert!(!setup.wsl && !setup.clone);
}

#[test]
fn clone_requires_wsl() {
    rayx()
        .args(["setup", "--clone"])
        .assert()
        .code(2)
        .stderr(contains("--wsl"));
}

#[test]
fn doctor_wsl_and_self_subcommands_parse() {
    for args in [
        &["rayx", "doctor", "--json"][..],
        &["rayx", "wsl", "status"][..],
        &["rayx", "wsl", "compact"][..],
        &["rayx", "self", "update"][..],
        &["rayx", "self", "update", "--check"][..],
        &["rayx", "fmt"][..],
    ] {
        Cli::try_parse_from(args).unwrap_or_else(|e| panic!("`{}` parses: {e}", args.join(" ")));
    }
}

#[test]
fn app_captures_raw_trailing_arguments_unchanged() {
    let cli = Cli::try_parse_from([
        "rayx",
        "app",
        "apps/lab",
        "test",
        "wasm",
        "--headed",
        "--no-default-features",
        "--features",
        "rayx_diagnostics",
        "--playwright-test",
        "a.spec.ts",
    ])
    .expect("app arguments parse");
    let Subcommand::App(app) = cli.command else {
        panic!("expected app");
    };
    assert_eq!(
        app.args,
        [
            "apps/lab",
            "test",
            "wasm",
            "--headed",
            "--no-default-features",
            "--features",
            "rayx_diagnostics",
            "--playwright-test",
            "a.spec.ts"
        ]
    );
}
