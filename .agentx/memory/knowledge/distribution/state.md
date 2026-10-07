---
agentx:
  kind: state
  scope: knowledge/distribution
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
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

Planned by the epic `rayx-cli-foundation` (design decision `d-release`); no source exists yet. `.github/workflows/ci.yml` will run fmt, Clippy and tests on Windows, macOS and Ubuntu; cargo-dist (`dist-workspace.toml`, generated `.github/workflows/release.yml`) builds `x86_64`/`aarch64` Windows MSVC, Linux musl and macOS; `src/update.rs` uses `axoupdater` with the dist install receipt. The repository is public as `rayx-ui/RayX.CLI`, so downloads need no authentication.

## Backlinks

- [Epic design (`d-release`)](../../../tasks/rayx-cli-foundation/design.json)
- [Epic requirements (REL)](../../../tasks/rayx-cli-foundation/requirements.json)
