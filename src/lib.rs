//! The `rayx` command-line tool: machine setup, doctor, WSL, self update and the app pipeline for
//! any app that uses RayX. The `rayx` binary only calls [`run`].

pub mod cli;
pub mod host;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, SelfCommand, WslCommand};

/// Runs the command line of this process and returns the process exit code.
pub fn run() -> ExitCode {
    run_from(std::env::args_os())
}

/// Runs the command line `args`, whose first item is the program name.
///
/// A usage error prints the problem and the usage to standard error and returns exit code 2;
/// `--help` and `--version` print to standard output and return success.
pub fn run_from<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            // A failure to print (a closed pipe) leaves nothing more to report.
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };
    dispatch(cli)
}

fn dispatch(cli: Cli) -> ExitCode {
    let name = match &cli.command {
        Command::Setup(_) => "setup",
        Command::Doctor(_) => "doctor",
        Command::Wsl {
            command: WslCommand::Status,
        } => "wsl status",
        Command::Wsl {
            command: WslCommand::Compact,
        } => "wsl compact",
        Command::SelfCommand {
            command: SelfCommand::Update { .. },
        } => "self update",
        Command::App(_) => "app",
        Command::Fmt => "fmt",
    };
    not_implemented(name)
}

fn not_implemented(command: &str) -> ExitCode {
    eprintln!("rayx: `{command}` is not implemented in this build");
    ExitCode::FAILURE
}
