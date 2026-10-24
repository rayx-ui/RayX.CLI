# Projects

What `rayx` reads from a project, and where. A project is a Cargo workspace, an NSH app (a
directory with `nsh.json`), or both. `rayx` writes into it only what a command is for: `rayx new`
creates the app, `rayx fmt` formats it, `rayx setup --test` installs the Playwright package's
dependencies, and a command that installs or runs nsh writes the `Cargo.lock` a Cargo workspace
lacks (as `cargo metadata` does; `--check`, `doctor` and `rayx nsh status` write nothing).

## Finding the project

`rayx` starts in the current directory, or the one `-C <dir>` names (`rayx app`, `rayx nsh`):

1. **The Cargo workspace:** `cargo locate-project --workspace`, with `RUSTUP_AUTO_INSTALL=0` so
   finding the project never installs a toolchain. On a machine where Cargo does not run yet, the
   manifests are read directly.
2. **An app without a workspace there:** the crates the app's `nsh.json` lists under `"rust"`, each
   in its own workspace.
3. **Neither:** no project. `rayx setup` then installs only what the machine needs, with the
   built-in defaults.

## The nsh pin

The NSH an app is built with, read in the directory nsh runs in:

| Where | What | Example |
| --- | --- | --- |
| `Cargo.lock` | the one `source` of every NSH package (NSH's own crates, not an app's `nsh-app-*`) | `git+https://github.com/dXnSix/nsh?tag=fork-0.2.0-4#327bb9ea65d08ffac4af498cc7fbbfa18a1b3623` |
| `rayx.toml` beside `nsh.json` | the `[nsh]` table, for an app with no NSH crates | see below |
| built in | the nsh this release of `rayx` was tested with | `fork-0.2.0-4` at `327bb9e` |

A `Cargo.lock` source is one of:

| Source | Installed from | Store id |
| --- | --- | --- |
| a git URL with `?tag=` and the commit | the tag's release archive for this host, else a checkout of the commit | the commit's first 12 hex digits |
| a git URL with `?branch=` or `?rev=`, or no ref | a checkout of the commit | the commit's first 12 hex digits |
| crates.io (`registry+https://github.com/rust-lang/crates.io-index`) at 0.2.0 | upstream's `v0.2.0` release archive, else a checkout of `longbridge/nsh` at `v0.2.0` | `v0.2.0` |
| none (a path dependency on an NSH checkout) | nothing: `rayx` builds and runs that checkout's CLI | none |

Two different sources in one lock, or locks of the `"rust"` crates that disagree, stop with both
named. A crate patched to the fork in `.cargo/config.toml` (`[patch.crates-io]`) shows up in the lock
as the fork's git source, which is how `rayx new` pins `mixed` and `rust` apps.

### `rayx.toml`

An app of TSX only has no Cargo project to lock NSH, so `rayx new --template ts` writes its pin
next to `nsh.json`:

```toml
[nsh]
git = "https://github.com/dXnSix/nsh"
tag = "fork-0.2.0-4"
commit = "327bb9ea65d08ffac4af498cc7fbbfa18a1b3623"
```

`tag` may be `branch` or `rev` instead, and an upstream release is `version = "0.2.0"` alone. The
commit is checked against what the tag gives before anything is installed. An app that has NSH
crates and an `[nsh]` table is refused: its pin is `Cargo.lock`'s.

## The other pins

The other versions come from the project first, then from the built-in defaults; `rayx doctor`
shows each with its source.

| Pin | Project source | Default |
| --- | --- | --- |
| Rust toolchain | `rust-toolchain.toml` (or `rust-toolchain`) from the app's directory up to the workspace root | stable |
| wasm-bindgen | the `wasm-bindgen` package in `Cargo.lock` | none: `wasm-bindgen-cli` is installed only when the lock has it |
| Node.js | `node` | 22 |
| JDK | `jdk` | 21 |
| Android platform | `android-platform` | `android-35` |
| Android build tools | `android-build-tools` | `35.0.1` |
| Android NDK | `android-ndk` | 28.0.12674087 |
| Emulator system image | `android-system-image` | `<platform>;google_apis` |
| Xcode | `xcode` | the newest stable |

The keys are in the workspace's `[workspace.metadata.rayx]` table:

```toml
[workspace.metadata.rayx]
node = "24"
jdk = "21"
android-platform = "android-35"
android-build-tools = "35.0.1"
android-ndk = "28.0.12674087"
android-system-image = "android-35;google_apis"
xcode = "26.0"
tools-node = "tests/e2e"
```

`tools-node` is the directory of the project's Playwright package, relative to the workspace root
(default `tools-node`): `rayx setup --test` installs its packages with pnpm and the Chromium of
its Playwright (`pnpm exec playwright install chromium`). The guests' Ubuntu (24.04) is built in and has no key, since a guest serves every project
on the machine.

## What `rayx` leaves alone

nsh's caches (`NSH_CACHE`), Cargo's registry and git caches, and each app's `.nsh/` and `target/`
stay where nsh and Cargo put them. `rayx` keeps only its store (`$RAYX_HOME/nsh`, else
`~/.rayx/nsh`) and its records (`setup.json` and `guests.json`).
