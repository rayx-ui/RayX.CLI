---
agentx:
  kind: state
  scope: knowledge/project
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - project
  supersedes: []
  superseded_by: []
---

# Project State

## Purpose And Usage

Read before changing how rayx finds the project it works on or which tool versions it installs.

## Current Technical Shape

`src/project/mod.rs`: `Project::discover` runs `cargo locate-project --workspace` and `cargo metadata --no-deps` with `RUSTUP_AUTO_INSTALL=0`, else reads the manifests (`Discovery::Files`); packages carry name, manifest and `[package.metadata.rayx]`, without the old pipeline's dependency model. `src/project/pins.rs`: `Pins::resolve` and `DEFAULTS` (node 22, jdk 21, android-35, build-tools 35.0.1, NDK 28.0.12674087, `google_apis` images, Ubuntu 24.04). `[workspace.metadata.rayx] tools-node` reaches `PlanEnv.tools_node` through `setup::environment_for`. `rayx fmt` (`src/fmt.rs`) formats one member at a time.

## Backlinks

- [Discovery](../../../../src/project/mod.rs), [pins](../../../../src/project/pins.rs), [fmt](../../../../src/fmt.rs)
- [Pin tests](../../../../tests/project_pins.rs), [sources](../../../../tests/pin_sources.rs), [fmt tests](../../../../tests/fmt_command.rs)
- 2026-10-24 current [Decision: pins from the project or the defaults table](../../decisions/project/2026-10-24-project-pins-from-project-or-defaults.md)
