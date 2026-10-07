---
agentx:
  kind: state
  scope: knowledge/project
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
  topics:
    - project-discovery
    - pins
    - cargo-metadata
  supersedes: []
  superseded_by: []
---
# Project State

## Purpose And Usage

Read before changing how `rayx` finds the project it works on or which tool versions it installs and builds with.

## Current Technical Shape

Planned by the epic `rayx-cli-foundation` (design decision `d-project-pins`); no source exists yet. The module will be `src/project/` (`mod.rs` for `Project::discover`, `pins.rs` for `Pins::resolve`). One `Pins` value flows to setup, doctor and the app pipeline.

## Backlinks

- [Epic design (`d-project-pins`)](../../../tasks/rayx-cli-foundation/design.json)
- [RayX xtask compile-time repository root this replaces](../../../../../RayX/xtask/src/fs_util.rs)
- [gpux Android pins](../../../../../Gpux/tooling/testkit/src/android.rs)
