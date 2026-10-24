---
agentx:
  kind: state
  scope: knowledge/repository
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - repository
  supersedes: []
  superseded_by: []
---

# Repository State

## Purpose And Usage

Read before repository-level work: manifests and dependencies, the guards, commits, consumer changes, the NSH fork, or memory maintenance.

## Current Technical Shape

One package `rayx-cli` (Rust 1.99.0, edition 2024, version 1.0.0), public as `rayx-ui/RayX.CLI` under Apache-2.0 once created. `tests/package_layout.rs`, `tests/dependency_sources.rs` and `tests/library_guard.rs` guard the layout, the dependency sources and D14. Integration tests are one `tests/<feature>.rs` group per feature with fixtures under `tests/fixtures/`.

## Backlinks

- [Instructions](../../../../AGENTS.md), [manifest](../../../../Cargo.toml)
- [Layout](../../../../tests/package_layout.rs), [dependency sources](../../../../tests/dependency_sources.rs), [library guard](../../../../tests/library_guard.rs)
- 2026-10-24 current [Decision: one package, public dependencies, no library above NSH](../../decisions/repository/2026-10-24-repository-one-package-public-dependencies-and-library-guard.md)
- 2026-10-24 current [Decision: memory taxonomy](../../decisions/repository/2026-10-24-repository-memory-taxonomy.md)
