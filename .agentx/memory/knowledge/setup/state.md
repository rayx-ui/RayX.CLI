---
agentx:
  kind: state
  scope: knowledge/setup
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T05:00:00+02:00
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

App commands use the engine through `src/app/prereqs.rs` (`Step::prompts`, `Step::optional_for_builds`, `environment_for`). The Chromium probe finds `playwright-core` in pnpm's store (`node_modules/.pnpm/playwright-core@*`), not only at the top level. All requirement sets, `doctor`, `setup --wsl` and `wsl status`/`compact` are implemented in `src/setup/`, `src/doctor.rs`, `src/setup/wsl.rs` and `src/wsl.rs` (epic `rayx-cli-foundation`, design decisions `d-setup-plan` and `d-wsl`). `src/setup/mod.rs` is the engine: `Set`, `PlanEnv`, `Step { probe, install }`, `Action` (`Run`, `AddToPath`, `WriteFile`, `RemoveFile`, `Custom`), `plan`, `check`, `execute`, `render_plan` and `run_plan`; `machine.rs` is the `Machine` observation trait (`SystemMachine`, `FakeMachine`). `--check` prints each step's status and exact commands and exits 1 when anything is missing; a run executes only missing steps, re-probes after each, stops at the first failure and ends with "Nothing to do" the second time. Apt steps batch into one `apt-get update` plus one `apt-get install` (`-y` only with `--yes`).

- `linux.rs`: Ubuntu/Debian (and `ID_LIKE` derivatives, also in WSL) apt list from SETUP-2, probed with `dpkg-query` (status `i` in the second `Status-Abbrev` character); other distributions fail with a message naming Ubuntu and Debian.
- `rust.rs`: rustup (`curl | sh` with `--default-toolchain none --no-modify-path`), the project's `rust-toolchain.toml` toolchain installed from the project directory (else `stable` plus `rustup default stable`), `~/.cargo/bin` on the user PATH. On Windows the toolchain must carry the native triple.
- `windows.rs`: winget (missing winget stops with the App Installer instruction), Build Tools (VCTools, plus the ARM64 component on ARM64; an existing Visual Studio is modified with its `setup.exe`), VC++ redistributables per architecture, LLVM and its PATH entry, the newest Windows SDK `fxc.exe` directory for the host architecture (fallback to the other) on PATH, rustup from `rustup-init.exe` for the native triple, `rustup set default-host`.
- `macos.rs`: Command Line Tools through the on-demand marker, the newest `softwareupdate` label and `sudo softwareupdate --install` (fallback `xcode-select --install` and wait; `Cx::with_clt_marker` redirects the marker for tests), Homebrew through the official script (a `sudo -v` first under `--yes`, `/opt/homebrew/bin` on PATH on Apple silicon), then the Rust steps. One step table feeds `setup`, `setup --check` and `doctor`. It replaces gpux's `LINUX-WSL.md` sections 2–4, the gpux skill's prerequisite references and `install-windows-prereqs.ps1`, and RayX xtask's `setup-android` and `setup-wasm`.

- `web.rs`: the pinned nightly with `rust-src` and the wasm32 target, the wasm32 target on the project's toolchain, `wasm-bindgen-cli` at the locked version (`cargo install ... --force --locked`), and Homebrew's LLVM on macOS (Apple's clang has no wasm32 backend; Linux and Windows get clang from the base set).
- `test_tools.rs`: Node at the pinned major (NodeSource on Ubuntu, `OpenJS.NodeJS.LTS`, Homebrew `node@<major>`), pnpm through `corepack enable --install-directory ~/.local/bin`, xvfb/xdotool and the Chromium libraries (`t64` names on Ubuntu 24.04 and Debian 13), and the project's Playwright package (`tools-node` or `[workspace.metadata.rayx] tools-node`): `pnpm install --frozen-lockfile --prefer-offline` (redone when `pnpm-lock.yaml` is newer than `node_modules/.modules.yaml`) and `pnpm exec playwright install chromium` into the per-user cache with `PLAYWRIGHT_BROWSERS_PATH` set explicitly. `find_tool` resolves node/corepack/pnpm on PATH (skipping `/mnt/*` under WSL) and in the install locations, and `with_node_path` prefixes the command's PATH with Node's directory because the corepack shim starts `node` by name.
- `android.rs`: JDK 21 + `JAVA_HOME` (user variable), Android Studio (skipped under WSL and without a desktop), SDK found in xtask order, command-line tools downloaded when missing, one `sdkmanager --install` for the missing packages (existing NDK honoured, system image at the platform pin for the host ABI), default AVD `rayx-<platform>-<abi>` (emulator log lines ignored), acceleration (kvm group, elevated `dism` for the Windows Hypervisor Platform with a restart notice), `cargo-ndk` and the Android Rust targets. `.bat` tools start by full path.
- `ios.rs` (macOS only): Xcode versions read from `version.plist` (no `xcodebuild` needed), `xcodes`/`mas`, `xcode-select --switch` to the pin-matching app, license (`accept` only with `--yes`), first launch, Simulator runtime, XcodeGen, iOS targets.
- `gpu.rs`: adapter detection (DXGI through the `windows` crate, `lspci` + `vulkaninfo`, `powershell.exe` under WSL, `system_profiler`) and one driver step. Checked 2026-10-08: winget has no NVIDIA, AMD or Qualcomm display driver package, so Windows Update is used (UsoClient scan + optional-updates page); Intel adds `Intel.IntelDriverAndSupportAssistant`; Ubuntu NVIDIA uses `ubuntu-drivers install`; Mesa covers AMD/Intel; virtual adapters are informational.
- `doctor.rs`: requirements as present/missing/wrong-version with `found` versions and exact fixes, GPU adapters, `hardwareAdapter` (needs a working driver), `interactiveDesktop` (console session / DISPLAY / Aqua), WSL findings and, from Windows, the `ext4.vhdx` sizes (`src/wsl.rs`). Exit 0 whenever the host could be inspected.

## Backlinks

- [Setup engine](../../../../src/setup/mod.rs), [Linux](../../../../src/setup/linux.rs), [Windows](../../../../src/setup/windows.rs), [macOS](../../../../src/setup/macos.rs), [Rust steps](../../../../src/setup/rust.rs)
- [Setup tests](../../../../tests/setup_engine.rs)
- [Epic design (`d-setup-plan`, `d-wsl`)](../../../tasks/rayx-cli-foundation/design.json)
- [gpux Linux on WSL guide](../../../../../Gpux/LINUX-WSL.md)
- [gpux Windows prerequisites](../../../../../Gpux/.claude/skills/gpux/references/windows-prerequisites.md)
- [gpux macOS, Linux and WASM prerequisites](../../../../../Gpux/.claude/skills/gpux/references/macos-linux-wasm-prerequisites.md)
- [Android set](../../../../src/setup/android.rs)
- [Test set (Playwright and Node)](../../../../src/setup/test_tools.rs)

- `setup/wsl.rs` (`setup --wsl [--clone [<dir>]]`, Windows only): requires `wsl --version` to print a dotted version (the label is localized), installs `Ubuntu-24.04` when absent and stops for the first user, installs the Linux `rayx` of the same version (release asset `rayx-cli-<arch>-unknown-linux-musl.tar.xz`, or `RAYX_WSL_BINARY`), runs `rayx setup --gpu` on Windows first for `--gpu`/`--all`, then `wsl -d Ubuntu-24.04 --exec bash -lc 'rayx setup <flags>'` in the same console. `--clone` clones only the current checkout through `/mnt/<drive>` (absolute or `~` target only), fetches an existing clone, and records it in `~/.config/rayx/wsl-clones`.
- `src/wsl.rs` (`wsl status [--json]`, `wsl compact [--distro] [--clean] [--yes]`): registry view of the distributions, `ext4.vhdx` sizes, `df` and build-output sizes of recorded clones; compaction deletes only directories named `target`, `artifacts` or `artifacts-temp` under recorded clones, runs `fstrim`, asks before `wsl --shutdown`, and compacts in one elevated PowerShell (`-EncodedCommand`, the disk path passed as base64 data, `Optimize-VHD` with a `diskpart` fallback that always detaches, exit code from `diskpart`). It never enables sparse disks. Verified on the development machine: `wsl status` (Ubuntu-24.04 vhdx 199.2 GiB on Windows, 192.5 GiB used inside). Not yet run with the owner's approval: a real compaction and the end-to-end `setup --wsl`.
- Android Studio is installed through winget `--interactive` and is `optional_for_builds` (see the decision of 2026-10-08).
