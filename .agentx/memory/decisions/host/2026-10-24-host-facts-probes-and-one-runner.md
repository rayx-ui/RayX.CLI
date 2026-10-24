---
agentx:
  kind: decision
  scope: decisions/host
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  promoted_from: rayx-1-0
  topics:
    - host
    - runner
    - probes
  supersedes: []
  superseded_by: []
---

## Decision: Describe the host with fixture probes and run every program through one Runner

### Context: Carried over from RayX.CLI.Old (`.agentx/memory/decisions/host/2026-10-08-host-facts-and-command-runner.md`, rayx 1.4.1) when epic `rayx-1-0` ported `src/host/` unchanged in behaviour (design `d-port`).

### Decision text
`host::facts()` builds `HostFacts { os, arch, emulated, wsl, distro }` from a `Probe` (environment, files, `IsWow64Process2`, `sysctl.proc_translated`); tests use `FixtureProbe`. Every external program runs through `host::Runner` over a `CommandSpec` in `Execute`, `Print` (the exact command line, `sudo` for non-root `Root` steps, an admin marker) or `Record` mode; `Runner::query` runs read-only probes. `host::path_env` edits the user PATH (the Windows registry through `PathStore`, a marked profile block on Unix) and persistent user variables.

### Reasoning: One seam for every program makes `--check`, doctor and tests exact; nsh, git, cargo and curl for the nsh store go through the same seam.

### Invariants
- rayx never reads, stores or forwards a password or token and never runs itself elevated; elevation is per command, through the operating system's prompt.
- `Print` mode has no side effects, PATH edits included.

### Supersedes: None in this repository.
