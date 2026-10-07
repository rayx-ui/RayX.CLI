//! Top-level command line: the commands, their flags, and nothing else.
//!
//! `app` keeps its own grammar (moved from RayX xtask), so it captures the raw trailing values and
//! leaves them to the app parser.

use clap::{Args, Parser, Subcommand};

/// The `rayx` command line.
#[derive(Debug, Parser)]
#[command(
    name = "rayx",
    version,
    about = "Set up a machine for gpux and RayX work, and build, run, package and test RayX apps",
    arg_required_else_help = true,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// A top-level `rayx` command.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install everything gpux and RayX development needs on this machine
    Setup(SetupArgs),
    /// Report every requirement on this host with the exact fix
    Doctor(DoctorArgs),
    /// Show the WSL status and compact the WSL virtual disk
    Wsl {
        #[command(subcommand)]
        command: WslCommand,
    },
    /// Update rayx to the latest release (`rayx self update`)
    #[command(name = "self")]
    SelfCommand {
        #[command(subcommand)]
        command: SelfCommand,
    },
    /// Build, run, package, publish, deploy and test a RayX app
    App(AppArgs),
    /// Format the project's Rust sources
    Fmt,
}

/// Flags of `rayx setup`.
#[derive(Debug, Default, Args)]
pub struct SetupArgs {
    /// Also install the web set: wasm target, wasm-bindgen and the browser tooling
    #[arg(long)]
    pub web: bool,
    /// Also install the test tools: Playwright and its browsers
    #[arg(long)]
    pub test: bool,
    /// Also install the Android set: JDK, SDK, NDK, emulator and AVD
    #[arg(long)]
    pub android: bool,
    /// Also install the iOS set: Xcode, its command line tools and the Simulator runtime
    #[arg(long)]
    pub ios: bool,
    /// Also install the GPU set: GPU drivers and Vulkan tooling
    #[arg(long)]
    pub gpu: bool,
    /// Install every set this host supports
    #[arg(long)]
    pub all: bool,
    /// Report what is missing without installing anything
    #[arg(long)]
    pub check: bool,
    /// Accept licenses and agreements the installers ask for
    #[arg(long)]
    pub yes: bool,
    /// Set up WSL 2 with Ubuntu-24.04 and run the same setup inside it (Windows only)
    #[arg(long)]
    pub wsl: bool,
    /// With --wsl: clone the current checkout into the WSL home directory
    #[arg(long, requires = "wsl")]
    pub clone: bool,
}

/// Flags of `rayx doctor`.
#[derive(Debug, Default, Args)]
pub struct DoctorArgs {
    /// Print the report as JSON
    #[arg(long)]
    pub json: bool,
}

/// Subcommands of `rayx wsl`.
#[derive(Debug, Subcommand)]
pub enum WslCommand {
    /// Show the WSL distributions, their state and the disk space they use
    Status,
    /// Remove build output on request, trim the file system and compact the virtual disk
    Compact,
}

/// Subcommands of `rayx self`.
#[derive(Debug, Subcommand)]
pub enum SelfCommand {
    /// Replace this rayx with the latest release
    Update {
        /// Report whether a newer release exists without installing it
        #[arg(long)]
        check: bool,
    },
}

/// Arguments of `rayx app`: the app grammar `[<dir>] build|run|pack|publish|deploy|test|assets
/// <target> [options]`, parsed by the app module.
#[derive(Debug, Args)]
pub struct AppArgs {
    /// The app directory, the action, the target and the options, passed through unchanged
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS"
    )]
    pub args: Vec<String>,
}
