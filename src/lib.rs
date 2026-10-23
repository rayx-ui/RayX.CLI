//! The `rayx` command-line tool: machine setup, doctor, WSL and Lima guests, self update, and any
//! Native Shell (NSH) app run through the nsh its project pins. The `rayx` binary only calls
//! [`run`].

pub mod cli;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};

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
        Command::Wsl { .. } => "wsl",
        Command::Lima { .. } => "lima",
        Command::SelfCommand { .. } => "self",
        Command::Fmt(_) => "fmt",
        Command::App(_) => "app",
        Command::Nsh { .. } => "nsh",
        Command::New(_) => "new",
        Command::Serve(_) => "serve",
    };
    not_available(name)
}

/// The answer of a command this build does not implement yet.
fn not_available(command: &str) -> ExitCode {
    eprintln!("rayx: `rayx {command}` is not available in this build yet");
    ExitCode::FAILURE
}
