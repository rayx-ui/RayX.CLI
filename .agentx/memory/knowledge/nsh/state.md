---
agentx:
  kind: state
  scope: knowledge/nsh
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - nsh
  supersedes: []
  superseded_by: []
---

# nsh State

## Purpose And Usage

Read before changing how rayx chooses, installs or runs nsh.

## Current Technical Shape

Planned by epic `rayx-1-0` (design §4, decisions `d-pin`, `d-store`, `d-install`, `d-run`, `d-fork-dev`), not implemented yet: the pin from `Cargo.lock` (or `rayx.toml`, else the built-in default `fork-0.2.0-4` at `327bb9e`), one store entry per commit under `$RAYX_HOME/nsh` with a receipt written last, installs from the fork's release archive or from a source checkout, and `rayx nsh status|path|install|prune`. The command surface exists in `src/cli.rs` (`NshCommand`) and answers that it is not available yet.

## Backlinks

- [Command surface](../../../../src/cli.rs)
- [Design §4](../../../../docs/architecture/rayx-nsh-proposal.md)
