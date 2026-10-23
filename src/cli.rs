//! Top-level command line: the commands, their flags, and nothing else.
//!
//! `app` passes nsh's own grammar through unchanged (D12): clap reads rayx's options before the
//! nsh command and captures everything after it raw, `--` included.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};

/// The `rayx` command line.
#[derive(Debug, Parser)]
#[command(
    name = "rayx",
    version,
    about = "Set up a machine for Native Shell (NSH) work, and run any NSH app through the nsh it pins",
    arg_required_else_help = true,
    disable_help_subcommand = true,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// A top-level `rayx` command.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install everything NSH development needs on this machine, including the project's nsh
    Setup(SetupArgs),
    /// Report every requirement on this host with the exact fix
    Doctor(DoctorArgs),
    /// Show the WSL status, compact the WSL virtual disk and remove WSL guests (Windows)
    Wsl {
        #[command(subcommand)]
        command: WslCommand,
    },
    /// Remove Lima VMs rayx created (macOS)
    Lima {
        #[command(subcommand)]
        command: LimaCommand,
    },
    /// Update rayx to the latest release (`rayx self update`)
    #[command(name = "self")]
    SelfCommand {
        #[command(subcommand)]
        command: SelfCommand,
    },
    /// Format the project's Rust sources, one workspace member at a time
    Fmt(FmtArgs),
    /// Run an nsh command with the project's pinned nsh (`rayx app run --target web`)
    App(AppArgs),
    /// Show, install, locate and prune the nsh builds rayx keeps for each pin
    Nsh {
        #[command(subcommand)]
        command: NshCommand,
    },
    /// Create a new NSH app pinned to rayx's default nsh
    New(NewArgs),
    /// Serve a packaged web app (`nsh package --target web`) on 127.0.0.1
    Serve(ServeArgs),
}

/// Flags of `rayx setup`.
#[derive(Debug, Default, Args)]
#[command(
    group(ArgGroup::new("guest").args(["wsl", "lima"])),
    group(ArgGroup::new("vm_target").args(["lima", "update"]).multiple(true)),
    group(ArgGroup::new("release_target").args(["wsl", "lima", "update"]).multiple(true))
)]
pub struct SetupArgs {
    /// Also install the web set: the wasm32 target, wasm-bindgen when the project locks it, and LLVM
    #[arg(long)]
    pub web: bool,
    /// Also install the test tools: Node, pnpm, the project's Playwright and its Chromium
    #[arg(long)]
    pub test: bool,
    /// Also install the Android set: JDK, SDK, NDK, emulator and AVD
    #[arg(long)]
    pub android: bool,
    /// Also install the iOS set: Xcode, its command line tools and the Simulator runtime
    #[arg(long)]
    pub ios: bool,
    /// Install every set this host supports
    #[arg(long)]
    pub all: bool,
    /// Report what is missing without installing anything
    #[arg(long)]
    pub check: bool,
    /// Accept licenses and agreements the installers ask for, update outdated tools and guests without asking
    #[arg(long)]
    pub yes: bool,
    /// Set up WSL 2 with the pinned Ubuntu and run the same setup inside it (Windows only)
    #[arg(long)]
    pub wsl: bool,
    /// Set up a Lima VM (`rayx`, or the name --vm gives) with the pinned Ubuntu and run the same setup inside it (macOS only; never part of --all)
    #[arg(long)]
    pub lima: bool,
    /// Re-run setup for the sets and projects recorded on this machine, then refresh every current guest (apt upgrade and setup --update inside)
    #[arg(
        long,
        conflicts_with_all = [
            "web", "test", "android", "ios", "all", "wsl", "lima", "clone", "vm_cpus",
            "vm_memory", "vm_disk", "vm_home_writable"
        ]
    )]
    pub update: bool,
    /// With --lima, --wsl or --update: put exactly this release (X.Y.Z, for example 1.0.1, also an older one) of the Linux rayx in the guest instead of this rayx's version; it is not remembered
    #[arg(long, requires = "release_target", value_name = "RELEASE")]
    pub release: Option<String>,
    /// With --lima or --update: the Lima VM by name (default `rayx`; one VM per project). With --update, repeat it to refresh only these recorded guests and skip this machine's own update
    #[arg(
        long = "vm",
        value_name = "NAME",
        requires = "vm_target",
        conflicts_with = "wsl"
    )]
    pub vm: Vec<String>,
    /// With --wsl or --lima: clone the current checkout into the guest (default `~/<checkout name>`)
    #[arg(
        long,
        requires = "guest",
        num_args = 0..=1,
        default_missing_value = "",
        value_name = "LINUX_DIR"
    )]
    pub clone: Option<String>,
    /// With --lima: CPUs of the VM (default 6, at most all but two of this Mac's; changes an existing VM after asking)
    #[arg(long, requires = "lima", conflicts_with = "wsl", value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub vm_cpus: Option<u32>,
    /// With --lima: memory of the VM in GiB (default 12, 2 up to half of this Mac's; changes an existing VM after asking)
    #[arg(long, requires = "lima", conflicts_with = "wsl", value_name = "GIB", value_parser = clap::value_parser!(u32).range(1..))]
    pub vm_memory: Option<u32>,
    /// With --lima: disk of the VM in GiB (default 100, at least 30; an existing VM's disk can only grow)
    #[arg(long, requires = "lima", conflicts_with = "wsl", value_name = "GIB", value_parser = clap::value_parser!(u32).range(1..))]
    pub vm_disk: Option<u32>,
    /// With --lima: mount the macOS home in the VM writable instead of read-only (changes an existing VM after asking)
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
    /// Unregister a WSL distribution rayx created, and everything in it
    Remove(RemoveArgs),
}

/// Subcommands of `rayx lima`.
#[derive(Debug, Subcommand)]
pub enum LimaCommand {
    /// Delete a Lima VM rayx created, and everything in it
    Remove(RemoveArgs),
}

/// Arguments of `rayx lima remove` and `rayx wsl remove`.
#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// The guest to remove: a name `rayx doctor` lists
    pub name: String,
    /// Do not ask before removing
    #[arg(long)]
    pub yes: bool,
}

/// Flags of `rayx wsl compact`.
#[derive(Debug, Default, Args)]
pub struct CompactArgs {
    /// The distribution to compact (default the pinned Ubuntu, else the first WSL 2 distribution)
    #[arg(long)]
    pub distro: Option<String>,
    /// First offer to delete the build output (`target/`, `.nsh/`) under the cloned checkouts
    #[arg(long)]
    pub clean: bool,
    /// Do not ask before deleting build output or shutting WSL down
    #[arg(long)]
    pub yes: bool,
}

/// Subcommands of `rayx self`.
#[derive(Debug, Subcommand)]
pub enum SelfCommand {
    /// Replace this rayx with the latest release, or with the one named by --release
    Update {
        /// Report whether a newer release exists without installing it
        #[arg(long)]
        check: bool,
        /// Install exactly this release (X.Y.Z, for example 1.0.1), also an older one after asking; it is not remembered
        #[arg(long, value_name = "RELEASE")]
        release: Option<String>,
        /// Skip the question before installing an older release, and refresh this machine's setup and guests with the new rayx without asking
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

/// Arguments of `rayx app`: rayx's own options, then the nsh command and its arguments unchanged.
#[derive(Debug, Args)]
#[command(
    subcommand_value_name = "COMMAND",
    after_help = "nsh commands: init, build, check, dev, run, test, package, publish, keygen, certify, verify, revoke, symbolicate.\nEverything after the command goes to nsh unchanged; `rayx app <COMMAND> --help` prints nsh's own help."
)]
pub struct AppArgs {
    /// Run in this directory instead of the current one: the pin is read and nsh runs there
    #[arg(short = 'C', value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Report missing prerequisites (the pinned nsh among them) instead of installing them
    #[arg(long)]
    pub no_install: bool,
    /// The nsh command and its arguments
    #[command(subcommand)]
    pub nsh: AppCommand,
}

impl AppArgs {
    /// The nsh command (`init`, `build`, `check`, `dev`, `run`, `test`, `package`, `publish`,
    /// `keygen`, `certify`, `verify`, `revoke`, `symbolicate`, or whatever the pinned nsh adds).
    pub fn command(&self) -> &OsStr {
        let AppCommand::Nsh(words) = &self.nsh;
        words.first().map_or(OsStr::new(""), OsString::as_os_str)
    }

    /// The nsh command's arguments, exactly as given (`--` and what follows included).
    pub fn args(&self) -> &[OsString] {
        let AppCommand::Nsh(words) = &self.nsh;
        words.get(1..).unwrap_or_default()
    }
}

/// The nsh command of `rayx app`: any name, with its arguments captured raw (D12).
#[derive(Debug, Subcommand)]
pub enum AppCommand {
    #[command(external_subcommand)]
    Nsh(Vec<OsString>),
}

/// Subcommands of `rayx nsh`.
#[derive(Debug, Subcommand)]
pub enum NshCommand {
    /// Show the project's nsh pin, where it comes from and its store entry
    Status {
        /// Run in this directory instead of the current one
        #[arg(short = 'C', value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Print the status as JSON
        #[arg(long)]
        json: bool,
    },
    /// Print the path of the project's nsh, installing it first when it is missing
    Path {
        /// Run in this directory instead of the current one
        #[arg(short = 'C', value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Exit 1 with the `rayx setup` command instead of installing a missing nsh
        #[arg(long)]
        no_install: bool,
    },
    /// Install the project's nsh into the store
    Install {
        /// Run in this directory instead of the current one
        #[arg(short = 'C', value_name = "DIR")]
        dir: Option<PathBuf>,
    },
    /// Remove the store entries no recorded project and no default pins
    Prune {
        /// Remove without asking
        #[arg(long)]
        yes: bool,
    },
}

/// The app templates `rayx new` knows: nsh's own three.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum Template {
    /// TSX with a Rust crate at the app's root
    #[default]
    Mixed,
    /// TSX only
    Ts,
    /// Rust only
    Rust,
}

impl Template {
    /// The template's name as nsh takes it.
    pub fn name(self) -> &'static str {
        match self {
            Template::Mixed => "mixed",
            Template::Ts => "ts",
            Template::Rust => "rust",
        }
    }
}

/// Arguments of `rayx new`.
#[derive(Debug, Args)]
pub struct NewArgs {
    /// The directory of the new app
    pub dir: PathBuf,
    /// The app's name (default the directory's name)
    #[arg(long)]
    pub name: Option<String>,
    /// The app template
    #[arg(long, value_enum, default_value_t = Template::Mixed)]
    pub template: Template,
}

/// Arguments of `rayx serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// The site to serve (default the current directory)
    #[arg(long, value_name = "SITE")]
    pub dir: Option<PathBuf>,
    /// The port on 127.0.0.1 (default `PORT`, else 8080)
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,
}
