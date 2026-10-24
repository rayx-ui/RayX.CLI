---
agentx:
  kind: knowledge
  scope: knowledge/nsh
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T06:00:00+02:00
  topics:
    - nsh
  supersedes: []
  superseded_by: []
---

# nsh Knowledge

- `nsh --version` prints the commit (`nsh 0.2.0 (327bb9e)`); every `fork-0.2.0-*` tag is version 0.2.0, so the store is keyed by commit.
- nsh finds its runtimes through `NSH_RUNTIME`, a distribution (`bin/../runtime/`) or a checkout, never beside a `cargo install`ed binary.
- nsh runs `cargo` without a working directory of its own and uses `$CARGO` when set.
- Cargo's version 4 lock percent-encodes a git ref (`feat/x` is `feat%2Fx`).
- `cargo test` (and anything started through rustup's proxy) sets `RUSTUP_TOOLCHAIN` for its children, which overrides a `rust-toolchain.toml`.
- nsh's on-demand web-runtime build in a checkout follows `CARGO_TARGET_DIR` but looks for its output beside its own CLI (`nsh: crates/nsh-cli/src/runtime.rs`); a candidate fork `fix/*` branch.
- A source build of the fork's `nsh-cli` takes about four minutes on an Apple silicon Mac; under heavy parallel load `quickjs-jit-sys`'s build script once lost a file it had just written (rerunning alone passed).
