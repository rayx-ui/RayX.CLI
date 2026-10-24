---
agentx:
  kind: state
  scope: knowledge/nsh
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T06:00:00+02:00
  topics:
    - nsh
  supersedes: []
  superseded_by: []
---

# nsh State

## Purpose And Usage

Read before changing how rayx chooses, installs or runs nsh.

## Current Technical Shape

- Pin (`src/nsh/pin.rs`): the one source of the project's NSH packages in `Cargo.lock` (`is_nsh_crate`: the defaults table's crates and `nsh-platform-gpui`; an app's `nsh-app-*` crates are its own), found through `cargo locate-project` or, when cargo cannot run, the manifests; else the `[nsh]` table of `rayx.toml` beside `nsh.json` (tag checked with `git ls-remote`); else the default (`fork-0.2.0-4` at `327bb9e`, with a warning inside an app). A missing lock is written by `cargo metadata` only through an executing runner; dry runs report the pin as unknown. Refs are percent-decoded. `resolve_in` keeps an unreadable pin as `Unreadable { dir, reason }`.
- Store (`src/nsh/store.rs`): `$RAYX_HOME/nsh/<id>` (else `~/.rayx/nsh`), complete once `receipt.json` is written (temp file and rename), `<id>.lock` held while installing; prune keeps the default and every recorded project's pin, holds each candidate's lock, keeps lock files, and removes nothing while a recorded pin cannot be read.
- Install (`src/nsh/install.rs`): the release's `nsh-<platform>.tar.gz`/`.zip` and `.sha256` (fork tags from the pin's repository, crates.io pins from `longbridge/nsh` at `v<version>`), else a shallow checkout built with `cargo build --release --locked -p nsh-cli --target-dir target`; `nsh --version` must name the pin's version and commit. `RustTools` finds `rustup`/`cargo` as `setup::rust` does; checkout commands run without `CARGO` and `RUSTUP_TOOLCHAIN`. A dry run writes nothing; a broken entry loses its receipt before it is redone.
- Commands (`src/nsh/mod.rs`): `rayx nsh status|path|install|prune`; `path` prints `RAYX_NSH`'s binary when set. The base set's last step (`src/setup/nsh.rs`) installs the pin; an unreadable pin's step reads it again when it runs. Doctor shows one `nsh` line.
- The fork has no release of `fork-0.2.0-4` yet (owner-run OWN-1/OWN-2), so installs of the default build from source (about four minutes).

## Backlinks

- [Pin](../../../../src/nsh/pin.rs), [store](../../../../src/nsh/store.rs), [install](../../../../src/nsh/install.rs), [commands](../../../../src/nsh/mod.rs), [setup step](../../../../src/setup/nsh.rs)
- [Pin tests](../../../../tests/nsh_pin.rs), [store tests](../../../../tests/nsh_store.rs), [install tests](../../../../tests/nsh_install.rs), [command tests](../../../../tests/nsh_command.rs), [setup step tests](../../../../tests/setup_nsh.rs), [real installs](../../../../tests/proof_nsh_install.rs)
- [Design §4](../../../../docs/architecture/rayx-nsh-proposal.md), [Projects](../../../../docs/projects.md)
