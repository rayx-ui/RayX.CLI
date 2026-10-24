---
agentx:
  kind: decision
  scope: decisions/project
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  promoted_from: rayx-1-0
  topics:
    - project
    - pins
  supersedes: []
  superseded_by: []
---

## Decision: Pins come from the project or the built-in defaults table

### Context: Carried over from RayX.CLI.Old (`.agentx/memory/decisions/project/2026-10-19-project-pins-from-project-or-default.md`) and adapted for NSH by epic `rayx-1-0` (design `d-port`, `d-nsh-sets`, `d-pin`).

### Decision text
`Pins::resolve` takes each pin from the project (`Cargo.lock`, `rust-toolchain.toml`, `[workspace.metadata.rayx]`) with source `project`, else from `DEFAULTS` in `src/project/pins.rs` with source `default`. Nothing else selects a pin: no member name, no compile-time path. The pins are design §7's: `rust_toolchain`, `wasm_bindgen`, `node`, `jdk`, `android_platform` (android-35), `android_build_tools` (35.0.1), `android_ndk`, `android_system_image`, `xcode`, `ubuntu`; the nsh pin joins them with the nsh store.

### Reasoning: One table makes every default visible and testable; a project declares what it needs, as Gamify and RayX's apps do.

### Invariants
- Built-in defaults live in one table in `project::pins`; no other module hard-codes a version.
- `PinSource` is `Project` or `Default`.

### Supersedes: None in this repository.
