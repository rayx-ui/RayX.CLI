# Handoff: rayx-cli-foundation, the macOS and iOS verification

Where the `rayx-cli-foundation` epic stands and exactly what is left. Read this first, then
[docs/app-project.md](docs/app-project.md) (the app commands), [docs/manual-verification.md](docs/manual-verification.md)
(the checklist and the results so far) and the epic's AgentX specs in
[.agentx/tasks/rayx-cli-foundation/](.agentx/tasks/rayx-cli-foundation).

Last updated: 2026-10-08, branch `feature/initial-setup` (pushed to `origin`, public repository
`rayx-ui/RayX.CLI`).

**If you are the agent on the Mac, follow [Part 1](#part-1-the-mac-session) step by step.**

## Status

`rayx` is the public RayX command-line tool: `setup`, `doctor`, `wsl`, `self update`, `app` and
`fmt`. All six sprints of the epic are sealed and reviewed, the epic's 51 final checks passed, and
CI is green on Linux, macOS and Windows (including a job that runs `rayx setup` for real on fresh
runners of all three). What an automated check or a hosted runner cannot prove is listed here:

| Area | Proven where | Left |
|---|---|---|
| Base, web, test sets on Linux | CI (fresh Ubuntu 24.04), WSL Ubuntu 24.04 on the owner's PC | nothing |
| Base, web sets on macOS | CI (fresh `macos-latest`, `rayx setup --web --yes` then `--check`) | the test, Android, GPU and **iOS** sets, and anything needing a person |
| Base, web sets on Windows | CI (`windows-latest`), the owner's PC | nothing |
| `setup --wsl --clone`, `wsl status`, `wsl compact` | the owner's PC (see [docs/manual-verification.md](docs/manual-verification.md)) | nothing |
| App pipeline on fixtures | CI on all three OSes, the threaded-WASM fixture on Ubuntu | macOS desktop `pack`, iOS |
| App pipeline on real RayX | Windows: Lab and Test Suite WASM, Lab host, Examples Mobile Android, headed WebGPU spec | macOS desktop, WASM on Metal, **iOS** |
| `self update`, release archives | unit tests, `dist plan` | the first tagged release (owner's call) |

AgentX state (`python <agentx>/agentx_task.py status --task-dir .agentx/tasks/rayx-cli-foundation`):
all 38 criteria are implemented and verified. Nothing on the Mac is a criterion of the epic; this
session closes the *manual follow-ups* the owner wants evidence for. A failure found on the Mac
is a normal bug: fix it, add a test, record it, and do not reopen sprints.

## Part 1: the Mac session

### 1.0 What you need

- macOS on Apple silicon (an Intel Mac also works for everything except the simulator speed).
- An interactive login session (the headed WebGPU run opens a browser window).
- A terminal where you can answer `sudo`, the Command Line Tools dialog and Homebrew's prompts.
- Your Apple ID signed in to the App Store *or* Xcode already installed. `rayx` never reads or
  stores credentials: when `xcodes` or `mas` needs an Apple ID, `rayx setup --ios` stops and says
  what to do; install Xcode by hand and run it again.
- Git, and the RayX and Gpux checkouts beside this one (`../RayX`, `../Gpux`, `../RayX.Dtcg`,
  `../RayX.Reactive`, `../RayX.SignalR`, `../wgpu`), only for [1.5](#15-real-rayx-apps). RayX's
  `RayX.Dtcg` sibling must be on a revision whose token runtime accepts RayX's tokens (`main` at
  the time of writing; see [1.5](#15-real-rayx-apps)).
- Python 3 and the AgentX skill (`~/.claude/skills/agentx`), to read the epic state. You do not
  need to advance anything.

### 1.1 Get the branch and build

```bash
git fetch origin
git switch feature/initial-setup
git pull --ff-only
cargo build --release          # rust-toolchain.toml installs 1.95.0 on the first cargo command
./target/release/rayx --version
```

If `cargo` is missing, that is the first finding: `rayx` cannot install Rust before it exists. Use
`curl https://sh.rustup.rs | sh` and say so in the report.

### 1.2 The whole test suite on macOS

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast 2>&1 | tee artifacts-temp/mac/cargo-test.log
```

`the_fixture_app_fails` printing a panic is expected (a test deliberately fails an app's own
test). Everything else must pass; CI's `macos-latest` job is green, so a failure here means a
difference between this Mac and the runner. The two heavy groups need the web and test sets; run
them after 1.4:

```bash
cargo test --test app_wasm_fixture --test app_playwright_template -- --include-ignored
```

### 1.3 `doctor`, and the setup sets one at a time

Run each command and keep the output in `artifacts-temp/mac/` (git-ignored). Read the output
critically: `rayx doctor` must be **truthful** about this Mac, and every `missing` line must come
with a command that fixes it.

```bash
mkdir -p artifacts-temp/mac
./target/release/rayx doctor            | tee artifacts-temp/mac/doctor-1.txt
./target/release/rayx doctor --json     > artifacts-temp/mac/doctor-1.json
./target/release/rayx setup --check --all | tee artifacts-temp/mac/setup-check-1.txt
```

Expected on a Mac that has never run `rayx`:

| Item | What `doctor` should say |
|---|---|
| host | `macos arm64` (or `x64`), not emulated unless the process is under Rosetta |
| `interactiveDesktop` | `true` in a logged-in session, `false` over SSH without a GUI |
| GPU | one Apple adapter (`system_profiler SPDisplaysDataType`), `hardwareAdapter: true`: macOS ships its drivers, the step installs nothing |
| base | Command Line Tools and Homebrew present or missing with the fix, Rust and the pinned toolchain |
| ios | Xcode, `xcode-select`, license, first launch, Simulator runtime, XcodeGen, `aarch64-apple-ios` and `aarch64-apple-ios-sim` |

Then install, in this order, answering prompts as they come, and check after each:

```bash
./target/release/rayx setup --yes                  # base: Command Line Tools, Homebrew, rustup
./target/release/rayx setup --check                # must exit 0
./target/release/rayx setup --web --yes            # nightly, wasm32, wasm-bindgen-cli, Homebrew LLVM
./target/release/rayx setup --test --yes           # Node, pnpm, Playwright package and Chromium
./target/release/rayx setup --ios --yes            # Xcode steps (needs your Apple ID, see 1.0)
./target/release/rayx setup --check --web --test --ios   # must exit 0, installs nothing
./target/release/rayx setup --web --test --ios --yes     # second run: "Nothing to do"
```

What to look for, because these were never run on a Mac:

| Step | Risk |
|---|---|
| Command Line Tools (`src/setup/macos.rs`) | uses the on-demand marker `/tmp/.com.apple.dt.CommandLineTools.installondemand.in-progress` and `softwareupdate`; falls back to `xcode-select --install` and waits. Check that it finds the right label and cleans the marker up. |
| Homebrew | official script with `NONINTERACTIVE=1` after `sudo -v` under `--yes`; `/opt/homebrew/bin` must reach the user PATH (a new shell may be needed: the output says so). |
| LLVM | Apple's clang has no wasm32 backend, so `--web` installs Homebrew's LLVM. |
| Node (`node@22`, keg-only) | `find_tool` also searches `/opt/homebrew/opt/node@22/bin`; pnpm through `corepack enable --install-directory ~/.local/bin`. |
| Xcode (`src/setup/ios.rs`) | versions from `Contents/version.plist`; `xcodes` installs the pinned or newest stable; `xcode-select --switch`; `xcodebuild -license accept` only with `--yes`; `-runFirstLaunch`; `xcodebuild -downloadPlatform iOS`; XcodeGen from Homebrew. Check each probe turns green after its install. |

### 1.4 The app pipeline on fixtures

The fixtures live in `tests/fixtures/`; copy one out so the repository stays clean:

```bash
rm -rf /tmp/rayx-mac && mkdir -p /tmp/rayx-mac
cp -R tests/fixtures/app-workspace /tmp/rayx-mac/ws
cp -R tests/fixtures/threaded_wasm_app /tmp/rayx-mac/wasm
cd /tmp/rayx-mac/ws
RAYX=~/path/to/RayX.CLI/target/release/rayx
$RAYX app apps/demo pack host                 # artifacts/apps/demo/host/release/
$RAYX app apps/demo test host                 # runs the package's own tests
cd /tmp/rayx-mac/wasm
$RAYX app . build wasm                        # threaded build with the pinned nightly
$RAYX app . test wasm                         # generates tests/e2e from the template, runs its smoke spec
```

`test wasm` runs headless Chromium on macOS (no extra flags); add `--headed --webgpu` to run on the
Mac's GPU through Metal, where the fixture itself does not use WebGPU, so `--webgpu` is expected to
fail the fixture's `BrowserWebGpu` check there; use it on a real RayX app (1.5).

### 1.5 Real RayX apps

Skip this if the RayX checkout is not here. From the RayX checkout:

```bash
cd ../RayX
git -C ../RayX.Dtcg status -sb            # RayX's tokens need the Dtcg revision RayX was built with
RAYX=../RayX.CLI/target/release/rayx
$RAYX app apps/lab build wasm
$RAYX app apps/test_suite build wasm --no-default-features --features rayx_diagnostics
$RAYX app apps/lab build host                 # macOS native, full release build (long)
$RAYX app apps/test_suite test wasm --headed --webgpu --no-default-features \
  --features rayx_diagnostics --playwright-test test_suite/rayx-collapsible-sizes.spec.ts
```

If every RayX web app stops at start-up (`WebPlatform::quit called` in the browser console, no
error), that is the `RayX.Dtcg` mismatch, not `rayx`: RayX's own tooling fails the same way. Use
`python ../RayX.CLI/scripts/prove-rayx-spec.py` from the RayX checkout; it builds a consistent
sibling set under `artifacts-temp/rxproof/` without touching any checkout.

### 1.6 iOS (F-1 and F-2 of this follow-up)

The Rust and Xcode pieces below were written from RayX's xtask and have **never been built on a
Mac** in this repository. Expect fixes.

```bash
cd ../RayX
$RAYX app apps/lab build ios --simulator       # generates the entry crate, runs XcodeGen, then xcodebuild
$RAYX app apps/lab run ios --simulator         # boots a Simulator, installs and launches the app
$RAYX app apps/lab pack ios --simulator        # stages artifacts/apps/RayXLab/ios/...
$RAYX app apps/examples_mobile build ios --simulator
```

| Piece | File | What may need fixing |
|---|---|---|
| command | `src/app/ios.rs` | picks a Simulator id with `xcrun simctl`, `xcodegen` in `platform/ios`, `xcodebuild -project … -scheme …` with `RAYX_IOS_RUST_MANIFEST`; checks the staged asset tree is inside the built `.app` (`verify_ios_app_bundle_assets`). |
| entry crate | `src/app/descriptor.rs` (`ios_rust_manifest`) | a staticlib crate under `artifacts-temp/apps/<slug>/ios/rust/` that includes the app's `src/main.rs`. |
| project | `apps/<app>/platform/ios/project.yml` (RayX) | the Xcode project XcodeGen generates; it builds the Rust staticlib through `RAYX_IOS_RUST_MANIFEST` and copies the staged `assets` folder as a resource. |
| prerequisites | `src/setup/ios.rs`, `src/app/prereqs.rs` | `rayx app … ios` checks the iOS set first; a missing Simulator runtime or XcodeGen should stop with `rayx setup --ios`. |

Record the exact error of every failure, fix it in this repository (or report it for RayX's
`project.yml` files, which need a RayX commit paired with the CLI one), add a unit test where the
failure can be reproduced without a Mac, and rerun.

### 1.7 Report back

Fill in the table in [docs/manual-verification.md](docs/manual-verification.md) (section "macOS"):
macOS and Xcode versions, the chip, and for each command the exit code and the one line that
matters. Commit it with any fixes:

```bash
git add -A
git commit -m "test(mac): record the macOS and iOS verification"   # Conventional Commits, no co-author lines
git push
```

CI on the push must stay green. If any source changed, rerun `cargo test` and the three CI jobs
before declaring done.

## Part 2: rules of this repository

- `CLAUDE.md` is the rulebook: crates.io dependencies only, every program through `host::Runner`
  (the moved xtask app pipeline under `src/app/` is a recorded exception), no password or token is
  ever read or stored, no elevation by `rayx` itself, licenses accepted only with `--yes`.
- Tests are one group per feature in `tests/<feature>.rs`; run the group you touch, not everything,
  until the final run.
- The epic is already closed in AgentX. Do not run `advance`, `check` or `review` unless you add
  specification work; plain fixes need commits only.
- Use Conventional Commits with scopes (`fix(setup): …`), and never add co-author lines.

## Part 3: where things are

| What | Where |
|---|---|
| macOS base steps | `src/setup/macos.rs` (Command Line Tools, Homebrew, `brew_program`) |
| iOS set | `src/setup/ios.rs` (Xcode, license, first launch, runtime, XcodeGen, targets) |
| `doctor` | `src/doctor.rs` (GPU adapter detection for macOS in `src/setup/gpu.rs`: `system_profiler SPDisplaysDataType -json`) |
| prerequisites of app commands | `src/app/prereqs.rs` |
| iOS build and run | `src/app/ios.rs` |
| tests with a fake Mac | `tests/setup_macos.rs`, `tests/setup_ios.rs`, `tests/doctor.rs` |
| decisions and rules | `.agentx/memory/` (start at `knowledge/INDEX.md`) |
