# rayx on Native Shell (NSH): design proposal

Status: **accepted, revision 4** · 2026-10-24. Every decision in §0 is yours, made on 2026-10-23 and 2026-10-24:
revision 2 recorded a new CLI starting at 1.0.0 (D4) and `rayx new` without app templates (D5); revision 3 the fork's
releases (D6, D8, D9) and `rayx.toml` (D7); revision 4 accepts the remaining proposals (D10 to D15). Nothing is left
open; §15 is the input for the AgentX epic.

Inputs examined:

- `RayX.CLI.Old` @ `d006250` (rayx 1.4.1): the previous `rayx`, its setup engine, guests, releases and the `rayx app`
  pipeline for GPUI/gpux apps
- the NSH fork `dXnSix/nsh` @ `327bb9e` (`fork-0.2.0-4`): `crates/nsh-cli`, `crates/xtask/src/dist.rs`, the release
  workflows, `website/docs/installation.md` and the fork docs on `private/fork-docs`
- `Six.Gamify` @ `041d197`: the first NSH app on RayX, with its `xtask`, Aspire AppHost, Playwright support and CI
- `RayX` @ `f6b2603` (`v0.4.0`): the `rayx_app` crates

Paths name their repository first: `old:` (RayX.CLI.Old), `nsh:` (the fork), `gamify:` (Six.Gamify).

Purpose: agree on how the new `rayx` works before writing its AgentX epic. §15 is the proposed epic outline.

---

## 0. Decision log

| # | Decision | Status |
|---|---|---|
| D1 | rayx runs NSH apps through the `nsh` CLI and wraps it the way rustup wraps rustc: each project's pinned nsh, installed into rayx's own store, never the `nsh` on `PATH` | Decided 2026-10-23 |
| D2 | Users get everything through `rayx setup`; nobody types `cargo install`. App commands install a missing pinned nsh themselves, as the old rayx's prerequisite step did | Decided 2026-10-23 |
| D3 | NSH web stays single-threaded: `--web` installs no nightly and no `rust-src` | Decided 2026-10-24 |
| D4 | This is a new CLI: a new repository `rayx-ui/RayX.CLI`, first release **1.0.0**. It keeps the crate name, binary and install path | Decided 2026-10-24 (§11) |
| D5 | `rayx new` is part of 1.0.0, built so app templates can be added later; the templates themselves are not designed now | Decided 2026-10-24 (§4.7) |
| D6 | Every `fork-*` tag gets a GitHub release with the compiled NSH distribution for each platform, built by a workflow in the fork; rayx downloads it | Decided 2026-10-24 (§10) |
| D7 | An app without Rust crates keeps its pin in a `rayx.toml` beside `nsh.json`, which `rayx new` writes | Decided 2026-10-24 (§4.1, §4.7) |
| D8 | The fork's releases cover `macos-arm64`, `linux-x86_64`, `windows-x86_64` and `linux-arm64` (Lima guests on Apple silicon), and `windows-arm64` once NSH builds there | Decided 2026-10-24 (§10) |
| D9 | The release workflow is a fork-only file on a new kind of fork branch, `fork/*` (`fork/release`), for changes that never go upstream | Decided 2026-10-24 (§10) |
| D10 | The pin is the NSH source the project's `Cargo.lock` names, and the store is keyed by its commit | Decided 2026-10-24 (§4.1) |
| D11 | A store entry is the release's distribution for this host; where there is none, a source checkout at the commit with the CLI built inside it | Decided 2026-10-24 (§4.3) |
| D12 | `rayx app` passes nsh's own grammar through unchanged | Decided 2026-10-24 (§4.4, §5) |
| D13 | The old rayx's machine side ports nearly unchanged; the `rayx app` pipeline (about 12,000 lines) is dropped | Decided 2026-10-24 (§8) |
| D14 | rayx names NSH, its toolchain, and nothing above it: not `rayx_app`, not any app | Decided 2026-10-24 (§8) |
| D15 | No `--gpu` set: NSH draws with what the operating system ships | Decided 2026-10-24 (§6) |

Rejected in D1:

- **Building nsh into rayx as a library.** `nsh-cli` is a binary only (`nsh: crates/nsh-cli/Cargo.toml`; its `run` is
  private in `main.rs:506`), so the fork would need a library target. Each rayx release would then be welded to one
  NSH commit, and a project on any other commit would need another rayx.
- **Using the `nsh` on `PATH`**, as Gamify does today. Homebrew's upstream `nsh` shadows the fork's, and nothing
  checks the commit (§3).

## 1. Summary

```bash
rayx setup --web --ios      # once per machine and project: everything, including this project's nsh
rayx app run --target web   # the project's nsh, run in the project's directory
rayx doctor                 # every requirement with its fix, the pins and the nsh store
```

- rayx stays the one prebuilt binary that sets up a machine (setup, doctor, WSL and Lima guests, self update), and it
  now runs any NSH app.
- It no longer builds apps itself. Every app action is the project's nsh: `rayx app <nsh command> [nsh arguments]`.
  rayx adds the prerequisites the target needs, the right nsh, and the environment nsh needs.
- The nsh comes from the project. rayx reads the NSH crates' source in `Cargo.lock` (a fork tag and commit, or a
  crates.io release) and keeps one build of the CLI per commit in `~/.rayx/nsh/`.
- Every fork tag gets a release with the compiled NSH distribution for each platform (the CLI with its prebuilt hosts
  and runtimes), built by a workflow in the fork. rayx downloads the one for its host. Where there is none (a
  branch pin, an uncommon host), it builds the CLI in a checkout at the commit instead, and nsh then builds its hosts
  and runtimes there on demand.
- `rayx new` creates an app that is pinned from its first commit: `nsh init`, then the pin written into the app.
- Roughly half of the old rayx (the machine side) ports as it is; the other half (the GPUI app pipeline) goes away.

## 2. What changed since the old rayx

- **RayX is an NSH framework now.** `RayX` v0.4.0 is `rayx_app`, `rayx_app_di` and `rayx_app_macros` on NSH; GPUI and
  gpux live on as `RayX.Old`. The old `rayx app` pipeline (threaded wasm with wasm-bindgen, XcodeGen, cargo-ndk,
  Gradle, the asset catalog, the Playwright template) has nothing left to build.
- **nsh already builds, runs, packages and tests apps** for the web, macOS, iOS, Android, Linux and Windows. What it
  lacks is what the old rayx is good at: installing a machine's toolchains, a `doctor` (nsh has none), guests and self update.
- **The upstream CLI is not enough.** Gamify and RayX take NSH from the fork's `fork-*` tags (`--strict-port`,
  wasm-bindgen for app crates, SVG and multi-resolution images, shaders and more). Upstream's release (Homebrew,
  `install.sh`) is a different NSH, and the fork publishes no binaries.
- **On GitHub, `rayx-ui/RayX.CLI` is now `rayx-ui/RayX.CLI.Old`**, as `RayX` became `RayX.Old`. The name is free for
  this repository (§11).

## 3. NSH facts that shape the design

Verified in the fork at `fork-0.2.0-4`.

| Fact | Where | Consequence |
|---|---|---|
| The CLI only checks that the app's NSH crates have its version, and every `fork-0.2.0-*` tag is version 0.2.0 | `nsh: crates/nsh-cli/src/rust_app.rs:172-232`, `:2022-2027` | A CLI from another fork tag builds the app with different code and says nothing. The store is keyed by commit, not by version |
| `nsh --version` prints the commit (`nsh 0.2.0 (327bb9e)`), also when built in a shallow clone or by `cargo install --git`, as long as `git` was on `PATH` | `nsh: crates/nsh-cli/build.rs`, `main.rs:77` | rayx verifies a build by version and commit |
| The CLI finds its runtimes (hosts, `nsh.wasm` and `nsh.js`, Android's `classes.dex`) through `NSH_RUNTIME`, else a distribution (`bin/../runtime/`), else a checkout (`<root>/target/<profile>/nsh` beside `crates/nsh-ios-host/Cargo.toml`), where it builds them on demand | `nsh: crates/nsh-cli/src/runtime.rs:141-216`, `:285-330`, `:360-414`, `:456-461` | `cargo install` gives none of the three: apps without Rust crates cannot run on a platform, and Android fails for every app. rayx installs a checkout or a distribution, never a bare binary |
| A distribution is `nsh/bin/nsh` plus `nsh/runtime/{ios,macos,linux,windows,gpui,android,web}` and `nsh/embed/` | `nsh: crates/xtask/src/dist.rs:1-16` | The prebuilt form of a store entry (§10) |
| The CLI runs `cargo` without a working directory of its own, and uses `$CARGO` when it is set | `nsh: rust_app.rs:1628-1650`, `:2088-2090`; `runtime.rs:317` | Cargo config, such as Gamify's `[patch.crates-io]` to the fork tag in `src/frontend/.cargo/config.toml`, applies only when nsh runs in the project. A `CARGO` set by `cargo run` makes nsh bypass rustup's proxy, and on macOS every web runtime link then fails (`gamify: src/frontend/xtask/src/web.rs:112-118`) |
| The fork's `rust-toolchain.toml` is `stable` with the iOS, Catalyst and three Android targets; `oxc` 0.153 needs rustc 1.97 or later | `nsh: rust-toolchain.toml` | Building in a checkout installs that toolchain |
| Building the CLI needs a C toolchain (QuickJS, zstd, ring) and takes about four minutes on an M-class Mac; the binary is 40.7 MB | `nsh: crates/nsh-cli/Cargo.toml`, the workspace's `lto = "fat"` release profile | Only the fallback (§4.3) builds; the base set already installs a C toolchain |
| Upstream's release builds `nsh-<platform>.tar.gz` for macos-arm64, linux-x86_64 and windows-x86_64 with `cargo xtask dist`, only on `v*` tags, as a draft. The fork never creates `v*` tags, so it has no binaries | `nsh: .github/workflows/release.yml`; the fork docs' `README.md` | The fork gets a release workflow of its own for `fork-*` tags (§10) |
| `nsh check` downloads TypeScript 7.0.2 from npm into nsh's own cache (`NSH_CACHE`) | `nsh: crates/nsh-cli/src/check.rs:19`, `:130`, `:173-190` | The first `check` needs the network; rayx leaves nsh's caches alone |
| nsh has no `doctor` and no setup; its host requirements are in its installation docs | `nsh: website/docs/installation.md` | rayx's setup sets carry them (§6) |
| Web needs the `wasm32-unknown-unknown` target, a clang that targets wasm32 and `llvm-ar` (Apple's clang cannot), and `wasm-bindgen-cli` at the exact version an app's crates use. iOS needs Xcode, a simulator runtime and the iOS targets. Android needs `platforms;android-35`, build-tools (`aapt2`, `zipalign`, `apksigner`), an NDK, a JDK and the Android targets. Linux needs GTK 4.14+ and WebKitGTK 6.0. Windows needs Build Tools with the Windows SDK, LLVM (libclang) and WebView2 | `nsh: android.rs:15-51`, `bindgen.rs:31`, `runtime.rs:326-329`, `run.rs:757-769`, `cdp.rs:60-110`, `website/docs/installation.md:159-203` | §6 |

## 4. How rayx manages nsh

### 4.1 The pin

rayx reads the pin in the directory it runs nsh in, so the two never disagree about Cargo config.

1. **Find the project:** `cargo locate-project --workspace` from the current directory (or `-C <dir>`), as the old rayx does
   (`old: src/project/mod.rs:161-344`). In no Cargo workspace there, rayx looks at the crates the app's `nsh.json`
   lists under `"rust"`, each in its own workspace.
2. **Read the NSH source:** in the workspace's `Cargo.lock`, every package whose name starts with `nsh-` must have one
   and the same `source`. Without a lock, `cargo metadata` writes it first. Two sources stop with both named, as nsh
   itself does for mixed sources.
3. **Classify it:**

| Source in `Cargo.lock` | Pin | Store id | Installed from |
|---|---|---|---|
| `git+https://github.com/dXnSix/nsh?tag=fork-0.2.0-4#327bb9ea65d0…` | git URL, tag, full commit | `327bb9ea65d0` (12 hex digits) | the tag's release archive for this host (§10), else a checkout of the commit (§4.3) |
| `git+…?branch=…#<commit>` or `?rev=…#<commit>`, or any other git URL | git URL, ref, full commit | the commit's 12 hex digits | a checkout of the commit: a branch or a bare commit has no release |
| `registry+https://github.com/rust-lang/crates.io-index` at 0.2.0 | upstream release 0.2.0 | `v0.2.0` | upstream's `v0.2.0` release archive for this host, else a checkout of `longbridge/nsh` at `v0.2.0` |
| none (a path dependency) | an NSH checkout on this machine | none | nothing to install: rayx builds and runs that checkout's CLI (§4.6) |

4. **No NSH crates** (an app of TSX only, D7): the `[nsh]` table of `rayx.toml` beside the app's `nsh.json`, which
   `rayx new` writes (§4.7):

   ```toml
   [nsh]
   git = "https://github.com/dXnSix/nsh"
   tag = "fork-0.2.0-4"
   commit = "327bb9ea65d08ffac4af498cc7fbbfa18a1b3623"
   ```

   An upstream release is `version = "0.2.0"` instead. The commit is checked against what the tag gives. An app that
   has NSH crates and an `[nsh]` table stops with both named: its pin is `Cargo.lock`'s.
5. **Neither** (an app made by plain `nsh init --template ts`, or no app at all): rayx's built-in default, the NSH that
   release of rayx was tested with (today `fork-0.2.0-4` at `327bb9e`). In an app, rayx says so on every command and
   shows the three lines that pin it; outside any app, `rayx setup` installs the default.

Gamify needs no change for this: its `Cargo.lock` already names
`git+https://github.com/dXnSix/nsh?tag=fork-0.2.0-4#327bb9ea…` for every NSH package.

### 4.2 The store

```text
~/.rayx/                         $RAYX_HOME, as the installer already uses it
  bin/rayx                       the installer's, unchanged
  nsh/
    327bb9ea65d0/                one NSH commit (fork-0.2.0-4)
      nsh/bin/nsh                the release's distribution for this host
      nsh/runtime/…              its prebuilt hosts and runtimes
      receipt.json               written last
    327bb9ea65d0.lock            held while installing
    5f0c2a91be47/                a commit with no archive for this host (a branch pin)
      source/                    a shallow checkout at the commit
        target/release/nsh       the CLI; hosts and runtimes build into target/ on demand
      receipt.json
    v0.2.0/                      an upstream release
      nsh/bin/nsh, nsh/runtime/…
      receipt.json
```

- **A receipt makes an entry complete:** `{ "version": 1, "id", "kind": "distribution" | "source", "url", "ref",
  "commit", "nsh": "0.2.0", "cli": "nsh/bin/nsh", "installed" }`. An entry without one is unfinished: the next
  install downloads it again, or resumes a source build (the checkout and Cargo's target directory are reused).
- **Locking:** an install holds `<id>.lock`. A second rayx (two terminals, or Aspire and a test run) waits and says
  so, instead of building the same commit twice.
- **Ids** use the commit's first 12 hex digits to keep Windows paths short; the receipt has the full commit.
- **Size:** a distribution is about what its archive unpacks to (the archives are 150 to 250 MB); a source entry is
  a checkout plus a Cargo target directory that grows as nsh builds hosts in it, several GB once a few targets were
  built (to be measured in S3). `rayx nsh prune` lists the entries no recorded project (`setup.json`) and not the
  built-in default pins, with their sizes, and removes them after asking; `rayx setup --update` offers it.
- **Nothing else moves:** nsh's caches (`NSH_CACHE`), Cargo's registry and git caches, and each app's `.nsh/` and
  `target/` stay where nsh and Cargo put them.

### 4.3 Installing an entry

Everything runs as the user, so an install needs no question and no administrator rights.

**From a release (the usual case).** For a fork tag (§10) or an upstream release:

1. Download `nsh-<platform>.tar.gz` (`.zip` on Windows) and its `.sha256` from
   `https://github.com/<owner>/<repo>/releases/download/<tag>/`, with `<platform>` the host's (`macos-arm64`,
   `linux-x86_64`, `linux-arm64`, `windows-x86_64`, …).
2. Check the digest, and unpack the archive's `nsh/` into `<id>/nsh/`.
3. Check the CLI (step 4 below).
4. Write the receipt.

A tag whose release has no archive for this host (none yet, because its workflow is still running, or none at all
for an uncommon host such as macOS x64) installs from source, and rayx says which archive it missed.

**From source (the fallback).** For a branch or bare-commit pin, and where a release has no archive:

1. `git init` and `git fetch --depth 1 <url> <commit>` in `source/`, then check out the commit detached. GitHub
   serves a commit by its id; for a host that does not, rayx fetches the ref and checks out the commit.
2. `rustup toolchain install` in `source/`, for NSH's own `rust-toolchain.toml`.
3. `cargo build --release --locked -p nsh-cli` in `source/`, without `CARGO` in the environment.
4. The CLI's `--version`: the version must equal the NSH crates' version in the project's lock, and the commit must
   be the pinned one.
5. Write the receipt.

In a checkout, nsh builds the rest itself the first time a command needs it: `nsh-macos-host` for
`run --target macos`, `nsh-platform-web` for the web runtime, `cargo xtask android --release [--dev]` for Android
(`nsh: runtime.rs:154-216`, `:360-414`). That costs minutes once per entry and target. A distribution brings all of
them prebuilt.

### 4.4 Running nsh

`rayx app [-C <dir>] [--no-install] <nsh command> [nsh arguments] [-- app arguments]`

1. **Pin and prerequisites:** resolve the pin (§4.1), then run the old rayx's prerequisite step (`old: src/app/prereqs.rs`)
   with the sets the command and its `--target` need (§6.1). The pinned nsh is one of its steps. It needs no
   administrator rights and no license answer, so a missing one is installed on the spot, with a line saying so.
   `--no-install` reports it instead, with the exact `rayx setup` command.
2. **Arguments:** everything after the command goes to nsh unchanged. rayx reads only the command name and
   `--target`, so nsh's own documentation applies, and a new nsh flag needs no rayx release.
3. **Working directory:** the current directory (or `-C`), the one the pin was read in.
4. **Environment:** rayx removes `CARGO`, and sets `CC_wasm32_unknown_unknown` and `AR_wasm32_unknown_unknown` to
   the LLVM the web set installed (Homebrew's on macOS) when they are unset. Everything else passes through.
5. **Process:** on Unix rayx `exec`s nsh, so a process manager (Aspire, a terminal, a test harness) that stops rayx
   stops nsh. On Windows it waits for nsh and exits with nsh's code.

Scripts that need the binary itself, such as Gamify's Playwright support, call `rayx nsh path`. It prints the CLI
of the current project's pin, installing it first when it is missing.

### 4.5 Setup and doctor

- `rayx setup` ends the base set with an `nsh` step, after the project's Rust toolchain. Its probe is the receipt
  plus `--version`; its install is §4.3. `--check` prints the commands it would run. The step asks nothing, so
  `--yes` does not change it.
- `rayx setup --update` already re-runs setup in every recorded project, so each project's current pin gets
  installed; then it offers `rayx nsh prune`.
- `rayx doctor` shows one line per pin, for example `nsh  fork-0.2.0-4 (327bb9e)  project  installed  3.1 GB` (the
  source is `project` or `default`, as for the other pins), and `--json` has the same fields.

### 4.6 Fork development

- **A path dependency on an NSH checkout** (§4.1) is how to work on the fork against an app: rayx runs
  `cargo build --release -p nsh-cli` in that checkout and runs its `target/release/nsh`, without copying anything
  into the store. Projects that must not depend on paths (Gamify, RayX) keep their own rule against it.
- **`RAYX_NSH=<path>`** runs that binary instead of resolving a pin, with a warning on every command:
  `rayx: RAYX_NSH=<path> replaces the project's nsh 327bb9ea65d0`. It is for trying a locally built CLI against a
  project that is pinned to a tag. It replaces Gamify's `NSH_CLI` and `NSH_E2E_CLI`.

### 4.7 New apps

`rayx new <dir> [--name <name>] [--template mixed|ts|rust]` creates an app that names its NSH from the start, so
every later `rayx` command in it finds the same nsh:

1. Install rayx's default pin if needed (§4.3), and run its `nsh init <dir> --name … --template …`. nsh's own three
   templates are the only ones in 1.0.0: `mixed` (TSX with a Rust crate at the app's root, the shape of a RayX app,
   and rayx's default), `ts` and `rust`.
2. For `mixed` and `rust`, write `.cargo/config.toml` in the app: a `[patch.crates-io]` entry for every NSH crate of
   the default pin (its `crates/nsh-*`), pointing at its git URL and tag, as Gamify's does. A store entry from a
   release has no checkout to list them from, so the defaults table carries the default pin's crate list with the pin.
   Then `cargo generate-lockfile`, so `Cargo.lock` holds the pin (§4.1).
3. For `ts`, which has no Cargo project, write `rayx.toml` with the default pin's `[nsh]` table (D7, §4.1).
4. Print the next steps: `rayx setup --web` (and `--ios` on a Mac), then `rayx app run`.

**Ready for templates, without any yet.** `--template` names a template. In 1.0.0 the names map to nsh's built-in
ones; later a name can map to a template rayx fetches by git tag, such as a RayX app with `rayx_app` and its
slices, run after step 2. What such a template contains is not designed here.

## 5. Command surface

| Command | New rayx | Compared with the old rayx |
|---|---|---|
| `rayx setup [--web] [--test] [--ios] [--android] [--all] [--check] [--yes] [--wsl\|--lima …] [--update] [--release …]` | Installs the sets and this project's nsh | The same flags without `--gpu` (D15); the sets' contents change (§6) |
| `rayx doctor [--json]` | Every requirement, the pins (now with `nsh`) and the guests | Adds the nsh store; no GPU adapters |
| `rayx new <dir> [--name …] [--template …]` | A new app, pinned (§4.7) | New |
| `rayx wsl status\|compact\|remove`, `rayx lima remove` | Unchanged | `compact --clean` also removes `.nsh/` build output |
| `rayx self update [--check] [--release …] [--yes]` | Unchanged | |
| `rayx fmt [--check]` | Unchanged | |
| `rayx app [-C <dir>] [--no-install] <command> …` | Runs the project's nsh: `init`, `build`, `check`, `dev`, `run`, `test`, `package`, `publish`, `keygen`, `certify`, `verify`, `revoke`, `symbolicate` | Replaces the old grammar (`rayx app <dir> build wasm`); targets are nsh's (`--target web`) |
| `rayx nsh status\|path\|install\|prune` | The pin and its store entry, the CLI's path, installing, removing entries | New |
| `rayx serve [--dir <site>] [--port <port>]` | A static host for what `nsh package --target web` writes: `.wasm` as `application/wasm`, `/health`, nothing outside the site | New; Gamify's `xtask web serve` |

The old grammar (`build|run|pack|publish|deploy|test|assets` for `host|windows|linux|macos|mobile|ios|android|wasm`)
is not kept as aliases. Its options (Android size knobs, the DevKit, `--webgpu`) mean nothing for NSH, and a
half-translation would hide nsh's own flags.

## 6. Setup sets for NSH

| Set | Old rayx | New rayx |
|---|---|---|
| base, every host | rustup, the project's toolchain, `~/.cargo/bin` on `PATH` | The same, then `nsh` (§4.5) |
| base, macOS | Command Line Tools, Homebrew | Unchanged |
| base, Linux | 24 apt packages, including gpux's X11, Wayland and Vulkan libraries | NSH's: `build-essential`, `pkg-config`, `git`, `curl`, `clang`, `llvm`, `libclang-dev`, `libgtk-4-dev`, `libwebkitgtk-6.0-dev` and GStreamer's plugins (the packages NSH's release workflow installs, plus what its docs list). GTK 4.14 and WebKitGTK 6.0 mean Ubuntu 24.04 or Debian 13 and later |
| base, Windows | winget, Build Tools, the VC++ redistributable, LLVM and its `PATH`, `fxc.exe` on `PATH`, rustup | Drops `fxc.exe` (gpux's shader compiler); adds a WebView2 probe (`Microsoft.EdgeWebView2Runtime`, already part of Windows 11) |
| `--web` | The nightly with `rust-src` (threaded wasm), wasm32 on the project toolchain, `wasm-bindgen-cli` at the lock's version, Homebrew's LLVM on macOS | No nightly (D3). wasm32, `wasm-bindgen-cli` when the lock has `wasm-bindgen` (Gamify: 0.2.129), Homebrew's LLVM on macOS (Linux and Windows have LLVM in base) |
| `--test` | Node, pnpm, the project's Playwright package and its Chromium; xvfb on Linux | Unchanged, and that Chromium also serves `nsh test --target web` (§6.1) |
| `--ios` | Xcode with its license and first launch, the Simulator runtime, XcodeGen, `aarch64-apple-ios[-sim]` | Drops XcodeGen (nsh generates its hosts); adds `aarch64-apple-ios-macabi` for Catalyst |
| `--android` | JDK, `JAVA_HOME`, Android Studio, the SDK with platform 34 and an NDK, emulator, AVD, acceleration, `cargo-ndk`, two Android targets | Platform 35 and build-tools 35.0.1 (nsh's `TARGET_SDK` and its own hint); the targets `aarch64-linux-android`, `armv7-linux-androideabi` and `x86_64-linux-android`; drops `cargo-ndk`, since nsh drives the NDK itself |
| `--gpu` | GPU drivers (Windows Update or Intel's assistant, `ubuntu-drivers` for NVIDIA), `vulkan-tools` | Dropped (D15). gpux drew everything through Vulkan and needed working vendor drivers. NSH draws with the system's own stack: Core Animation and Metal on Apple platforms, GTK 4 (with the distribution's Mesa) on Linux, Direct2D on Windows (`nsh: website/docs/linux.md:10`, `windows.md:10`). Only `--target gpui`, for embedding NSH in a GPUI app, needs a GPU, and the drivers the system ships suffice for it |

### 6.1 What each app command needs

| `rayx app` command and target | Sets beyond base |
|---|---|
| any command with `--target web` | web |
| `test --target web` | web and test. With no Chrome, Edge or Chromium installed, rayx passes nsh `--browser` with the test set's Chromium |
| `--target ios`, `ios-simulator` or `catalyst` | ios |
| `--target android` | android |
| `--target macos`, `linux`, `windows` or `gpui`, or no target on a desktop host | none |
| `build`, `check`, `init`, `keygen`, `certify`, `publish`, `verify`, `revoke`, `symbolicate` | none |

## 7. Pins

Every pin keeps the old order: the project first (`Cargo.lock`, `rust-toolchain.toml`, `[workspace.metadata.rayx]`),
then the built-in default (`old: src/project/pins.rs`).

| Pin | Project source | Default | Change |
|---|---|---|---|
| `nsh` | the NSH crates' source in `Cargo.lock` | `fork-0.2.0-4` at `327bb9e` | New |
| `rust_toolchain` | `rust-toolchain.toml` | stable | |
| `wasm_bindgen` | `Cargo.lock` | none | |
| `node` | `node` | 22 | |
| `jdk` | `jdk` | 21 | |
| `android_platform` | `android-platform` | `android-35` | Was `android-34` |
| `android_build_tools` | `android-build-tools` | `35.0.1` | Was none; nsh needs build-tools |
| `android_ndk` | `android-ndk` | `28.0.12674087` | To check against NSH's Android build |
| `android_system_image` | `android-system-image` | `<platform>;google_apis` | Follows the platform |
| `xcode` | `xcode` | the newest | |
| `ubuntu` | none | 24.04 | |
| `web_toolchain`, `playwright`, `android_ndk_api` | | | Removed: only the old pipeline used them |

`[workspace.metadata.rayx] tools-node`, the Playwright package's directory, stays; Gamify's is `tests/e2e`.

## 8. What ports from RayX.CLI.Old

`old: src/` has 27,238 lines; 12,183 of them are `src/app/`. Tests follow their modules (§12).

| Module | Lines | In the new rayx |
|---|---|---|
| `host/` (`mod`, `probe`, `runner`, `path_env`) | 1,940 | Port as is. Every program still runs through `host::Runner` |
| `setup/mod.rs`: the engine (sets, steps, plan, check, execute, records hook) | 1,047 | Port; add the `nsh` step |
| `setup/rust.rs`, `setup/macos.rs` | 459 | Port as is |
| `setup/linux.rs` | 93 | Adapt: NSH's apt packages (§6) |
| `setup/windows.rs` | 423 | Adapt: drop `fxc-path`, add WebView2 |
| `setup/web.rs` | 182 | Adapt: drop the nightly |
| `setup/test_tools.rs` | 540 | Port as is |
| `setup/ios.rs` | 385 | Adapt: drop XcodeGen, add the Catalyst target |
| `setup/android.rs` | 951 | Adapt: platform 35, build-tools, NSH's targets, no `cargo-ndk` |
| `setup/gpu.rs` | 594 | Drop (D15) |
| `setup/guest.rs`, `setup/wsl.rs`, `setup/lima/`, `machine.rs`, `migrate.rs`, `records.rs`, `refresh.rs`, `remove.rs`, `setup/update.rs` | 5,226 | Port as is, without the Windows-side GPU step of `--wsl`. The records keep their place and schema version 1, so `setup --update` also refreshes guests that the old rayx created (§11) |
| `doctor/` | 770 | Port; add the nsh lines, drop the GPU adapters |
| `project/mod.rs` | 476 | Port discovery; drop the per-package dependency model that fed the pipeline's entry crates |
| `project/pins.rs` | 205 | Adapt (§7) |
| `release.rs`, `update.rs` | 649 | Port as is |
| `wsl.rs` | 713 | Port; `.nsh` joins the build output `compact --clean` removes |
| `fmt.rs` | 75 | Port as is |
| `cli.rs`, `lib.rs` | 322 | Rewrite the `app` surface; add `nsh` and `serve` |
| `app/prereqs.rs` | 218 | Keep the logic; the map becomes §6.1 |
| the rest of `app/`: `descriptor`, `graph`, `desktop`, `wasm`, `android`, `ios`, `assets`, `package_assets`, `playwright` and its templates, `args`, `context`, `process`, `fs_util`, `test_support` | 11,965 | Drop |

New modules: `nsh/pin.rs` (§4.1), `nsh/store.rs` and `nsh/install.rs` (§4.2, §4.3), `nsh/run.rs` (§4.4), `new.rs`
(§4.7) and `serve.rs` (§5).

The old repository's rules stay: one package with a thin binary; dependencies anyone can fetch; no
`env!("CARGO_MANIFEST_DIR")` outside tests; pins only from the project or the defaults table; no passwords or tokens,
no self-elevation, licenses only with `--yes`; a test group per feature. The library-agnostic rule changes: rayx
names NSH, its toolchain, and still no library or app built on it (D14).

The old repository's AgentX decisions that still hold (one `Runner`, one setup plan for setup, `--check` and doctor, the guest
layer and its records, release-plz with dist) move into this repository's memory when the epic starts; the
app-pipeline ones stay behind.

## 9. What moves out of Gamify's xtask

| Gamify today | With the new rayx |
|---|---|
| The README's prerequisites: the fork CLI through `cargo install --git … --tag …`, Homebrew's LLVM, `wasm-bindgen-cli`, `~/.cargo/bin` first on `PATH` | `rayx setup --web --ios --test` |
| `cargo xtask nsh verify` (`gamify: src/frontend/xtask/src/nsh.rs`): one fork tag in the patches and the lock, the installed CLI from that tag and first on `PATH`, the tag pushed | Removed. The pin is the lock's commit and rayx runs exactly that build; Cargo keeps the patches and the lock in agreement, and a tag that is not pushed fails rayx's fetch |
| `cargo xtask deps verify` (`deps.rs`) | Stays: its rules (the dXn and RayX tags, no `rayx` or `gpux`, services free of `nsh-core`) are Gamify's. Its "one NSH source" rule is also rayx's pin rule |
| `web.rs`: `nsh_cli()` (`NSH_CLI`), `outside_cargo`, `wasm_toolchain` | Moves into rayx (§4.4). `NSH_CLI` goes |
| `cargo xtask web dev`: writes `config/runtime.json`, then `exec`s `nsh run . --target web --strict-port --port $PORT` | Writes the config, then `exec`s `rayx app run . --target web --strict-port --port $PORT`. The AppHost does not change |
| `cargo xtask web serve [--package]`: `nsh package` inside `with_config`, then its own static server | `--package` stays in xtask (it needs the config) and runs `rayx app package . --target web --out …`; serving becomes `rayx serve --dir … --port $PORT` |
| `xtask config`, `assets`, `tokens`, `quality` | Stay: Gamify's configuration, GameX's assets, its theme and its quality gates. `quality frontend` drops `nsh verify` and runs `rayx app check .` |
| Playwright's `forkCli` (a second CLI install under `artifacts-temp/nsh-cli/<tag>`), `NSH_E2E_CLI`, the tag in `support/app.ts` and four specs, `unlockNsh` | `rayx nsh path` |
| `quality.yml`: `NSH_FORK_TAG`, `brew install llvm`, `cargo install … nsh-cli`, `wasm-bindgen-cli`, Playwright's Chromium | Install rayx, then `rayx setup --web --test --yes` |

The fork tag is written in seven files besides `Cargo.lock` today (`.cargo/config.toml`, `quality.yml`,
`tests/e2e/support/app.ts` and four specs); afterwards only in `.cargo/config.toml`.

`xtask tokens`, the design-token compiler, would suit any NSH app with a `design/tokens.json`, but it mirrors NSH
0.2.0's built-in colours and Tailwind palette. It stays in Gamify until a second app needs it.

## 10. Fork releases (D6)

Every `fork-*` tag gets a GitHub release on `dXnSix/nsh` with NSH's distribution compiled for each platform: the same
archives upstream builds for its `v*` releases. rayx downloads them (§4.3), so nobody compiles NSH on their machine
unless their host has no archive.

**The workflow** is `.github/workflows/fork-release.yml` in the fork:

- **Runs** when a `fork-*` tag is pushed (`scripts/sync-main.sh tag` pushes it), and by hand with a tag, which builds
  an existing tag such as `fork-0.2.0-4`.
- **Checks** that the tag is `fork-<workspace version>-<n>` and that its commit is on `main`.
- **Builds** each platform with upstream's own steps (`nsh: .github/workflows/release.yml`, job `dist`): Java 17 and
  Android 35, stable Rust with wasm32, LLVM, GTK 4 on Linux, then `cargo xtask dist` and its smoke test
  (`--version`, `init`, `build --release`). A run takes about an hour.
- **Packs** `nsh-<platform>.tar.gz` (`.zip` on Windows), each holding one `nsh/` directory
  (`nsh: crates/xtask/src/dist.rs`), with a `.sha256` beside it, and attests them (`actions/attest`) as rayx's own
  releases do.
- **Publishes** the release once every platform has built: titled with the tag, published at once (a draft cannot be
  downloaded without signing in), with GitHub's generated notes. Nothing else: no installers, Homebrew, Scoop or
  crates.io, which stay upstream's.

**Platforms (D8):**

| Platform | Runner | Why |
|---|---|---|
| `macos-arm64` | `macos-15` | Upstream's: Apple silicon Macs; includes the iOS runtimes |
| `linux-x86_64` | `ubuntu-24.04` | Upstream's: Linux machines and WSL |
| `windows-x86_64` | `windows-latest` | Upstream's |
| `linux-arm64` | `ubuntu-24.04-arm` | New: the Lima guests on Apple silicon Macs, where Linux is tested |
| `windows-arm64` | `windows-11-arm` | New, once NSH's Windows build is proven there; until then those hosts build from source |

macOS x64 builds from source.

**Where the workflow lives (D9).** It never goes upstream, and the fork's branches today are `feat/*` and `fix/*`,
which are meant to. So there is a third kind, `fork/*`, for fork-only changes. `fork/release` holds the workflow and
joins `main` through a pull request like the others. As a new file it never conflicts with an upstream sync; the
price is that a change upstream makes to its `release.yml` (a new tool, a new runner) has to be copied over by hand,
so `sync-main.sh upstream` warns when a sync changes `release.yml`.

**The fork docs change with it:** the `private/fork-docs` README describes `fork/*`, and its tag section says that a
tag now produces a release, replacing "reinstall the CLI with `cargo install`" with `rayx setup`.

**The first release** is a run by hand for `fork-0.2.0-4`, so Gamify's current pin downloads before the next tag.

## 11. Distribution and the move from the old rayx

- **Repository:** a new, public `rayx-ui/RayX.CLI` (the old one is `rayx-ui/RayX.CLI.Old`), so its CI runs on GitHub
  Actions.
- **Version (D4):** a new CLI, first release **1.0.0**, with its own history and changelog. The old CLI's policy
  carries over: rayx stays on 1.x, and a break is a `feat` or `fix` commit with a `Migration:` footer, never `!` or
  `BREAKING CHANGE:`.
- **Unchanged from the old CLI:** crate `rayx-cli`, binary `rayx`, cargo-dist with the shell and PowerShell
  installers for the same six targets, `$RAYX_HOME/bin` or `~/.rayx/bin`, release-plz in git-only mode, GitHub
  artifact attestations.
- **Machines with the old rayx 1.4.1 do not move by themselves.** Its `rayx self update` asks GitHub for
  `rayx-ui/RayX.CLI` (`old: src/update.rs`), which will be this repository, and sees 1.0.0 as older than 1.4.1, so it
  changes nothing. The new rayx comes from its installer, which replaces `~/.rayx/bin/rayx` and the install receipt.
- **The machine's records carry over.** The new rayx keeps `~/.config/rayx/setup.json` and `guests.json` where the
  old one keeps them, in schema version 1, so `rayx setup --update` re-runs the recorded setups and refreshes the
  WSL and Lima guests the old rayx created, putting the new rayx into them.
- **A machine that still builds GPUI apps** (`RayX.Old`, `RayX.GameX`) keeps the old rayx by not installing the new
  one there: both use the same install path.
- **The old repository's links:** `RayX.CLI.Old`'s README installs from `rayx-ui/RayX.CLI` URLs, including pinned
  ones like `releases/download/v1.0.0/…`. Once this repository exists, those fetch the new rayx, so the old README
  moves to `RayX.CLI.Old` URLs before this repository is created.

## 12. Testing

The old layout stays: one file per feature group in `tests/` (`<area>_<feature>.rs`), selected with
`cargo test --test <group>`, using the `Runner::record()`, `FakeMachine` and `FixtureProbe` fakes and fixtures copied
into a temporary directory.

- **Ported groups:** `setup_*` (except `setup_gpu`), `guest_*`, `release_*`, `self_update*`, `wsl_*`, `readme_*`,
  `cli_surface`, `command_runner`, `doctor`, `fmt_command`, `host_facts`, `path_env`, `pin_sources`, `project_pins`,
  `dependency_sources`, `lima_named_vms` and `update_target_vm`. The `app_*` groups and `library_agnostic` go; a new
  guard replaces the latter (D14).
- **New groups:** `nsh_pin` (lock sources, mixed sources, no lock, `rayx.toml`, both at once, the default),
  `nsh_store` (receipts, unfinished entries, locking, prune), `nsh_install` (the archive for each host, the digest, a
  missing archive falling back to source, the source build's commands, the version and commit check), `nsh_run` (arguments, working directory, environment, `RAYX_NSH`, `exec`),
  `app_prereqs` (§6.1), `new_app` (§4.7: each template, the patches, the lock, an existing directory), `serve` and
  `setup_nsh` (the base-set step, `--check`, the doctor lines).
- **Fork release proof:** the workflow run by hand for `fork-0.2.0-4` publishes an archive per platform (§10), and
  each unpacks to a CLI that prints `nsh 0.2.0 (327bb9e)`.
- **Real proof** (ignored by default, run on demand): an install of `fork-0.2.0-4` from its release, and one from
  source, into a temporary `RAYX_HOME`, then `rayx app build` of a fixture app, `rayx app run --target web` until `/nsh.version` answers, and
  `rayx app test --target web`. On the Mac also macOS and the iOS simulator; Linux in a Lima guest; Windows and WSL by
  hand, logged in `docs/manual-verification.md` as the old rayx does.
- **Consumer proof:** Gamify on rayx (§9), with its Playwright specs and AppHost frontend tests passing on macOS.

## 13. Risks

| Risk | Mitigation |
|---|---|
| rayx relies on how nsh finds its runtimes (the distribution and checkout layouts) and on its `--version` format, which upstream may change | rayx uses only those three, each covered by the real-proof group; a change shows up when a project moves to a new fork tag |
| A host without an archive (macOS x64, Windows ARM64 for now, branch pins) compiles: minutes per pin and target, and several GB | rayx says so before it starts; `rayx setup` does it up front; prune |
| A new tag's release takes about an hour; a machine that needs the tag sooner compiles | rayx names the missing archive and builds from source; tags are rare |
| `fork-release.yml` drifts from upstream's `release.yml` (a new tool, a new runner) | `sync-main.sh upstream` warns when a sync changes `release.yml` (§10); the smoke test fails a broken archive before it is published |
| Paths under `~\.rayx\nsh\<id>\source\target\…` get long on Windows | 12-digit ids, `core.longPaths` in the checkout, proven on Windows in S6 |
| nsh run in another directory than the one the pin was read in would see other Cargo config | rayx reads the pin and runs nsh in the same directory; `-C` changes both |
| The prerequisite step probes several tools before every `rayx app` command | Measure in S4; if it is slow, skip the other probes while the pin's receipt exists and the target's sets were satisfied before |
| The new rayx replaces the old one's binary, so a machine that still builds GPUI apps loses `rayx app` when it installs the new one | The README says so; such a machine keeps the old rayx (§11) |
| The old README's install links point at `rayx-ui/RayX.CLI` and would install the new rayx | Move them to `RayX.CLI.Old` URLs before this repository is created (§11) |

## 14. Questions

All answered: Q1 (D4), Q2 (D6), Q3 (D7), Q4 (D15) and Q5 (D5). Nothing is open.

## 15. Proposed epic outline

The AgentX epic `rayx-1-0` (`.agentx/tasks/rayx-1-0/`) refines this outline into nine sprints: it splits the port in
two, moves the NSH set changes into the port, and adds a proof sprint and an owner-run sprint. Its plan is the
authority for the order of work. It changes this repository, the fork (`fork/release` and `private/fork-docs`) and,
for its install links, `RayX.CLI.Old`'s README. It proves rayx against Gamify without changing it; Gamify's own move
to rayx (S6 below) is a follow-up epic in Gamify.

The outline as first proposed:

| Sprint | Content | Proof |
|---|---|---|
| S1 The machine side | The repository; `host`, the setup engine and base sets, `doctor`, records, guests, `self update`, `fmt`, the release configuration; no `app/` | The ported groups; `rayx setup --check` and `rayx doctor` on macOS |
| S2 Fork releases | `fork/release` with `fork-release.yml` (§10), its pull request into the fork's `main`, the `sync-main.sh` warning and the fork docs; a run by hand for `fork-0.2.0-4` | The release `fork-0.2.0-4` with an archive and digest per platform, each printing `nsh 0.2.0 (327bb9e)` |
| S3 The pin and the store | §4.1 to §4.3 (releases first, source as the fallback), `rayx.toml`, `rayx nsh status\|path\|install\|prune`, the base-set `nsh` step, the doctor lines | `nsh_*` and `setup_nsh`; real installs of `fork-0.2.0-4` from its release and from source |
| S4 Running nsh | `rayx app` (§4.4), the prerequisite map (§6.1), `RAYX_NSH`, path checkouts, `rayx new` (§4.7), `rayx serve` | `nsh_run`, `app_prereqs`, `new_app`, `serve`; an app made by `rayx new`, built and run on the web through rayx |
| S5 Sets for NSH | §6: Linux, Windows, web, test, iOS, Android | `setup_*`; `rayx setup --all --check` on macOS and in a Lima guest |
| S6 Gamify on rayx | §9, in Gamify | Gamify's Playwright specs and AppHost frontend tests; web, macOS and the iOS simulator on the Mac; Linux in Lima; Windows and WSL as available |
| S7 Release 1.0.0 | README, `docs/releasing.md`, the old repository's links (§11) | 1.0.0 through release-plz and dist; the installer over an old 1.4.1 install keeps its records and guests |
| Later | App templates for `rayx new` (§4.7) | |
