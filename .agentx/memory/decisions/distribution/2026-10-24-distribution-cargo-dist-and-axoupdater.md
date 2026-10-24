---
agentx:
  kind: decision
  scope: decisions/distribution
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  promoted_from: rayx-1-0
  topics:
    - distribution
    - releases
    - self-update
  supersedes: []
  superseded_by: []
---

## Decision: Release with cargo-dist and update with axoupdater

### Context: Carried over from RayX.CLI.Old (`.agentx/memory/decisions/distribution/2026-10-08-distribution-cargo-dist-and-axoupdater.md`); epic `rayx-1-0` keeps the crate, binary and install path (design `d-release`).

### Decision text
cargo-dist owns `.github/workflows/release.yml`, the six targets, checksums, attestations and the shell and PowerShell installers into `$RAYX_HOME/bin` or `~/.rayx/bin`. `rayx self update` uses `axoupdater` with the install receipt for app `rayx-cli` from `rayx-ui/RayX.CLI`; without a receipt it says how to update.

### Invariants
- `release.yml` changes only through `dist generate` with the pinned dist version.

### Supersedes: None in this repository.
