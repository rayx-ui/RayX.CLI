---
agentx:
  kind: state
  scope: knowledge/app
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - app
  supersedes: []
  superseded_by: []
---

# App State

## Purpose And Usage

Read before changing `rayx app`, `rayx new` or `rayx serve`.

## Current Technical Shape

Planned by epic `rayx-1-0` (design §4.4, §4.7, §5, §6.1; decisions `d-run`, `d-new`, `d-serve`), not implemented yet. The clap surface exists in `src/cli.rs`: `rayx app [-C <dir>] [--no-install] <command> [args]` hands everything after the nsh command to nsh raw, `--` included (an external subcommand), `rayx new <dir> [--name] [--template mixed|ts|rust]` and `rayx serve [--dir] [--port]`. The old GPUI app pipeline is not ported (D13).

## Backlinks

- [Command surface](../../../../src/cli.rs), [its tests](../../../../tests/cli_surface.rs)
- [Design §4.4 and §5](../../../../docs/architecture/rayx-nsh-proposal.md)
