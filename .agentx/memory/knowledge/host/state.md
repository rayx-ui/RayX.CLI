---
agentx:
  kind: state
  scope: knowledge/host
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:59:00+02:00
  topics:
    - host-facts
    - command-runner
    - privilege
    - path
  supersedes: []
  superseded_by: []
---
# Host State

## Purpose And Usage

Read before changing how `rayx` detects the machine, runs external programs, asks for root or administrator rights, or edits the user PATH.

## Current Technical Shape

Implemented in `src/host/` (epic `rayx-cli-foundation`, design decision `d-host`, sprint 1). Every other module receives `HostFacts` and a `Runner` instead of probing or spawning on its own.

- `mod.rs`/`probe.rs`: `detect(&dyn Probe) -> HostFacts { os, arch, emulated, wsl, distro }` is pure; `SystemProbe` asks the machine (`IsWow64Process2`, `sysctl.proc_translated`, `/etc/os-release`, `WSL_DISTRO_NAME`, `/proc/sys/kernel/osrelease`) and `FixtureProbe` scripts any host for tests. `Distro::is_supported` accepts Ubuntu, Debian and `ID_LIKE` derivatives.
- `runner.rs`: `Runner::{execute, print, print_to, record}` over `CommandSpec { program, args, env, cwd, privilege, interactive }`. `Print` writes the exact command line (`sudo` for non-root `Root` steps, `[admin]` marker) and `Record` captures specs with scripted `respond` answers; `is_dry_run()` tells callers to skip non-command side effects. A non-root `Root` step first runs an interactive `sudo -v` (a failure is `RunError::Declined`, the command does not run) and then `sudo [env K=V] program args`. `Admin` goes through the `Elevator` trait; `WindowsElevator` runs `cmd.exe` behind a `ShellExecuteExW` `runas` prompt with output redirected to a temp file and refuses `"`, `%` and line breaks in arguments. `with_sudo_program` and `with_elevator` are the test seams.
- `path_env.rs`: `PathStore` (`RegistryPathStore` on Windows, `MemoryPathStore` for tests) with `prepend_to_windows_path` (case/slash/`%VAR%`-insensitive duplicate check, value type preserved, `WM_SETTINGCHANGE` best effort), marked `# >>> rayx >>>` profile blocks for zsh/bash/sh through `add_to_profile`, and `UserPath` over both. `PathChange::notice()` carries the new-shell message.

The real UAC prompt cannot run in tests; the Unix `fake_sudo` tests in `tests/command_runner.rs` run on Linux and macOS only.

## Backlinks

- [Host module](../../../../src/host/mod.rs), [runner](../../../../src/host/runner.rs), [PATH editing](../../../../src/host/path_env.rs)
- [Host tests](../../../../tests/command_runner.rs)
- [Epic design (`d-host`)](../../../tasks/rayx-cli-foundation/design.json)
- [Windows prerequisite script this replaces](../../../../../Gpux/.claude/skills/gpux/scripts/install-windows-prereqs.ps1)
