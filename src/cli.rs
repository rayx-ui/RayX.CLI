//! Top-level command line: the commands, their flags, and nothing else.
//!
//! `app` keeps its own grammar (moved from RayX xtask), so it captures the raw trailing values and
//! leaves them to the app parser.

use clap::{ArgGroup, Args, Parser, Subcommand};

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
    /// Format the project's Rust sources, one workspace member at a time
    Fmt(FmtArgs),
}

/// Flags of `rayx setup`.
#[derive(Debug, Default, Args)]
#[command(group(ArgGroup::new("guest").args(["wsl", "lima"])))]
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
    /// Set up WSL 2 with the pinned Ubuntu and run the same setup inside it (Windows only)
    #[arg(long)]
    pub wsl: bool,
    /// Set up the Lima VM `rayx` with the pinned Ubuntu and run the same setup inside it (macOS only)
    #[arg(long)]
    pub lima: bool,
    /// Re-run setup for the sets and projects recorded on this machine, then refresh the guests
    #[arg(
        long,
        conflicts_with_all = [
            "web", "test", "android", "ios", "gpu", "all", "wsl", "lima", "clone", "vm_cpus",
            "vm_memory", "vm_disk", "vm_home_writable"
        ]
    )]
    pub update: bool,
    /// With --wsl or --lima: clone the current checkout into the guest (default `~/<checkout name>`)
    #[arg(
        long,
        requires = "guest",
        num_args = 0..=1,
        default_missing_value = "",
        value_name = "LINUX_DIR"
    )]
    pub clone: Option<String>,
    /// With --lima: CPUs of the VM when it is created (default 6, at most what this Mac has)
    #[arg(long, requires = "lima", conflicts_with = "wsl", value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub vm_cpus: Option<u32>,
    /// With --lima: memory of the VM in GiB when it is created (default 12, at most what this Mac can spare)
    #[arg(long, requires = "lima", conflicts_with = "wsl", value_name = "GIB", value_parser = clap::value_parser!(u32).range(1..))]
    pub vm_memory: Option<u32>,
    /// With --lima: disk of the VM in GiB when it is created (default 100, at most what this Mac can spare)
    #[arg(long, requires = "lima", conflicts_with = "wsl", value_name = "GIB", value_parser = clap::value_parser!(u32).range(1..))]
    pub vm_disk: Option<u32>,
    /// With --lima: mount the macOS home in the VM writable instead of read-only
    #[arg(long, requires = "lima", conflicts_with = "wsl")]
    pub vm_home_writable: bool,
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
    Status {
        /// Print the status as JSON
        #[arg(long)]
        json: bool,
    },
    /// Remove build output on request, trim the file system and compact the virtual disk
    Compact(CompactArgs),
}

/// Flags of `rayx wsl compact`.
#[derive(Debug, Default, Args)]
pub struct CompactArgs {
    /// The distribution to compact (default Ubuntu-24.04, else the first WSL 2 distribution)
    #[arg(long)]
    pub distro: Option<String>,
    /// First offer to delete the build output under the cloned checkouts
    #[arg(long)]
    pub clean: bool,
    /// Do not ask before deleting build output or shutting WSL down
    #[arg(long)]
    pub yes: bool,
}

/// Subcommands of `rayx self`.
#[derive(Debug, Subcommand)]
pub enum SelfCommand {
    /// Replace this rayx with the latest release
    Update {
        /// Report whether a newer release exists without installing it
        #[arg(long)]
        check: bool,
        /// Refresh this machine's setup and guests with the new rayx without asking
        #[arg(long, conflicts_with = "check")]
        yes: bool,
    },
}

/// Arguments of `rayx fmt`: `--check` may appear anywhere.
#[derive(Debug, Args)]
pub struct FmtArgs {
    /// `--check` reports unformatted members instead of formatting them
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS"
    )]
    pub args: Vec<String>,
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
