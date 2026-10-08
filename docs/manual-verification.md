# Manual verification

What automated tests and hosted CI runners cannot prove is run by hand and recorded here. Each row
names the machine, the command, the result and the date. A row without a result is still open.

## Windows 11 and WSL (owner's PC, 2026-10-08)

Windows 11 Pro 10.0.26300, WSL 2, Ubuntu-24.04 (default user uid 1000), Docker Desktop installed.

| Check | Command | Result |
|---|---|---|
| Doctor on Windows | `rayx doctor` | pass: host, GPU adapter, toolchains and the Visual Studio C++ compilers reported truthfully |
| Setup is idempotent | `rayx setup --web --test --android --yes`, then `--check` | pass: second run installs nothing, `--check` exits 0 |
| Android Studio install | `rayx setup --android --yes` | pass: `winget install --interactive` shows the installer, the owner accepts the license and UAC, Studio is installed afterwards |
| App pipeline on RayX | Lab and Test Suite `build wasm`, Lab `build host`, Examples Mobile `build android`, headed WebGPU spec | pass |
| WSL-1 end-to-end | `rayx setup --wsl --web --clone` | pass: Linux `rayx` installed from `RAYX_WSL_BINARY`, checkout cloned to `~/RayX.CLI`, inner `rayx setup --web` installed the pinned nightly ("Installed 1 step(s)") |
| WSL-2 clone is recorded | `~/.config/rayx/wsl-clones` | pass: the clone directory is listed |
| WSL-3 status | `rayx wsl status` after a `cargo build` in the clone | pass: virtual disk path and sizes, and the clone's `target` (989.5 MiB) listed |
| WSL-4 compaction | `rayx wsl compact --clean --yes` | pass: build output removed, `fstrim` trimmed 805 GiB of free space, one UAC prompt, diskpart attached, compacted and detached the disk: 204.8 GiB before, 204.3 GiB after (544.0 MiB given back) |
| Status after compaction | `rayx wsl status` | pass: "build output none under the cloned checkouts", disk 204.3 GiB |

Findings fixed while verifying:

- `wsl compact` printed PowerShell's CLIXML progress records ("Preparing modules for first use")
  after diskpart. The compact script now sets `$ProgressPreference = 'SilentlyContinue'`.
- The disk gave back little because the Ubuntu disk is mostly data (197.9 GiB used). The
  compaction itself is proven; the size gained depends on how much was freed inside.
- The "installing the Linux rayx from" message prints a `\\?\D:\…` verbatim path (cosmetic).

## Linux (hosted CI)

`rayx setup --web --test --yes` and the ignored heavy groups run on `ubuntu-latest`; `rayx setup
--web` runs for real on fresh Ubuntu, macOS and Windows runners (the `setup` job in
`.github/workflows/ci.yml`). WSL Ubuntu 24.04 on the owner's PC passes the same flow above.

## macOS and iOS (open)

To be run on a Mac by following [HANDOFF.md](../HANDOFF.md). Record the results here.

| Check | Command | Result |
|---|---|---|
| Machine | macOS version, Xcode version, chip | |
| Test suite | `cargo test --no-fail-fast` | |
| Doctor | `rayx doctor` truthful, every missing item has a fix | |
| Base, web, test sets | `rayx setup --web --test --yes`, then `--check` | |
| iOS set | `rayx setup --ios --yes`, then `--check --ios` | |
| Fixtures | `rayx app … pack host`, `build wasm`, `test wasm` | |
| Real RayX web | Lab and Test Suite `build wasm`, headed WebGPU spec | |
| Real RayX macOS | Lab `build host` | |
| iOS build | `rayx app apps/lab build ios --simulator` | |
| iOS run | `rayx app apps/lab run ios --simulator` | |
| iOS pack | `rayx app apps/lab pack ios --simulator` | |

## Not verifiable before a release

The first tagged release (cargo-dist builds of all six targets, the installers) and a real `rayx
self update` download need a published release; `self update --check` reports that none exists.
