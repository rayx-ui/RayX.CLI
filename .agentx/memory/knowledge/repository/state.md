---
agentx:
  kind: state
  scope: knowledge/repository
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T00:30:00+02:00
  topics:
    - repository
    - governance
    - agentx-memory
    - sibling-repositories
  supersedes: []
  superseded_by: []
---
# Repository State

## Purpose And Usage

Read before repository-level work: manifests and dependency policy, commits, cross-repository changes with RayX, or memory maintenance.

## Current Technical Shape

`RayX.CLI` lives beside `RayX` and `Gpux` (`D:\Work\projects\RayX\RayX.CLI`) and is public on GitHub as `rayx-ui/RayX.CLI` under Apache-2.0. It depends on neither: RayX and Gpux stay private, so the CLI builds from crates.io alone and its CI proves behavior with fixture projects, while real-app proof runs from the local RayX checkout. RayX will call `rayx` from CI and docs and keep a slim xtask for its own chores. Gpux changes in this epic only for the nightly and JDK pins; a separate Gpux change will point its setup docs at `rayx`. The first epic is `rayx-cli-foundation`.

## Backlinks

- [Repository instructions](../../../../AGENTS.md)
- [Epic requirements](../../../tasks/rayx-cli-foundation/requirements.json)
- [RayX root manifest](../../../../../RayX/Cargo.toml)
- [Gpux instructions](../../../../../Gpux/CLAUDE.md)
