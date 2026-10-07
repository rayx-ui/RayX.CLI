use anyhow::{Context, Result, bail};
use std::process::{Command, Stdio};

/// Windows ends a console process interrupted with Ctrl+C by this status.
/// Stopping a run that way is how a run ends, not a failure.
#[cfg(windows)]
const STATUS_CONTROL_C_EXIT: i32 = 0xC000_013A_u32 as i32;

/// Whether an exit code is a Ctrl+C stop rather than a failure.
fn interrupted(status: &std::process::ExitStatus) -> bool {
    #[cfg(windows)]
    let interrupted = status.code() == Some(STATUS_CONTROL_C_EXIT);
    #[cfg(not(windows))]
    let interrupted = {
        let _ = status;
        false
    };
    interrupted
}

pub fn run(command: &mut Command) -> Result<()> {
    let status = command
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("failed to run {:?}", command))?;
    if !status.success() && !interrupted(&status) {
        bail!("command {:?} failed with {status}", command);
    }
    Ok(())
}

pub fn command_succeeds(command: &mut Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
