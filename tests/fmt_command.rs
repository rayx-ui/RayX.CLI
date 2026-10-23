//! `rayx fmt [--check]`: one `cargo fmt -p <member>` per workspace member, with a list of the
//! members that fail.

mod common;

use common::Workspace;
use rayx_cli::fmt::run_in;
use rayx_cli::host::{Outcome, Runner};

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(ToString::to_string).collect()
}

/// The `cargo fmt` command lines a recording runner saw.
fn commands(runner: &Runner) -> Vec<Vec<String>> {
    runner
        .specs()
        .iter()
        .map(|spec| {
            let mut line = vec![spec.program.clone()];
            line.extend(spec.args.clone());
            line
        })
        .collect()
}

#[test]
fn every_member_is_formatted_by_its_own_command() {
    let workspace = Workspace::fixture("fmt-workspace");
    let mut runner = Runner::record();

    run_in(&workspace.root, Vec::new(), &mut runner).expect("formats");

    assert_eq!(
        commands(&runner),
        [
            ["cargo", "fmt", "-p", "fmt_alpha"],
            ["cargo", "fmt", "-p", "fmt_beta"]
        ]
    );
    let cwd = runner.specs()[0].cwd.as_ref().expect("a working directory");
    assert_eq!(
        std::fs::canonicalize(cwd).expect("canonical"),
        std::fs::canonicalize(&workspace.root).expect("canonical")
    );
}

#[test]
fn check_may_appear_anywhere_in_the_arguments() {
    let workspace = Workspace::fixture("fmt-workspace");
    let mut runner = Runner::record();

    run_in(&workspace.root, args(&["--check"]), &mut runner).expect("checks");

    assert_eq!(
        commands(&runner),
        [
            ["cargo", "fmt", "-p", "fmt_alpha", "--check"],
            ["cargo", "fmt", "-p", "fmt_beta", "--check"]
        ]
    );
}

#[test]
fn the_members_that_fail_are_listed_after_every_member_ran() {
    let workspace = Workspace::fixture("fmt-workspace");
    let mut runner = Runner::record().responder(|spec| {
        spec.args
            .iter()
            .any(|arg| arg == "fmt_alpha")
            .then(|| Outcome::failure(1))
    });

    let error = run_in(&workspace.root, args(&["--check"]), &mut runner).expect_err("fails");

    assert_eq!(
        error.to_string(),
        "1 of the workspace packages are not formatted: fmt_alpha"
    );
    assert_eq!(
        commands(&runner).len(),
        2,
        "the second member was still checked"
    );
}

#[test]
fn a_formatting_failure_without_check_says_it_could_not_format() {
    let workspace = Workspace::fixture("fmt-workspace");
    let mut runner = Runner::record().responder(|_| Some(Outcome::failure(1)));

    let error = run_in(&workspace.root, Vec::new(), &mut runner).expect_err("fails");

    assert_eq!(
        error.to_string(),
        "2 of the workspace packages could not be formatted: fmt_alpha, fmt_beta"
    );
}

#[test]
fn unknown_arguments_are_refused_before_anything_runs() {
    let workspace = Workspace::fixture("fmt-workspace");
    let mut runner = Runner::record();

    let error = run_in(&workspace.root, args(&["--all"]), &mut runner).expect_err("refused");

    assert!(error.to_string().contains("--all"), "{error:#}");
    assert!(runner.specs().is_empty());
}

#[test]
fn a_directory_inside_a_member_formats_the_whole_workspace() {
    let workspace = Workspace::fixture("fmt-workspace");
    let mut runner = Runner::record();

    run_in(&workspace.path("beta/src"), Vec::new(), &mut runner).expect("formats");

    assert_eq!(commands(&runner).len(), 2);
}

#[test]
fn cargo_fmt_really_formats_and_checks_a_member() {
    let workspace = Workspace::fixture("fmt-workspace");
    workspace.write("beta/src/lib.rs", "pub fn   beta( )->u32{1}\n");

    let error = run_in(&workspace.root, args(&["--check"]), &mut Runner::execute())
        .expect_err("beta is not formatted");
    assert_eq!(
        error.to_string(),
        "1 of the workspace packages are not formatted: fmt_beta"
    );

    run_in(&workspace.root, Vec::new(), &mut Runner::execute()).expect("formats");
    assert_eq!(
        workspace.read("beta/src/lib.rs"),
        "pub fn beta() -> u32 {\n    1\n}\n"
    );
    run_in(&workspace.root, args(&["--check"]), &mut Runner::execute()).expect("now formatted");
}
