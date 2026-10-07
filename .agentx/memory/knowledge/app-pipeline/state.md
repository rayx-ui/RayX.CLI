---
agentx:
  kind: state
  scope: knowledge/app-pipeline
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
  topics:
    - app-commands
    - wasm
    - android
    - ios
    - assets
    - fmt
    - app-project-contract
  supersedes: []
  superseded_by: []
---
# App pipeline State

## Purpose And Usage

Read before changing app builds, generated entry manifests, the WASM server or Playwright runs, mobile packaging, asset staging, `rayx fmt`, or the `[package.metadata.rayx]` contract.

## Current Technical Shape

Planned by the epic `rayx-cli-foundation` (design decisions `d-app-move`, `d-fmt`, `d-cli-surface`); no source exists yet. RayX xtask's app modules move into `src/app/` (fresh history, provenance in the commit message), then take a `ProjectContext` instead of the compile-time repository root. The app grammar and flags stay exactly as in xtask; `docs/app-project.md` will document the app-project contract for a later `rayx new`.

## Backlinks

- [Epic design (`d-app-move`)](../../../tasks/rayx-cli-foundation/design.json)
- [RayX xtask command grammar](../../../../../RayX/xtask/src/command.rs)
- [RayX xtask app metadata and entry manifests](../../../../../RayX/xtask/src/app.rs)
- [RayX xtask WASM build, server and tests](../../../../../RayX/xtask/src/wasm.rs)
- [RayX xtask guide](../../../../../RayX/docs/src/xtask.md)
- 2026-06-29 current [RayX app-directory xtask decision](../../../../../RayX/.agentx/memory/decisions/platform/packaging-targets/2026-06-29-platform-packaging-targets-app-directory-xtask.md)
