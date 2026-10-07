---
agentx:
  kind: state
  scope: knowledge/project
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T05:00:00+02:00
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

Implemented in `src/project/` (epic `rayx-cli-foundation`, design decision `d-project-pins`, sprint 2). One `Pins` value flows to setup, doctor and the app pipeline.

- `Project::discover(runner, cwd, app)` resolves a relative app path against `cwd`, then runs `cargo locate-project --workspace` and `cargo metadata --no-deps` with `RUSTUP_AUTO_INSTALL=0` (a probe must never make rustup install a project's pinned toolchain). When cargo cannot run (a fresh machine, or the pinned toolchain missing) it falls back to the manifests: nearest `[workspace]`, members including `dir/*`, `[workspace.metadata.rayx]` and `[package.metadata.rayx]`. `Project.discovery` says which way it was found. Under `cargo test` the harness forces `RUSTUP_TOOLCHAIN` on children, so fallback tests script cargo's failure.
- `Pins::resolve(Option<&Project>)`: project (`Cargo.lock` wasm-bindgen, `rust-toolchain.toml` channel, `[workspace.metadata.rayx]` keys `web-toolchain`, `node`, `jdk`, `playwright`, `android-*`, `xcode`; integers accepted), then the gpux-checkout profile (a workspace containing `gpux-testkit`: Android platform/build-tools/NDK), then `DEFAULTS` in `pins.rs`, the one table of default versions. Every `Pin` carries its `PinSource`.
- `android-system-image` defaults to `<platform>;google_apis` of the resolved platform pin (reporting its source), so a gpux checkout's android-36 platform gets an android-36 image; the explicit key overrides it.
- Fixture workspaces live in `tests/fixtures/{plain-app,pinned-app,gpux-checkout,missing-toolchain}`, each with its own `[workspace]`.

## Backlinks

- [Project discovery](../../../../src/project/mod.rs), [pins](../../../../src/project/pins.rs)
- [Pin tests](../../../../tests/project_pins.rs)
- [Epic design (`d-project-pins`)](../../../tasks/rayx-cli-foundation/design.json)
- [RayX xtask compile-time repository root this replaces](../../../../../RayX/xtask/src/fs_util.rs)
- [gpux Android pins](../../../../../Gpux/tooling/testkit/src/android.rs)
