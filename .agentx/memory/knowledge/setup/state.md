---
agentx:
  kind: state
  scope: knowledge/setup
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T00:30:00+02:00
  topics:
    - setup
    - doctor
    - wsl
    - windows-arm64
    - installers
  supersedes: []
  superseded_by: []
---
# Setup State

## Purpose And Usage

Read before changing what `rayx setup` installs, how it decides what is missing, what `rayx doctor` reports, or how `setup --wsl` drives WSL.

## Current Technical Shape

Planned by the epic `rayx-cli-foundation` (design decisions `d-setup-plan` and `d-wsl`); no source exists yet. The module will be `src/setup/` (`mod.rs` engine; `linux.rs`, `windows.rs`, `macos.rs`; `rust.rs`, `web.rs`, `test_tools.rs`, `android.rs`, `ios.rs`; `wsl.rs`) plus `src/setup/gpu.rs`, `src/doctor.rs` and `src/wsl.rs` (`rayx wsl status`/`compact`). One step table feeds `setup`, `setup --check` and `doctor`. It replaces gpux's `LINUX-WSL.md` sections 2–4, the gpux skill's prerequisite references and `install-windows-prereqs.ps1`, and RayX xtask's `setup-android` and `setup-wasm`.

## Backlinks

- [Epic design (`d-setup-plan`, `d-wsl`)](../../../tasks/rayx-cli-foundation/design.json)
- [gpux Linux on WSL guide](../../../../../Gpux/LINUX-WSL.md)
- [gpux Windows prerequisites](../../../../../Gpux/.claude/skills/gpux/references/windows-prerequisites.md)
- [gpux macOS, Linux and WASM prerequisites](../../../../../Gpux/.claude/skills/gpux/references/macos-linux-wasm-prerequisites.md)
- [RayX xtask setup-android](../../../../../RayX/xtask/src/android.rs)
- [RayX xtask setup-wasm](../../../../../RayX/xtask/src/node_tools.rs)
