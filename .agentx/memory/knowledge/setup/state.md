---
agentx:
  kind: state
  scope: knowledge/setup
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T01:30:00+02:00
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

Base set implemented in `src/setup/` (epic `rayx-cli-foundation`, design decision `d-setup-plan`, sprint 2); the web, test, Android, iOS and GPU sets, `doctor` and WSL come in later sprints (`plan()` returns `SetNotAvailable` for those sets). `src/setup/mod.rs` is the engine: `Set`, `PlanEnv`, `Step { probe, install }`, `Action` (`Run`, `AddToPath`, `WriteFile`, `RemoveFile`, `Custom`), `plan`, `check`, `execute`, `render_plan` and `run_plan`; `machine.rs` is the `Machine` observation trait (`SystemMachine`, `FakeMachine`). `--check` prints each step's status and exact commands and exits 1 when anything is missing; a run executes only missing steps, re-probes after each, stops at the first failure and ends with "Nothing to do" the second time. Apt steps batch into one `apt-get update` plus one `apt-get install` (`-y` only with `--yes`).

- `linux.rs`: Ubuntu/Debian (and `ID_LIKE` derivatives, also in WSL) apt list from SETUP-2, probed with `dpkg-query` (status `i` in the second `Status-Abbrev` character); other distributions fail with a message naming Ubuntu and Debian.
- `rust.rs`: rustup (`curl | sh` with `--default-toolchain none --no-modify-path`), the project's `rust-toolchain.toml` toolchain installed from the project directory (else `stable` plus `rustup default stable`), `~/.cargo/bin` on the user PATH. On Windows the toolchain must carry the native triple.
- `windows.rs`: winget (missing winget stops with the App Installer instruction), Build Tools (VCTools, plus the ARM64 component on ARM64; an existing Visual Studio is modified with its `setup.exe`), VC++ redistributables per architecture, LLVM and its PATH entry, the newest Windows SDK `fxc.exe` directory for the host architecture (fallback to the other) on PATH, rustup from `rustup-init.exe` for the native triple, `rustup set default-host`.
- `macos.rs`: Command Line Tools through the on-demand marker, the newest `softwareupdate` label and `sudo softwareupdate --install` (fallback `xcode-select --install` and wait; `Cx::with_clt_marker` redirects the marker for tests), Homebrew through the official script (a `sudo -v` first under `--yes`, `/opt/homebrew/bin` on PATH on Apple silicon), then the Rust steps. One step table feeds `setup`, `setup --check` and `doctor`. It replaces gpux's `LINUX-WSL.md` sections 2–4, the gpux skill's prerequisite references and `install-windows-prereqs.ps1`, and RayX xtask's `setup-android` and `setup-wasm`.

## Backlinks

- [Setup engine](../../../../src/setup/mod.rs), [Linux](../../../../src/setup/linux.rs), [Windows](../../../../src/setup/windows.rs), [macOS](../../../../src/setup/macos.rs), [Rust steps](../../../../src/setup/rust.rs)
- [Setup tests](../../../../tests/setup_engine.rs)
- [Epic design (`d-setup-plan`, `d-wsl`)](../../../tasks/rayx-cli-foundation/design.json)
- [gpux Linux on WSL guide](../../../../../Gpux/LINUX-WSL.md)
- [gpux Windows prerequisites](../../../../../Gpux/.claude/skills/gpux/references/windows-prerequisites.md)
- [gpux macOS, Linux and WASM prerequisites](../../../../../Gpux/.claude/skills/gpux/references/macos-linux-wasm-prerequisites.md)
- [RayX xtask setup-android](../../../../../RayX/xtask/src/android.rs)
- [RayX xtask setup-wasm](../../../../../RayX/xtask/src/node_tools.rs)
