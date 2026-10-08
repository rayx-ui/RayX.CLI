---
agentx:
  kind: decision
  scope: decisions/host
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - host
    - runner
    - privilege
  supersedes: []
  superseded_by: []
---

## Decision: Describe the host with fixture probes and run every program through one Runner

### Context: Epic `rayx-cli-foundation` (design decision `d-host`), 2026-10-07 to 2026-10-08.

### Decision text

`host::facts()` builds `HostFacts { os, arch, emulated, wsl, distro }` from a `Probe` trait (environment variables, file reads, `IsWow64Process2` through `windows-sys`, `sysctlbyname("sysctl.proc_translated")` through `libc`). Tests construct `HostFacts` through fixture probes.

`host::Runner` executes a `CommandSpec { program, args, env, cwd, privilege: None | Root | Admin, interactive }` in one of three modes: `Execute`, `Print` (writes the exact command line, prefixing `sudo` for `Root` when not root and marking `Admin`) and `Record` (tests). An `Admin` command on Windows runs as one elevated child through `ShellExecuteExW` with the `runas` verb, writing its output to a temporary file the parent reads back. Interactive commands inherit the console so `sudo`, UAC and license prompts reach the user.

`host::path_env` edits the user PATH: the Windows registry through a `PathStore` trait (real `HKCU\Environment` store plus `WM_SETTINGCHANGE` broadcast, or an in-memory store for tests), and a marked block in the shell profile on Unix.

### Reasoning: See the epic design and the code it describes.

### Invariants

- The CLI never reads, stores or forwards a password or token and never runs itself elevated; elevation is per command, through the operating system's prompt.
- `Print` mode performs no side effects at all, including PATH edits.

### Supersedes: None.
