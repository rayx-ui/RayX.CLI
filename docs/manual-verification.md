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

## Lima guests on this Mac (2026-10-14)

macOS 26.6.2 on an Apple M5 (arm64, 10 cores, 24 GiB), Lima 2.2.1, branch `feat/lima-guests`
(`rayx` built with `cargo build --release`). The owner's `gpux` VM (6 CPUs, 12 GiB) was running the
whole time and was never started, stopped, edited or removed. Raw logs: `artifacts-temp/agentx/rayx-cli-guests/mac/`.

| Check | Command | Result |
|---|---|---|
| Dry run | `rayx setup --lima --vm-cpus 2 --vm-memory 4 --web --clone --check` | pass: the create, start, installer, clone and inner `setup --web --check` lines were printed; nothing was created and no record was written |
| MAN-1 create, customize prompt | `rayx setup --lima --vm-cpus 2 --vm-memory 4 --web --clone` in a pseudo-terminal | pass: `Create Lima VM rayx: 2 CPUs, 4 GiB memory, 100 GiB disk, home mounted read-only. Customize? [y/N]` answered `y`, then CPUs, memory, disk (Enter) and `Mount the home writable?` (`n`); the VM was created and started (the create line is the one `--check` printed in `01-check.log`: `--vm-type=vz`, `template:ubuntu-24.04`, `--mount-only=/Users/dinko`), the Linux `rayx` installed with the release installer (see Deviations), the checkout cloned to `~/RayX.CLI` |
| Inner setup | the same command, rerun with `--yes` | pass: the inner `rayx setup --web` installed 7 steps (apt packages, rustup, the project toolchain, the pinned nightly, `wasm-bindgen-cli`) in about 3 minutes; `guests.json` recorded kind, name, Ubuntu 24.04, `--web`, the clone and the VM settings |
| Home is read-only | `touch /Users/dinko/work-private/should-not-exist` inside the VM | pass: `Read-only file system`; the file does not exist on the Mac |
| MAN-1 second run | `rayx setup --lima --vm-cpus 2 --vm-memory 4 --web --clone --yes` | pass: no VM command beyond reads, the clone fetched, the inner setup printed `Nothing to do: everything is installed.` |
| MAN-1 doctor | `rayx doctor` | pass: `lima rayx: running, 2 CPUs, 4 GiB memory, 100 GiB disk, home read-only, Ubuntu 24.04, current` under `Lima: limactl 2.2.1`; `gpux` is not listed (not recorded) |
| MAN-1 edit | `rayx setup --lima --vm-cpus 3 --vm-memory 4 --web --yes` | pass: `05-edit.log` shows `changing the VM rayx: 2 -> 3 CPUs`, the stop, `Instance rayx configuration edited` and the start; `limactl list` shows 3 CPUs |
| MAN-2 refresh, host | `rayx setup` (no-op, records), then `rayx setup --update --check` and `--yes` | pass (`18-host-update.log`): the Mac was already set up, so `rayx setup` printed `Nothing to do` and wrote `setup.json` (`base`, this checkout); `--update --check` and `--update --yes` ran the recorded set in that project (`==> setup --update: /Users/dinko/work-private/projects/RayX/RayX.CLI`, everything installed, nothing changed); the record was removed afterwards |
| MAN-2 refresh, guest | Linux `rayx` built inside the VM (`cargo build --release`, 1m47s) and used through `RAYX_LIMA_BINARY`; `rayx setup --update --check` (`07-update.log`), then `--yes` (`08`–`10`) | pass: `--check` printed the guest's `rayx` and Ubuntu versions, the install and the apt commands and started nothing. The first `--yes` runs failed in `apt-get upgrade` on a transient mirror 404 (`07`, `08`; `rayx` reported the guest as failed); after `apt-mark hold u-boot-tools` in the test VM the refresh passed (`09`: binary installed, apt upgrade, inner `rayx setup --update --yes`, which printed `nothing recorded` because the guest's first setup had been run by the released 1.0.0 binary that does not record); `10` reran `setup --lima --web --clone --yes` with the branch binary, after which the guest's `setup.json` held `base`, `web` and the clone, and `setup --update --yes` printed `==> setup --update: /home/dinko.guest/RayX.CLI` and `Nothing to do` inside the guest |
| MAN-2 migration | a local build with the pin temporarily `26.04` (not committed): `rayx setup --lima --web` in a pseudo-terminal | pass: `Create rayx with Ubuntu 26.04 next to rayx-24.04? [y/N]` answered `y`; `limactl stop`, `limactl rename --tty=false rayx rayx-24.04`, `limactl create ... template:ubuntu-26.04` with the old 3 CPUs, 4 GiB, 100 GiB and the read-only home, start, the Linux `rayx` from `RAYX_LIMA_BINARY`, the clone re-cloned from the Mac checkout, `rayx setup --web` inside it; `13-migrate.log` also shows `rayx now runs Ubuntu 26.04` and the printed `rayx lima remove rayx-24.04`. Checked interactively right after (not saved as a log): `VERSION_ID=26.04` in the new VM, `guests.json` with `rayx-24.04` retired (24.04) and `rayx` current (26.04) with the clone and the VM settings, and the old VM stopped |
| MAN-2 removal | `rayx lima remove rayx-24.04`, `rayx lima remove gpux --yes`, `rayx lima remove rayx` | pass: each removal asked `Remove <name> and everything in it? [y/N]` (answered `y`); `gpux` was refused (`gpux was not created by rayx; remove it with limactl delete yourself`, exit 1) and kept running; the current guest's removal said `rayx setup --lima` creates it again; the record ended empty and `limactl list` showed only `gpux` |
| Smoke after the corrective sprint | `rayx setup --lima --vm-cpus 2 --vm-memory 4 --yes` (base set), a second run, `rayx doctor`; then a local 26.04-pin build `rayx setup --lima --vm-cpus 3 --yes`, then `rayx lima remove` of both | pass (`19`–`21`): created (`Installed 4 step(s)`), recorded with the VM settings, second run `Nothing to do`, doctor `lima rayx: running, 2 CPUs, 4 GiB memory, 100 GiB disk, home read-only, Ubuntu 24.04, current`; the migration stopped and renamed the old VM, created the new one with 3 CPUs (the flag laid over the old settings, no `limactl edit` of the old VM, which stayed at 2 CPUs), restored the setup and kept `rayx-24.04` retired; both were removed with `--yes`, `gpux` untouched, the temporary record removed |

Deviations from the criteria text:

- MAN-1 expects the Linux `rayx` to come from `RAYX_LIMA_BINARY`. The first install used the
  published v1.0.0 release installer instead, because no Linux build of this branch existed before
  the VM did; the released 1.0.0 binary ran the MAN-1 inner `setup --web`, the second run, doctor
  and the CPU change. The branch's own Linux binary (built inside the VM) was used through
  `RAYX_LIMA_BINARY` for the refresh (`07`–`10`) and the migration (`13`), so that path ran on a
  real VM too.
- The Linux binary for the migration was copied from the old VM to a directory under the Mac home
  (`~/.rayx-lima-proof`, removed afterwards) so the new VM could read it through the read-only mount.
- The MAN-2 migration used a local build with the pin temporarily `26.04` (`target/pin26`, never
  committed; `src/project/pins.rs` is back at `24.04`).

Findings fixed while verifying:

- `limactl rename` asks `Do you want to start the instance now? (Y/n)` when it has a terminal, so
  the migration waited on it forever. The rename (and the rollback rename) now pass `--tty=false`,
  and `tests/guest_migration.rs` checks the arguments. The interrupted run had renamed the VM before
  the record was written; the VM was renamed back by hand.
- The inner `rayx setup` asks apt's `Do you want to continue? [Y/n]` unless `--yes` is given (apt
  gets `-y` only with `--yes`), so an unattended first run needs `--yes`; this is the documented
  behaviour, not a defect.
- A transient 404 for one package (`u-boot-tools`) on the Ubuntu ports mirror made `apt-get upgrade`
  fail inside the guest; `rayx` reported the guest as failed and exited 1, and the refresh passed
  after `apt-mark hold u-boot-tools` in the test VM.
- Without `--clone`, the inner `rayx setup --web` runs in the guest's home, outside any project, so
  it installs a `stable` toolchain and its wasm target; run with `--clone` to set up the project.

## Open: Windows and WSL guest layer, and the first release (owner-run)

Rows without a result are still open (epic `rayx-cli-guests`, MAN-3 and MAN-4). Fill in the date,
the versions and the result when you run them.

Windows 11 PC with WSL 2 and `Ubuntu-24.04` (MAN-3), with a local Windows build
(`cargo build --release`) and a Linux build for `RAYX_WSL_BINARY` (built inside the distribution
with `cargo build --release`, then the `\\wsl$\...` path or a copy on the Windows disk):

| Check | Command | Result |
|---|---|---|
| WSL setup on the guest layer | `rayx setup --wsl --web --clone --yes` from a checkout on the Windows disk | pass (2026-10-14, exit 0), twice. First with the Linux `rayx` 1.0.0 from the release installer (`installing the Linux rayx 1.0.0`, `~/.rayx/bin`), then with `RAYX_WSL_BINARY` (`installing the Linux rayx from /mnt/c/...`). The existing clone `~/RayX.CLI` was fetched into, not re-cloned; the inner `rayx setup --web --yes` said `Nothing to do: everything is installed.` |
| The guest is recorded | `%LOCALAPPDATA%\rayx\guests.json` lists `Ubuntu-24.04` (kind `wsl`, flags, the clone) and `setup.json` is untouched by `--wsl` | pass: `kind` `wsl`, `name` `Ubuntu-24.04`, `ubuntu` `24.04`, `flags` `["--web"]`, one clone (`source` `D:/Work/projects/RayX/RayX.CLI`, `branch` `feat/lima-guests`, `target` `$HOME/RayX.CLI`), `retired` false. No `setup.json` exists in `%LOCALAPPDATA%\rayx` before or after |
| The in-guest clones file | `~/.config/rayx/clones` inside the distribution lists the clone; `rayx wsl status` still shows its build output | pass: `clones` holds `/home/dinko/RayX.CLI`. `rayx wsl status` said `build output none` until `cargo build --release` ran in the clone, then `build output  381.9 MiB under the cloned checkouts` with `/home/dinko/RayX.CLI/target` |
| Refresh | `rayx setup --update --yes` | pass with the `RAYX_WSL_BINARY` build (exit 0): `refreshing Ubuntu-24.04`, `installing the Linux rayx from /mnt/c/...`, `sudo apt-get update && sudo apt-get upgrade -y` (0 upgraded, 2 held back by phasing), `rayx setup --update --yes` inside, which printed `==> setup --update: /home/dinko/RayX.CLI` and `Nothing to do: everything is installed.`. **fail** with the release `rayx` 1.0.0 as the Linux `rayx` (first run, exit 1): the apt step upgraded 52 packages, then the inner 1.0.0 does not know the flag: `error: unexpected argument '--update' found` (`exited with code 2`). The first refresh needs a Linux `rayx` that has `setup --update`, so it needs the next release, or `RAYX_WSL_BINARY`. In between, with the inner `setup.json` still missing, the inner step printed `nothing recorded: run `rayx setup` first` and exited 0 |
| Doctor | `rayx doctor` | pass (three runs, exit 0): `wsl Ubuntu-24.04: running, disk 214.6 GiB, Ubuntu 24.04, current` under `Guests:`, plus a `WSL disk:` line for each distribution. The 1 missing requirement is the Android AVD `rayx-android-34-x86_64`, which is unrelated |
| Removal refusal | `rayx wsl remove not-a-rayx-guest-test --yes` (a name `rayx` did not create; do not use a real distribution) | pass: `rayx: not-a-rayx-guest-test was not created by rayx; remove it with wsl --unregister yourself`, exit 1; `wsl -l -q` still lists `Ubuntu-24.04` and `docker-desktop` |

Run on 2026-10-14: Windows 10.0.26300.9457 (Windows 11 Pro), WSL 2.6.3.0 (kernel 6.6.87.2-1, WSLg
1.0.71), `rayx 1.0.0` built from `feat/lima-guests` at `12adb9a`, `Ubuntu-24.04` (24.04.4 LTS, user
`dinko`, uid 1000). The Linux `rayx` of the first pass came from the release installer (`rayx 1.0.0`);
the second pass used `RAYX_WSL_BINARY`, a `cargo build --release` of this branch made inside the
distribution (`CARGO_TARGET_DIR` outside the clone) and copied to the Windows disk. The `sudo`
password was typed by the owner in the terminal.

The refresh failure with the release `rayx` 1.0.0 is a property of this run, not of the product: the
branch is still versioned 1.0.0, so the host's `rayx 1.0.0` found the guest's released `rayx 1.0.0`
"matching" and did not replace it, and that binary does not know `setup --update`. A release that
contains the epic has a higher version, so the refresh installs the matching Linux `rayx` first.

Seen on the way (no change needed for the rows):

- The release installer printed `WARN(B: The following commands are shadowed by other commands in
  your PATH: rayx`, with a stray `(B` (an unreset terminal escape sequence), because an older
  `~/.local/bin/rayx` (0.1.0) of this distribution is shadowed by `~/.rayx/bin/rayx`.
- `rayx` sets no `DEBIAN_FRONTEND`, so `apt-get upgrade` in a small terminal printed `debconf: unable
  to initialize frontend: Dialog` and fell back to Readline; harmless, but `--yes` could run apt
  non-interactively.

Runbook for the Windows rows (PowerShell, from the checkout on the Windows disk; it logs everything
to `rayx-man3.log` next to you, which you can paste into the table above):

```powershell
cargo build --release
$rayx = "$PWD\target\release\rayx.exe"
# Optional: a Linux build for the guest, made inside the distribution and copied to the Windows disk.
# $env:RAYX_WSL_BINARY = "D:\rayx-linux\rayx"
Start-Transcript -Path rayx-man3.log
Get-Date -Format o
& $rayx setup --wsl --web --clone --yes          # row 1: WSL setup on the guest layer
Get-Content "$env:LOCALAPPDATA\rayx\guests.json" # row 2: Ubuntu-24.04 is recorded
wsl -d Ubuntu-24.04 --exec bash -lc 'cat ~/.config/rayx/clones'   # row 3 (and `rayx wsl status`)
& $rayx wsl status
& $rayx setup --update --yes                      # row 4: refresh (apt upgrade + inner update)
& $rayx doctor                                    # row 5: the guest report
& $rayx wsl remove not-a-rayx-guest-test --yes    # row 6: refused (a name rayx did not create); never use a real distribution here
Stop-Transcript
```

First release that contains the epic (MAN-4), from the previous release on the Mac and on Windows:

| Check | Command | Result |
|---|---|---|
| Update offers the refresh | `rayx self update` | deferred (owner, 2026-10-14: no release contains the epic yet; follow-up: run these two rows after the first release that does): expect `Refresh this machine's setup and guests with rayx <new>? [y/N]`; `y` runs the new binary's `setup --update` and the guest is refreshed; `n` prints ``run `rayx setup --update` to refresh the setup and guests`` |
| Unattended | `rayx self update --yes` on the next release | deferred (same follow-up) |

## Linux (hosted CI)

`rayx setup --web --test --yes` and the ignored heavy groups run on `ubuntu-latest`; `rayx setup
--web` runs for real on fresh Ubuntu, macOS and Windows runners (the `setup` job in
`.github/workflows/ci.yml`). WSL Ubuntu 24.04 on the owner's PC passes the same flow above.

## macOS and iOS (this Mac, 2026-10-08/09)

macOS 26.6.2 (25G83) on an Apple M5 (arm64), Xcode 26.4.1, iOS Simulator runtimes 26.4 and 26.5,
branch `feature/initial-setup`. Every failure found was fixed in this repository with a test; the
ones that belong to RayX or Gpux are listed under "Changes for the siblings".

| Check | Command | Result |
|---|---|---|
| Machine | macOS, Xcode, chip | macOS 26.6.2, Xcode 26.4.1 (newest stable, license and first launch already done), Apple M5 `arm64`, Metal 4 adapter |
| Test suite | `cargo test --no-fail-fast` | pass: 398 passed; the one printed failure is `the_fixture_app_fails`, which fails an app's own test on purpose. The heavy groups (`app_wasm_fixture`, `app_playwright_template`, `--include-ignored`) pass |
| Doctor | `rayx doctor` | pass, with one lie found and fixed: it said `present Homebrew` for Homebrew 3.5.6 (2022), which crashes on macOS 26.6.2 (`MacOSVersionError`). The probe now runs `brew config` and reports an unusable Homebrew as outdated. Every other missing item came with a working fix command |
| Base, web, test sets | `rayx setup --web --test --yes`, then `--check`, then a second run | pass after fixes: Homebrew had to be updated first (owner approved; it needs `git` over HTTPS because the owner's `~/.gitconfig` rewrites GitHub URLs to SSH), then `brew install node@22` was not seen because nvm's Node 17 was first on PATH (fixed: `select_node`). Then `--check` exits 0 ("Everything is installed") and the second run says "Nothing to do" |
| iOS set | `rayx setup --ios --yes`, then `--check --ios` | pass: Xcode, `xcode-select`, license, first launch and the Simulator runtime were already present and probed green; XcodeGen (Homebrew) and `aarch64-apple-ios`, `aarch64-apple-ios-sim` were installed; `--check` exits 0, a second run installs nothing |
| Fixtures | `rayx app … pack host`, `test host`, `build wasm`, `test wasm` | pass: packaged `rayx_demo` and `assets`; the threaded WASM build is verified (shared memory, atomics); the headless Chromium smoke spec passes on both device scale factors |
| Real RayX web | Lab and Test Suite `build wasm`, headed WebGPU spec | pass after a fix: the build scripts compiled C with Apple's clang, which has no wasm32 backend (`arborium-sysroot`); `rayx` now points `CC`, `CXX` and `AR` at Homebrew's LLVM. The headed WebGPU spec `rayx-collapsible-sizes.spec.ts` passes on the Mac's GPU (2 passed) in the real checkout against the current `RayX.Dtcg` `main`, once RayX opens its themes with the RayX value extensions (see below) |
| Real RayX macOS | `rayx app apps/lab build host` | pass: release build in 3 min 30 s, `target/release/rayx_lab` |
| iOS build | `rayx app apps/lab build ios --simulator` | pass after four fixes: the generated entry crate had no `Cargo.lock` and resolved `libc` 0.2.190, which `backtrace` 0.3.76 does not compile against for iOS (the entry crate now starts from the project's lock); the Xcode Rust phase did not declare its output (RayX `project.yml`); `io-surface` references macOS-only CGL symbols that `-force_load` made the linker resolve (RayX `project.yml`: dead-code stripping); 3226 packaged assets verified in the `.app` |
| iOS run | `rayx app apps/lab run ios --simulator` | pass: the real RayX Lab renders in the iPhone 17 Simulator (gallery sidebar, icons, themed content, `175 components · dark · cyan · mobile · v0.5.1`, no errors in the log). Found and fixed on the way: `rayx` ran `simctl boot Shutdown` (the state word, not the device id); Gpux asked Vulkan and GL for an adapter and found none on iOS (black window); the asset service never looked inside the `.app` bundle on iOS; `main.m` read the window before the async app callback created it; `RayX.Dtcg`'s strict 2025.10 validation rejected RayX's dimension line heights; the iOS runner built the Gpux application without an asset source (no images or icons); and the platform theme context defaulted to `desktop` |
| iOS pack | `rayx app apps/lab pack ios --simulator` | pass after a fix: `pack ios` rejected `--simulator`; it takes the same selectors as `build ios` now. It still only prepares the output directory and reports Xcode's build directory |
| `examples_mobile` | `rayx app apps/examples_mobile run ios --simulator` | pass after Gpux fixes (36 `extern "C"` blocks that edition 2024 requires to be `unsafe`): the home screen with its cards and tab bar renders |
| Smoke app | `rayx app apps/mobile_smoke run ios --simulator` | pass: a new RayX app with no theming and no SignalR draws four coloured stripes through Gpux's Metal backend on the iPhone 17 Simulator, built and launched by `rayx` alone |
| WASM run | `rayx app apps/lab build wasm`, then `run wasm` | pass: 1 min 17 s build, threaded WASM verified; served on `127.0.0.1:7878`, the Lab renders in the browser pane with `crossOriginIsolated` and WebGPU available |
| Android set | `rayx setup --android --yes` in the Terminal panel, then `--check --android` | pass: JDK 21 cask (sudo prompt answered by the owner), SDK licenses accepted by `--yes`, `cargo-ndk`, `aarch64-linux-android` and `x86_64-linux-android`; `--check` exits 0. Finding: without `--yes` and a terminal the JDK "update?" question defaults to no and setup stops at `JAVA_HOME` |
| Android build and run | `rayx app apps/examples_mobile build android`, then `run android` | pass after three fixes: `gradlew` is committed without its executable bit (a Windows-authored project), so Gradle never started (now run through `sh`); a shell that predates `rayx setup` has no `JAVA_HOME` and Gradle fell back to JDK 26, whose `jlink` cannot build the Android JDK image (the build now finds the pinned JDK itself and says which); the emulator launcher took the first AVD listed, a user AVD whose system image was not installed, and waited 3 minutes with the emulator's output discarded (it now prefers the `rayx-` AVD, reports an emulator that exits, and tries the next AVD). The emulator also starts with `-gpu host` first: macOS's `auto` gives only software Vulkan, which the RayX renderer rejects (the app starts and never opens a window). Examples Mobile then installs and renders on the Android 34 arm64 emulator. Follow-up: the setup probes were loose (any AVD satisfied the default-AVD check, any NDK satisfied the NDK pin), so `doctor` called a Mac with only a broken `Pixel_9_Pro_XL` and NDK 27.2 complete. Both now require the pinned item; after installing `ndk;28.0.12674087` the build reports and uses NDK 28.0 although 27.2 is still on disk. Re-tested with the working `rayx-` AVD deleted: `doctor` and `setup --check` report it missing and name `Pixel_9_Pro_XL` as the only other AVD, `rayx setup --android` recreates it unattended, and `run android` then boots it with `-gpu host` (the new AVD has `hw.gpu.enabled=no`) and the app renders |

### Changes for the siblings

These are committed and pushed (no pull request opened) in `../RayX` (branch `fix/ios-simulator-run`; its `Cargo.lock` is the one `RayX.SignalR` 0.2.0 resolves to) and `../Gpux` (branch
`fix/ios-simulator-rendering`), because they belong to those repositories; RayX's commits depend on
the Gpux ones:

- RayX, to run Gradle on Linux and macOS: the executable bit on the three `platform/android/gradle/gradlew`
  files (they were committed as `100644`; staged, not yet committed). `rayx` also runs the wrapper
  through `sh`, so a checkout without the bit still builds.
- RayX, to run on iOS: the three iOS `project.yml` files (`outputFiles` on the Rust script phase
  and `DEAD_CODE_STRIPPING` in Release); the `main.m` of the Lab and `examples_mobile` (the display
  link starts always and looks the window up each frame, because the app callback is async and the
  window appears after `didFinishLaunching`); `crates/rayx/src/application/entry.rs` (installs the
  iOS stderr logger first, so a startup error is not lost); `rayx_runtime`'s asset options, where
  `<app>.app/assets` is now the installed root on iOS (renamed `native_installed_public_root`);
  the new `apps/mobile_smoke`, a RayX app with no theming and no SignalR, and its workspace entry.
- RayX and Gpux, for images and icons on iOS: `gpui_mobile::ios::ffi::set_app_assets` (Gpux) and
  `register_built_app` calling it with the asset service (RayX); the Lab selects the `mobile`
  platform context on iOS and Android.
- RayX, for the `RayX.Dtcg` refactoring (the steps of Dtcg's `CHANGELOG.md` migration section):
  `rayx_theme` opens projects with `RuntimeOptions … ValueExtensions::rayx()` (the design system
  aliases dimension line heights and uses `dev.rayx.blur`, which strict DTCG 2025.10 rejects),
  maps `DtcgError::UnresolvedReference` to a token-shaped `ThemeError`, and its tests and fixtures
  follow the stricter structure rules (`ModifierAxis` is `#[non_exhaustive]`; `x-resolver`,
  `unknown` and `x-unknown` moved under `$defs` and `$extensions`). All ten `rayx_theme` test
  groups pass.
- RayX, for the `RayX.SignalR` refactoring: no source change is needed. RayX's committed
  `Cargo.lock` was made against `RayX.SignalR`'s `feat/production-grade-client` (0.2.0), and `rayx`
  and `rayx_lab` check cleanly against that branch (exported to a scratch directory, not checked
  out) and against the `master` (0.1.0) the sibling checkout is on.
- Gpux: `gpui_wgpu` chooses Metal on Apple platforms (it asked Vulkan and GL, found no adapter and
  left every iOS window black); `gpui_mobile` makes the 36 `extern "C"` blocks `unsafe extern "C"`
  and installs a stderr logger for `log` on iOS (`GPUI_IOS_LOG=debug`, passed to the Simulator as
  `SIMCTL_CHILD_GPUI_IOS_LOG=debug`), which was the only way to see any of the above.

### Open

- `../RayX.SignalR` was moved from `master` (0.1.0) to `feat/production-grade-client` (0.2.0), the
  branch RayX's lock was made against; cargo no longer rewrites RayX's lock on every run.
- The Lab keeps its desktop shell on a phone: a two-pane layout and its own menu bar over the status
  bar. That is the Lab's UI design (no responsive or safe-area layout), not the pipeline.
- No pull requests are open for `fix/ios-simulator-rendering` (Gpux) and `fix/ios-simulator-run` (RayX).
- Homebrew's own remote URLs were SSH (`git@github.com:`) and the owner's global `url.insteadOf`
  sends HTTPS to SSH too, so `brew update` failed until Homebrew was updated with
  `GIT_CONFIG_GLOBAL=/dev/null git fetch` and `git checkout -B stable <tag>`.

## Release v1.0.0

Published 2026-10-14 by release-plz and the Release workflow (run 38565448357): the tag `v1.0.0` was
pushed by the first `release-plz` run on `main` (the untagged `1.0.0` in `Cargo.toml` counts as an
initial release, so no release pull request was needed for it). The GitHub Release is Latest, not a
prerelease, with the six archives and their `.sha256` files, `rayx-cli-installer.sh`,
`rayx-cli-installer.ps1`, `sha256.sum` and the notes of the `CHANGELOG.md` section.
`gh attestation verify` succeeds for the macOS ARM64, Windows ARM64 and Linux x64 archives and fails
for a modified copy.

| Host | Check | Result |
| --- | --- | --- |
| macOS ARM64 (this Mac) | `curl ... releases/latest/download/rayx-cli-installer.sh \| sh` on a host without `rayx` | pass: `~/.rayx/bin/rayx`, receipt `~/.config/rayx-cli/rayx-cli-receipt.json`, PATH from `~/.config/rayx-cli/env.sh` sourced in `.zshrc`, `.profile` and `.bash_profile` |
| macOS ARM64 | new shell: `which rayx`, `rayx --version`, `rayx self update --check` | pass: `/Users/dinko/.rayx/bin/rayx`, `rayx 1.0.0`, `installed 1.0.0, latest release 1.0.0: rayx is up to date` |
| macOS ARM64 | running the installer again | pass: reinstalls in place without error, `rayx 1.0.0` |

Pending follow-up (owner decision 2026-10-14: these need the owner's Windows PC and are not part of
the `rayx-cli-releases` epic; run them and record the results here):

- Windows x64: the PowerShell one-liner on a host without `rayx`; a new terminal finds `rayx` (user
  `Path` has `%USERPROFILE%\.rayx\bin`), `rayx --version` prints `rayx 1.0.0`,
  `rayx self update --check` reports up to date, and running the installer again works.
- Linux x64 in WSL, and `rayx setup --wsl` from the v1.0.0 Windows `rayx`: installs `rayx 1.0.0` into
  `~/.rayx/bin` of `Ubuntu-24.04`, a new WSL shell finds it, and `rayx self update --check` inside WSL
  reports up to date instead of the no-receipt message.
- RayX CI (`rayx-ui/RayX#2`, pins the v1.0.0 installer in `pr-test.yml`): after the GitHub Actions
  billing was fixed the jobs ran for the first time (2026-10-14, attempt 5). The release-installed
  `rayx` works: `Install rayx` (the v1.0.0 installer, `~/.rayx/bin` on `GITHUB_PATH`), `Install the
  web set` and `Install the web and test sets` pass in both jobs. Both jobs then fail in steps that
  have nothing to do with the pin: `cargo fmt --check` and `rayx app ... test wasm` cannot load the
  workspace because RayX's `Cargo.toml` references the sibling `../RayX.SignalR` by path and the
  workflow never checks it out. RayX `master`'s earlier runs (2026-10-06, 2026-10-08) never got past
  checkout or job startup, so this failure is first seen on PR #2 and is independent of the pin. Owner
  decision: RayX is fixed separately so it builds without that sibling; this repository does not
  work around it. Re-run `rayx-ui/RayX#2` after that fix; both jobs (Rust Workspace, WASM
  Playwright WebGPU) must pass.
