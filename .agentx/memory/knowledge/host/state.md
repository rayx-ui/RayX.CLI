---
agentx:
  kind: state
  scope: knowledge/host
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - host
  supersedes: []
  superseded_by: []
---

# Host State

## Purpose And Usage

Read before changing how rayx detects the machine, runs external programs, asks for root or administrator rights, or edits the user PATH.

## Current Technical Shape

Ported unchanged from RayX.CLI.Old (`d006250`). `src/host/mod.rs`/`probe.rs` compute `HostFacts` from a `Probe`; `runner.rs` runs every `CommandSpec` in `Execute`, `Print` or `Record` mode (`Runner::query` for read-only probes, `sudo -v` before non-root `Root` steps, `WindowsElevator` for `Admin`); `path_env.rs` edits the user PATH and persistent variables (`RegistryPathStore`, marked `# >>> rayx >>>` profile blocks).

## Backlinks

- [Host module](../../../../src/host/mod.rs), [runner](../../../../src/host/runner.rs), [PATH editing](../../../../src/host/path_env.rs)
- [Tests](../../../../tests/command_runner.rs), [host facts](../../../../tests/host_facts.rs), [PATH](../../../../tests/path_env.rs)
- 2026-10-24 current [Decision: host facts and one Runner](../../decisions/host/2026-10-24-host-facts-probes-and-one-runner.md)
