---
agentx:
  kind: rules
  scope: knowledge/nsh
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T06:00:00+02:00
  topics:
    - nsh
  supersedes: []
  superseded_by: []
---

# nsh Rules

- Never run the `nsh` on `PATH`; run the pin's build from the store, in the directory the pin was read in, without `CARGO` in the environment.
- Read a pin through the caller's runner: only an executing runner writes a missing `Cargo.lock`, and an unreadable pin is an error, never the default.
- NSH's packages are `pin::is_nsh_crate`, not every `nsh-*` name: `nsh init` names an app's crate `nsh-app-<slug>`.
- Build in an NSH checkout through `install::in_nsh_checkout` (no `CARGO`, no `RUSTUP_TOOLCHAIN`), with the `rustup`/`cargo` `setup::rust` finds and `--target-dir target`.
- An entry is installed only with `receipt.json`; remove the receipt before redoing a broken entry, and hold `<id>.lock` while installing or pruning.
