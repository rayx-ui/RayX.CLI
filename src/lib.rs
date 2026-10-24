//! The `rayx` command-line tool: machine setup, doctor, WSL and Lima guests, self update, and any
//! Native Shell (NSH) app run through the nsh its project pins. The `rayx` binary only calls
//! [`run`].

pub mod cli;
pub mod doctor;
pub mod fmt;
pub mod host;
pub mod nsh;
pub mod project;
pub mod release;
pub mod setup;
pub mod update;
pub mod wsl;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command, LimaCommand, SelfCommand, WslCommand};

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
        Command::Setup(args) => return ExitCode::from(setup::run(args)),
        Command::Fmt(args) => return run_fmt(args.args.clone()),
        Command::Doctor(args) => return ExitCode::from(doctor::run(args)),
        Command::Wsl {
            command: WslCommand::Status { json },
        } => return ExitCode::from(wsl::run_status(*json)),
        Command::Wsl {
            command: WslCommand::Compact(args),
        } => return ExitCode::from(wsl::run_compact(args)),
        Command::Wsl {
            command: WslCommand::Remove(args),
        } => {
            return ExitCode::from(setup::remove::run(setup::guest::GuestKind::Wsl, args));
        }
        Command::Lima {
            command: LimaCommand::Remove(args),
        } => {
            return ExitCode::from(setup::remove::run(setup::guest::GuestKind::Lima, args));
        }
        Command::SelfCommand {
            command:
                SelfCommand::Update {
                    check,
                    release,
                    yes,
                },
        } => return ExitCode::from(update::run(*check, *yes, release.as_deref())),
        Command::App(_) => "app",
        Command::Nsh { command } => return ExitCode::from(nsh::run(command)),
        Command::New(_) => "new",
        Command::Serve(_) => "serve",
    };
    not_available(name)
}

/// Runs `rayx fmt` and turns its error into a message and an exit code.
fn run_fmt(args: Vec<String>) -> ExitCode {
    match fmt::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rayx: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The answer of a command this build does not implement yet.
fn not_available(command: &str) -> ExitCode {
    eprintln!("rayx: `rayx {command}` is not available in this build yet");
    ExitCode::FAILURE
}
