---
agentx:
  kind: state
  scope: knowledge/setup
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - setup
  supersedes: []
  superseded_by: []
---

# Setup State

## Purpose And Usage

Read before changing what `rayx setup` installs, how it decides what is missing, what `rayx doctor` reports, or how the WSL and Lima guests work.

## Current Technical Shape

- Engine (`src/setup/mod.rs`, `machine.rs`): `Set` is `base`, `web`, `test`, `android`, `ios` (no GPU set, D15); `plan`, `check`, `execute`, `run_plan`, `--check`, `--yes`, the outdated-item prompt; `records.rs` writes `setup.json` and `guests.json` (version 1, atomic, never on `--check`; `--gpu` from old records is dropped on read).
- Base set: Linux installs NSH's apt packages (GTK 4, WebKitGTK 6.0, GStreamer plugins, clang, LLVM) and refuses Ubuntu below 24.04 and Debian below 13 (`linux::too_old`); Windows adds the WebView2 Runtime and has no `fxc.exe` step; macOS and the Rust steps are 1.x's.
- Web: wasm32 on the project toolchain, `wasm-bindgen-cli` only when `Cargo.lock` has `wasm-bindgen`, Homebrew's LLVM on macOS; no nightly (D3). Test: Node, pnpm, Playwright and its Chromium; `test_tools::playwright_chromium` finds that Chromium for `rayx app test --target web`. iOS: Xcode, simulator runtime, the device, simulator and Catalyst targets, no XcodeGen. Android: platform 35, build-tools 35.0.1, the pinned NDK (found through `ANDROID_NDK_HOME` as nsh finds it), three ABI targets, no cargo-ndk.
- Doctor (`src/doctor/`): every applicable set's steps, the pins with their sources, guests, the interactive desktop and WSL PATH findings; no GPU section.
- Guests (`guest.rs`, `wsl.rs`, `lima/`, `migrate.rs`, `refresh.rs`, `remove.rs`, `update.rs`, `src/wsl.rs`): as in 1.4.1, without the Windows-side GPU step; `wsl compact --clean` also removes `.nsh/`.

## Backlinks

- [Engine](../../../../src/setup/mod.rs), [records](../../../../src/setup/records.rs), [Linux](../../../../src/setup/linux.rs), [Windows](../../../../src/setup/windows.rs), [web](../../../../src/setup/web.rs), [test](../../../../src/setup/test_tools.rs), [iOS](../../../../src/setup/ios.rs), [Android](../../../../src/setup/android.rs)
- [Doctor](../../../../src/doctor/mod.rs), [guest layer](../../../../src/setup/guest.rs), [Lima](../../../../src/setup/lima/mod.rs), [WSL status and compact](../../../../src/wsl.rs)
- [Engine tests](../../../../tests/setup_engine.rs), [doctor tests](../../../../tests/doctor.rs)
- 2026-10-24 current [Decision: one plan for setup, --check and doctor](../../decisions/setup/2026-10-24-setup-one-plan-for-setup-check-and-doctor.md)
- 2026-10-24 current [Decision: the guest layer, records, refresh and replacement](../../decisions/setup/2026-10-24-setup-guest-layer-records-refresh-and-replacement.md)
- 2026-10-24 current [Decision: named Lima VMs, chosen releases, targeted update](../../decisions/setup/2026-10-24-setup-named-lima-vms-chosen-releases-and-targeted-update.md)
- 2026-10-24 current [Decision: WSL scripts run with --exec](../../decisions/setup/2026-10-24-setup-wsl-scripts-run-with-exec.md)
- 2026-10-24 current [Decision: Android Studio through its wizard](../../decisions/setup/2026-10-24-setup-android-studio-through-its-wizard.md)
