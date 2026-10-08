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
| Android build and run | `rayx app apps/examples_mobile build android`, then `run android` | pass after three fixes: `gradlew` is committed without its executable bit (a Windows-authored project), so Gradle never started (now run through `sh`); a shell that predates `rayx setup` has no `JAVA_HOME` and Gradle fell back to JDK 26, whose `jlink` cannot build the Android JDK image (the build now finds the pinned JDK itself and says which); the emulator launcher took the first AVD listed, a user AVD whose system image was not installed, and waited 3 minutes with the emulator's output discarded (it now prefers the `rayx-` AVD, reports an emulator that exits, and tries the next AVD). The emulator also starts with `-gpu host` first: macOS's `auto` gives only software Vulkan, which the RayX renderer rejects (the app starts and never opens a window). Examples Mobile then installs and renders on the Android 34 arm64 emulator. Follow-up: the setup probes were loose (any AVD satisfied the default-AVD check, any NDK satisfied the NDK pin), so `doctor` called a Mac with only a broken `Pixel_9_Pro_XL` and NDK 27.2 complete. Both now require the pinned item; after installing `ndk;28.0.12674087` the build reports and uses NDK 28.0 although 27.2 is still on disk |

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

## Not verifiable before a release

The first tagged release (cargo-dist builds of all six targets, the installers) and a real `rayx
self update` download need a published release; `self update --check` reports that none exists.
