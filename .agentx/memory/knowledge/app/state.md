---
agentx:
  kind: state
  scope: knowledge/app
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T06:00:00+02:00
  topics:
    - app
  supersedes: []
  superseded_by: []
---

# App State

## Purpose And Usage

Read before changing `rayx app`, `rayx new` or `rayx serve`.

## Current Technical Shape

- `rayx app [-C <dir>] [--no-install] <command> [args]` (`src/app/mod.rs`): reads the pin where nsh will run through the command's runner, runs the prerequisite step, and launches the pin's CLI with every argument unchanged (`--browser <the test set's Chromium>` inserted before `--` for `test --target web` when no browser is installed), without `CARGO`, with `CC_`/`AR_wasm32_unknown_unknown` and `ANDROID_NDK_HOME` only when unset. Unix `exec`s; Windows registers a Ctrl+C handler for rayx alone, waits and exits with nsh's code.
- Prerequisites (`src/app/prereqs.rs`): design §6.1's map from command and `--target` to sets; silent items installed with a line, owner items asked about in a terminal or refused with the `rayx setup` command, `--no-install` reports; on a host `rayx setup` refuses only the nsh step is ensured (`rayx nsh install -C <dir>`).
- Fork development: `RAYX_NSH` (absolute against where rayx started, warning on every command, also printed by `rayx nsh path`); a path pin builds the checkout's CLI (`--target-dir target`) and writes nothing to the store.
- `rayx new` (`src/new.rs`): the default pin's `nsh init`, then `.cargo/config.toml` patches and `cargo generate-lockfile` (`mixed`, `rust`) or `rayx.toml` (`ts`); refuses when that file exists. `rayx serve` (`src/serve.rs`): a blocking HTTP/1.1 static host on 127.0.0.1.
- Proven on this Mac: an app made by `rayx new` builds, runs on the web, passes its tests on the web, macOS and the iOS simulator, packages and is served; Gamify shows its pin, checks, packages and is served without a tracked change.

## Backlinks

- [rayx app](../../../../src/app/mod.rs), [prerequisites](../../../../src/app/prereqs.rs), [rayx new](../../../../src/new.rs), [rayx serve](../../../../src/serve.rs)
- [Run tests](../../../../tests/nsh_run.rs), [prerequisite tests](../../../../tests/app_prereqs.rs), [fork development](../../../../tests/nsh_fork_dev.rs), [new](../../../../tests/new_app.rs), [serve](../../../../tests/serve.rs)
- Real proofs: [web](../../../../tests/proof_app_web.rs), [Apple](../../../../tests/proof_app_apple.rs), [Gamify](../../../../tests/proof_gamify.rs)
- [Design §4.4, §4.6, §4.7, §5, §6.1](../../../../docs/architecture/rayx-nsh-proposal.md)
