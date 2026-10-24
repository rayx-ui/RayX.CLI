---
agentx:
  kind: state
  scope: knowledge/distribution
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T06:00:00+02:00
  topics:
    - distribution
  supersedes: []
  superseded_by: []
---

# Distribution State

## Purpose And Usage

Read before changing CI, release targets, the installers or how rayx updates itself.

## Current Technical Shape

`ci.yml` runs fmt, clippy and `cargo test` on `ubuntu-24.04`, `macos-latest` and `windows-latest` for pull requests, pushes to `main` and by hand, in `rayx-ui/RayX.CLI` only. release-plz (`release-plz.toml`, `release-plz.yml`) and cargo-dist 0.33.0 (`dist-workspace.toml`, generated `release.yml`) are 1.4.1's. The package starts at 1.0.0 with a hand-written `CHANGELOG.md` section; `docs/releasing.md` describes the first release. `src/update.rs` (axoupdater) and `src/release.rs` (`OLDEST_RELEASE` and `MIN_REFRESH_RELEASE` 1.0.0, existence probes) implement `rayx self update [--check] [--release] [--yes]`.

The NSH fork's releases (design §10): `.github/workflows/fork-release.yml` on the fork's `fork/release` branch (worktree `../../dXn/nsh/nsh-wt/release`) publishes `nsh-<platform>` archives and digests for `fork-*` tags (linux-arm64 without the Android runtime, windows-arm64 optional); `private/fork-docs` documents it, and its `sync-main.sh` warns when an upstream range changes `release.yml`. Neither branch is pushed yet, and `fork-0.2.0-4` has no release (owner-run OWN-1, OWN-2).

## Backlinks

- [CI](../../../../.github/workflows/ci.yml), [release-plz](../../../../release-plz.toml), [dist](../../../../dist-workspace.toml), [releasing](../../../../docs/releasing.md)
- [Self update](../../../../src/update.rs), [releases](../../../../src/release.rs)
- [Release tests](../../../../tests/release_automation.rs), [policy](../../../../tests/release_policy.rs)
- 2026-10-24 current [Decision: release-plz tags, dist releases, CI on main](../../decisions/distribution/2026-10-24-distribution-release-plz-tags-dist-releases.md)
- 2026-10-24 current [Decision: cargo-dist and axoupdater](../../decisions/distribution/2026-10-24-distribution-cargo-dist-and-axoupdater.md)
