//! `rayx fmt [--check]`: `cargo fmt` for every member of the selected workspace, one package per
//! invocation. `cargo fmt --all` passes every source file of the workspace to one `rustfmt`
//! process, which exceeds the Windows command-line limit (`os error 206`), so the workspace is
//! formatted package by package instead. Moved from RayX xtask's `fmt`.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::host::{CommandSpec, Runner};
use crate::project::Project;

/// Runs `rayx fmt` in the current directory.
pub fn run(args: Vec<String>) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    run_in(&cwd, args, &mut Runner::execute())
}

/// Formats (or with `--check`, checks) every member of the workspace around `cwd` through
/// `runner`. `--check` may appear anywhere in `args`. Every member is attempted; the error lists
/// those that failed.
pub fn run_in(cwd: &Path, args: Vec<String>, runner: &mut Runner) -> Result<()> {
    let (check, rest): (Vec<String>, Vec<String>) =
        args.into_iter().partition(|arg| arg == "--check");
    if let Some(unknown) = rest.first() {
        bail!("unknown argument to fmt: {unknown} (the only option is --check)");
    }
    let check = !check.is_empty();

    // Discovery only reads, so it runs for real whatever mode the formatting is in.
    let project =
        Project::discover(&mut Runner::print(), cwd, None).map_err(|error| anyhow!("{error}"))?;
    let mut members: Vec<&str> = project
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    members.sort_unstable();
    members.dedup();
    if members.is_empty() {
        bail!(
            "{} lists no workspace packages",
            project.workspace_root.display()
        );
    }

    let mut failed = Vec::new();
    for member in members {
        let mut spec = CommandSpec::new("cargo")
            .args(["fmt", "-p", member])
            .cwd(&project.workspace_root)
            .interactive();
        if check {
            spec = spec.arg("--check");
        }
        match runner.run(&spec) {
            Ok(outcome) if outcome.is_success() => {}
            _ => failed.push(member),
        }
    }
    if !failed.is_empty() {
        bail!(
            "{} of the workspace packages {}: {}",
            failed.len(),
            if check {
                "are not formatted"
            } else {
                "could not be formatted"
            },
            failed.join(", ")
        );
    }
    Ok(())
}
