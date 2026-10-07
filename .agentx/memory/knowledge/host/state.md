---
agentx:
  kind: state
  scope: knowledge/host
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
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

Planned by the epic `rayx-cli-foundation` (design decision `d-host`); no source exists yet. The module will be `src/host/` (`mod.rs` for `HostFacts` and the `Probe` trait, `runner.rs`, `path_env.rs`). Every other module receives `HostFacts` and a `Runner` instead of probing or spawning on its own.

## Backlinks

- [Epic design (`d-host`)](../../../tasks/rayx-cli-foundation/design.json)
- [Windows prerequisite script this replaces](../../../../../Gpux/.claude/skills/gpux/scripts/install-windows-prereqs.ps1)
