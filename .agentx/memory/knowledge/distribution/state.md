---
agentx:
  kind: state
  scope: knowledge/distribution
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:59:00+02:00
  topics:
    - ci
    - releases
    - cargo-dist
    - self-update
    - installers
  supersedes: []
  superseded_by: []
---
# Distribution State

## Purpose And Usage

Read before changing CI, release targets, the installers, or how `rayx` updates itself.

## Current Technical Shape

Planned by the epic `rayx-cli-foundation` (design decision `d-release`). `.github/workflows/ci.yml` runs fmt, Clippy and tests on Windows, macOS and Ubuntu (toolchain from `rust-toolchain.toml` through `actions-rust-lang/setup-rust-toolchain`); the release pieces are not written yet: cargo-dist (`dist-workspace.toml`, generated `.github/workflows/release.yml`) builds `x86_64`/`aarch64` Windows MSVC, Linux musl and macOS; `src/update.rs` uses `axoupdater` with the dist install receipt. The repository is public as `rayx-ui/RayX.CLI`, so downloads need no authentication.

## Backlinks

- [CI workflow](../../../../.github/workflows/ci.yml)
- [Epic design (`d-release`)](../../../tasks/rayx-cli-foundation/design.json)
- [Epic requirements (REL)](../../../tasks/rayx-cli-foundation/requirements.json)
