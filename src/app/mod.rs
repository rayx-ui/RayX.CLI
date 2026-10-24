//! `rayx app [-C <dir>] [--no-install] <command> [arguments]`: an nsh command run with the nsh the
//! project pins (design §4.4, D12, RUN-1 to RUN-3).
//!
//! rayx reads only the command name and `--target` (for the prerequisites, [`prereqs`]); every
//! argument after the command goes to nsh unchanged, `--` and what follows included. nsh runs in
//! the directory the pin was read in, without `CARGO`, with the web set's LLVM as the wasm32 C
//! tools and the pinned NDK as `ANDROID_NDK_HOME` when those are unset; everything else passes
//! through. On Unix rayx replaces itself with nsh; on Windows it waits and exits with nsh's code.
//!
//! Fork development has two doors (design §4.6): `RAYX_NSH=<path>` runs that binary instead of
//! the pin's, with a warning on every command, and a path dependency on an NSH checkout builds and
//! runs that checkout's CLI. Neither writes to the store.

pub mod prereqs;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::cli::AppArgs;
use crate::host::path_env::UserPath;
use crate::host::{CommandSpec, HostFacts, Os, Runner};
use crate::nsh::install::{RustTools, in_nsh_checkout};
use crate::nsh::pin::{Pin, Source};
use crate::nsh::store::Store;
use crate::nsh::{self, Status};
use crate::project::Project;
use crate::setup::{self, Cx, Machine, SystemMachine, android, ask_on_terminal, macos};

pub use crate::nsh::RAYX_NSH;

/// What rayx hands to nsh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    /// The command and its arguments.
    pub args: Vec<OsString>,
    /// The directory the pin was read in.
    pub cwd: PathBuf,
    /// Variables set for nsh (only those that were unset).
    pub set: Vec<(String, String)>,
    /// Variables nsh runs without.
    pub unset: Vec<String>,
}

/// Starts nsh: the seam tests record.
pub trait Launcher {
    /// Runs `launch` and returns its exit code; on Unix the real launcher does not return when it
    /// succeeds, because the process becomes nsh.
    fn launch(&mut self, launch: &Launch) -> Result<u8>;
}

/// The real launcher: `exec` on Unix, so a process manager that stops rayx stops nsh; a child
/// whose exit code becomes rayx's on Windows.
pub struct ProcessLauncher;

impl Launcher for ProcessLauncher {
    fn launch(&mut self, launch: &Launch) -> Result<u8> {
        let mut command = std::process::Command::new(&launch.program);
        command.args(&launch.args).current_dir(&launch.cwd);
        for key in &launch.unset {
            command.env_remove(key);
        }
        for (key, value) in &launch.set {
            command.env(key, value);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let error = command.exec();
            Err(anyhow!("cannot run {}: {error}", launch.program.display()))
        }
        #[cfg(not(unix))]
        {
            // Ctrl+C reaches nsh, which shares this console. rayx registers a handler that takes
            // the event for itself only (a handler is not inherited, unlike the null form, which
            // would keep it from nsh too), so it waits for nsh's exit code, as cargo does.
            #[cfg(windows)]
            // SAFETY: the handler is a plain function that touches nothing and lives as long as
            // the process.
            unsafe {
                windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(ignore_ctrl), 1);
            }
            let status = command
                .status()
                .with_context(|| format!("cannot run {}", launch.program.display()))?;
            Ok(status
                .code()
                .map_or(1, |code| u8::try_from(code).unwrap_or(1)))
        }
    }
}

/// A console control handler that takes Ctrl+C and Ctrl+Break for rayx alone while nsh runs.
#[cfg(windows)]
unsafe extern "system" fn ignore_ctrl(_: u32) -> windows_sys::core::BOOL {
    1
}

/// The prerequisite step `prepare` runs: the plan environment, the sets beyond the base set, and
/// the directory `rayx setup` would run in. The real one is [`prereqs::ensure`].
pub type Ensure<'a> = dyn FnMut(
        &setup::PlanEnv,
        &[setup::Set],
        &Path,
        &mut Runner,
        &dyn Machine,
        &mut UserPath,
    ) -> Result<usize>
    + 'a;

/// Runs `rayx app` on this machine and returns the exit code (on Unix only when nsh could not be
/// started).
pub fn run(args: &AppArgs) -> u8 {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let host = crate::host::facts();
    let machine = SystemMachine;
    let mut user_path = match UserPath::system(host.os) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("rayx: cannot edit the user PATH: {error}");
            return 1;
        }
    };
    let options = prereqs::Options::from_terminal(args.no_install);
    let mut runner = Runner::execute();
    let mut ensure = |env: &setup::PlanEnv,
                      sets: &[setup::Set],
                      dir: &Path,
                      runner: &mut Runner,
                      machine: &dyn Machine,
                      user_path: &mut UserPath| {
        let mut confirm = ask_on_terminal;
        prereqs::ensure(
            env,
            sets,
            dir,
            &options,
            runner,
            machine,
            user_path,
            &mut io::stdout(),
            &mut confirm,
        )
    };
    let prepared = prepare(
        args,
        &cwd,
        &host,
        &mut runner,
        &machine,
        &mut user_path,
        &mut ensure,
        &mut io::stderr(),
    );
    match prepared.and_then(|launch| ProcessLauncher.launch(&launch)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("rayx: {error:#}");
            1
        }
    }
}

/// Everything before nsh starts: the pin (read in the directory nsh runs in), the prerequisites
/// through `ensure`, then [`launch_for`]. `err` gets the warnings.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    args: &AppArgs,
    cwd: &Path,
    host: &HostFacts,
    runner: &mut Runner,
    machine: &dyn Machine,
    user_path: &mut UserPath,
    ensure: &mut Ensure,
    err: &mut dyn Write,
) -> Result<Launch> {
    let dir = match &args.dir {
        Some(dir) => std::path::absolute(cwd.join(dir)).unwrap_or_else(|_| cwd.join(dir)),
        None => cwd.to_path_buf(),
    };
    if !dir.is_dir() {
        return Err(anyhow!("{} is not a directory", dir.display()));
    }
    let replacement = nsh::replacement(machine, cwd);

    // The pin, read where nsh runs, through the command's runner: `rayx app` executes, so a
    // missing `Cargo.lock` is written first (PIN-1). `RAYX_NSH` replaces its CLI.
    let pin = if replacement.is_some() {
        nsh::pin::resolve(runner, &dir).ok()
    } else {
        Some(nsh::resolve(runner, &dir, err)?)
    };
    if let Some(path) = &replacement {
        let _ = writeln!(err, "{}", nsh::replacement_warning(path, pin.as_ref()));
    }

    // The prerequisites of the command and its target; the pinned nsh is the base set's last
    // step (none with `RAYX_NSH`).
    let project = Project::discover(&mut Runner::print(), &dir, None).ok();
    let nsh = if replacement.is_some() {
        None
    } else {
        pin.clone().map(Ok)
    };
    let env = setup::environment_with(host.clone(), project.as_ref(), false, nsh);
    let command = args.command().to_string_lossy().into_owned();
    let target = prereqs::target(args.args());
    let sets = prereqs::required_sets(&command, target.as_deref());
    ensure(&env, &sets, &dir, runner, machine, user_path)?;

    let mut cx = Cx::new(&env, runner, machine, user_path);
    launch_for(
        args,
        &dir,
        pin.as_ref(),
        replacement.as_deref(),
        &mut cx,
        err,
    )
}

/// The launch of nsh in `dir`: the CLI (`replacement`, else the pin's), the command and its
/// arguments unchanged (with `--browser` where RUN-2 adds it), and the environment.
pub fn launch_for(
    args: &AppArgs,
    dir: &Path,
    pin: Option<&Pin>,
    replacement: Option<&Path>,
    cx: &mut Cx,
    err: &mut dyn Write,
) -> Result<Launch> {
    let program = match (replacement, pin) {
        (Some(path), _) => path.to_path_buf(),
        (None, Some(pin)) => cli_of(pin, cx.runner, cx.machine, &cx.env.host, err)?,
        (None, None) => return Err(anyhow!("no nsh pin and no {RAYX_NSH}")),
    };
    let command = args.command().to_string_lossy().into_owned();
    let nsh_args = args.args();
    let browser = prereqs::browser_args(&command, nsh_args, cx);
    let split = nsh_args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(nsh_args.len());
    let mut launch_args: Vec<OsString> = Vec::with_capacity(nsh_args.len() + 3);
    launch_args.push(args.command().to_os_string());
    launch_args.extend(nsh_args[..split].iter().cloned());
    launch_args.extend(browser);
    launch_args.extend(nsh_args[split..].iter().cloned());
    Ok(Launch {
        program,
        args: launch_args,
        cwd: dir.to_path_buf(),
        set: environment(cx),
        unset: vec!["CARGO".to_string()],
    })
}

/// The pin's CLI: the store entry's (the prerequisites installed it), or a path checkout's,
/// built in place with `cargo build --release -p nsh-cli --target-dir target`.
fn cli_of(
    pin: &Pin,
    runner: &mut Runner,
    machine: &dyn Machine,
    host: &HostFacts,
    err: &mut dyn Write,
) -> Result<PathBuf> {
    if let Source::Path { checkout } = &pin.source {
        let _ = writeln!(
            err,
            "rayx: building the nsh CLI of the checkout {}",
            checkout.display()
        );
        // The checkout's own toolchain, with the cargo setup's Rust steps find.
        let cargo = RustTools::find(machine, host.os).cargo;
        runner
            .run_checked(
                // `--target-dir target` outranks a user's `CARGO_TARGET_DIR` and
                // `build.target-dir`: the CLI rayx runs is the one this build makes.
                &in_nsh_checkout(CommandSpec::new(cargo.as_str()).args([
                    "build",
                    "--release",
                    "-p",
                    "nsh-cli",
                    "--target-dir",
                    "target",
                ]))
                .cwd(checkout)
                .interactive(),
            )
            .with_context(|| format!("building nsh in {}", checkout.display()))?;
        let exe = if host.os == Os::Windows {
            "nsh.exe"
        } else {
            "nsh"
        };
        return Ok(checkout.join("target").join("release").join(exe));
    }
    let store =
        Store::locate(machine).ok_or_else(|| anyhow!("no home directory for the nsh store"))?;
    let status = Status::of(pin, &store, host);
    status.cli.filter(|_| status.installed).ok_or_else(|| {
        anyhow!(
            "nsh {} is not installed; install it with `{}`",
            pin.label(),
            nsh::setup_command(&pin.dir)
        )
    })
}

/// The variables rayx sets for nsh: the wasm32 C tools of the web set's LLVM and the pinned NDK as
/// `ANDROID_NDK_HOME`, each only when unset, and a set `PATH` with `~/.cargo/bin` in front when
/// that directory exists and `PATH` lacks it (the prerequisite step may just have installed
/// rustup, and nsh runs `cargo` by name).
pub fn environment(cx: &mut Cx) -> Vec<(String, String)> {
    let mut set = Vec::new();
    // An unset or empty `PATH` stays as it is: nsh then gets the system's default search path.
    if let Some(bin) = setup::rust::cargo_bin_dir(cx).filter(|bin| cx.machine.is_dir(bin))
        && let Some(path) = cx.machine.env("PATH").filter(|path| !path.is_empty())
    {
        let dirs: Vec<PathBuf> = std::env::split_paths(&path)
            .filter(|dir| !dir.as_os_str().is_empty())
            .collect();
        if !dirs.contains(&bin)
            && let Ok(joined) = std::env::join_paths(std::iter::once(bin).chain(dirs))
        {
            set.push(("PATH".to_string(), joined.to_string_lossy().into_owned()));
        }
    }
    let unset = |cx: &Cx, name: &str| cx.machine.env(name).is_none_or(|value| value.is_empty());
    if let Some((cc, ar)) = wasm_tools(cx) {
        for (name, tool) in [
            ("CC_wasm32_unknown_unknown", cc),
            ("AR_wasm32_unknown_unknown", ar),
        ] {
            if unset(cx, name) {
                set.push((name.to_string(), tool.display().to_string()));
            }
        }
    }
    if unset(cx, "ANDROID_NDK_HOME")
        && let Some(sdk) = android::find_sdk(cx)
        && let Some(ndk) = android::pinned_ndk(cx, &sdk)
    {
        set.push(("ANDROID_NDK_HOME".to_string(), ndk.display().to_string()));
    }
    set
}

/// The clang and llvm-ar that target wasm32: Homebrew's LLVM on macOS (Apple's clang cannot),
/// the distribution's on Linux, LLVM's own on Windows; `None` when they are not installed.
pub fn wasm_tools(cx: &Cx) -> Option<(PathBuf, PathBuf)> {
    let (cc, ar) = match cx.env.host.os {
        Os::MacOs => {
            let brew = macos::brew_program(cx)?;
            let prefix = brew.parent()?.parent()?.to_path_buf();
            let bin = prefix.join("opt").join("llvm").join("bin");
            (bin.join("clang"), bin.join("llvm-ar"))
        }
        Os::Linux => (
            PathBuf::from("/usr/bin/clang"),
            PathBuf::from("/usr/bin/llvm-ar"),
        ),
        Os::Windows => {
            let bin = PathBuf::from(
                cx.machine
                    .env("ProgramFiles")
                    .unwrap_or_else(|| r"C:\Program Files".to_string()),
            )
            .join("LLVM")
            .join("bin");
            (bin.join("clang.exe"), bin.join("llvm-ar.exe"))
        }
    };
    (cx.machine.exists(&cc) && cx.machine.exists(&ar)).then_some((cc, ar))
}
