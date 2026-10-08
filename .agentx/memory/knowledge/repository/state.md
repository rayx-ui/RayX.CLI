---
agentx:
  kind: state
  scope: knowledge/repository
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:59:00+02:00
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

`RayX.CLI` lives beside `RayX` and `Gpux` (`D:\Work\projects\RayX\RayX.CLI`) and is public on GitHub as `rayx-ui/RayX.CLI` under Apache-2.0. It depends on neither: RayX and Gpux stay private, so the CLI builds from crates.io alone and its CI proves behavior with fixture projects, while real-app proof runs from the local RayX checkout. RayX will call `rayx` from CI and docs and keep a slim xtask for its own chores. Gpux changes in this epic only for the nightly and JDK pins; a separate Gpux change will point its setup docs at `rayx`. The first epic is `rayx-cli-foundation` (complete except the owner-run manual steps listed in its handoffs); RayX now calls `rayx` and keeps only the `code-themes`, `component-index` and `theme` chores in `xtask/`, with its source-policy tests in `xtask/src/source_policy.rs` and CI pinned to a CLI commit (`RAYX_CLI_REV` in `.github/workflows/pr-test.yml`; move it with the CLI). The root package `rayx-cli` (library `rayx_cli`, binary `rayx`) has the modules `cli`, `host`, `project`, `setup` (with `doctor` and `wsl`), `app`, `fmt`, `update` and `wsl`; integration tests are one `tests/<feature>.rs` group per feature, and `tests/package_layout.rs` enforces the manifest rules (registry-only dependencies, Rust 1.95.0 pin, thin `main.rs`).

## Backlinks

- [Package manifest](../../../../Cargo.toml)
- [Package layout test](../../../../tests/package_layout.rs)
- [Repository instructions](../../../../AGENTS.md)
- [Epic requirements](../../../tasks/rayx-cli-foundation/requirements.json)
- [RayX root manifest](../../../../../RayX/Cargo.toml)
- [Gpux instructions](../../../../../Gpux/CLAUDE.md)
