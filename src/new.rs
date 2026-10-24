//! `rayx new <dir> [--name <name>] [--template mixed|ts|rust]`: an NSH app that names its nsh from
//! its first commit (design §4.7, D5, D7, NEW-1).
//!
//! 1. Install rayx's default pin if needed and run its `nsh init <dir> --name … --template …`.
//! 2. `mixed` and `rust`: write `.cargo/config.toml` with a `[patch.crates-io]` entry for every
//!    NSH crate of the default pin (git URL and tag), then `cargo generate-lockfile`, so the app's
//!    `Cargo.lock` holds the pin.
//! 3. `ts`: write `rayx.toml` with the default pin's `[nsh]` table.
//! 4. Print the next steps.
//!
//! `--template` takes nsh's own three names; later a name can map to a template rayx fetches,
//! run after step 2.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cli::{NewArgs, Template};
use crate::host::{CommandSpec, HostFacts, Mode, Os, Runner};
use crate::nsh::install::{Install, RustTools, without_cargo};
use crate::nsh::pin::{RAYX_TOML, default_pin, rayx_toml_table};
use crate::nsh::store::Store;
use crate::project::pins::{DEFAULTS, DefaultNsh};
use crate::setup::{Machine, SystemMachine};

/// Runs `rayx new` on this machine.
pub fn run(args: &NewArgs) -> u8 {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let host = crate::host::facts();
    let machine = SystemMachine;
    let mut runner = Runner::execute();
    match create(args, &cwd, &host, &machine, &mut runner, &mut io::stdout()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("rayx: {error:#}");
            1
        }
    }
}

/// The `.cargo/config.toml` that patches every NSH crate of `default` to its git tag.
pub fn cargo_config(default: &DefaultNsh) -> String {
    let mut text = format!(
        "# The NSH this app is built with: rayx's default pin when the app was created\n\
         # ({} at {}). Cargo.lock records its commit, which is the app's nsh pin;\n\
         # move every line together to change it.\n\n[patch.crates-io]\n",
        default.tag,
        &default.commit[..7]
    );
    for name in default.crates {
        text.push_str(&format!(
            "{name} = {{ git = \"{}\", tag = \"{}\" }}\n",
            default.git, default.tag
        ));
    }
    text
}

/// The `rayx.toml` of a `ts` app.
pub fn rayx_toml(default: &DefaultNsh) -> String {
    format!(
        "# The nsh this app runs with: it has no Rust crates, so no Cargo.lock names it.\n{}",
        rayx_toml_table(default)
    )
}

/// The body of `rayx new` over injectable collaborators; progress and the next steps on `out`.
pub fn create(
    args: &NewArgs,
    cwd: &Path,
    host: &HostFacts,
    machine: &dyn Machine,
    runner: &mut Runner,
    out: &mut dyn Write,
) -> Result<()> {
    let dir = cwd.join(&args.dir);
    // rayx pins the app in one file; one that is already there is the user's, and nsh's own list
    // of conflicting files does not name it, so nothing starts.
    let pin_file = match args.template {
        Template::Mixed | Template::Rust => dir.join(".cargo").join("config.toml"),
        Template::Ts => dir.join(RAYX_TOML),
    };
    if pin_file.exists() {
        bail!(
            "{} exists, and `rayx new` would replace it with the app's nsh pin; move it away first",
            pin_file.display()
        );
    }
    let store = Store::locate(machine)
        .ok_or_else(|| anyhow::anyhow!("no home directory for the nsh store (set RAYX_HOME)"))?;
    let pin = default_pin(cwd);
    // As setup's Rust steps find them: a shell opened before `rayx setup` installed Rust has no
    // `~/.cargo/bin` on its PATH yet.
    let tools = RustTools::find(machine, host.os);
    let installed = Install {
        runner,
        store: &store,
        host,
        tools: &tools,
        out,
    }
    .run(&pin)?;

    // nsh refuses a directory with files in it, and then nothing below is written.
    let mut init = CommandSpec::new(installed.cli.display().to_string())
        .arg("init")
        .arg(dir.display().to_string())
        .args(["--template", args.template.name()]);
    if let Some(name) = &args.name {
        init = init.args(["--name", name.as_str()]);
    }
    runner
        .run_checked(&without_cargo(init).cwd(cwd).interactive())
        .context("nsh init")?;

    match args.template {
        Template::Mixed | Template::Rust => {
            if !dir.join("Cargo.toml").is_file() && runner.mode() != Mode::Print {
                bail!(
                    "nsh init wrote no Cargo.toml in {}, so the pin cannot go into a Cargo.lock",
                    dir.display()
                );
            }
            write(
                &dir.join(".cargo").join("config.toml"),
                &cargo_config(&DEFAULTS.nsh),
                runner,
            )?;
            runner
                .run_checked(
                    &without_cargo(CommandSpec::new(tools.cargo.as_str()).arg("generate-lockfile"))
                        .cwd(&dir)
                        .interactive(),
                )
                .context("cargo generate-lockfile")?;
        }
        Template::Ts => write(&dir.join(RAYX_TOML), &rayx_toml(&DEFAULTS.nsh), runner)?,
    }

    let shown = args.dir.display();
    let _ = writeln!(
        out,
        "\nCreated {shown}, pinned to nsh {}. Next:",
        pin.label()
    );
    let _ = writeln!(out, "  cd {shown}");
    let web = if host.os == Os::MacOs {
        "  rayx setup --web --ios"
    } else {
        "  rayx setup --web"
    };
    let _ = writeln!(out, "{web}");
    let _ = writeln!(out, "  rayx app run --target web");
    Ok(())
}

/// Writes a file unless this is a dry run.
fn write(path: &Path, text: &str, runner: &Runner) -> Result<()> {
    if runner.mode() == Mode::Print {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}
