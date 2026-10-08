---
agentx:
  kind: decision
  scope: decisions/distribution
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - release
    - self-update
  supersedes: []
  superseded_by: []
---

## Decision: Release with cargo-dist and update with axoupdater

### Context: Epic `rayx-cli-foundation` (design decision `d-release`), 2026-10-07 to 2026-10-08.

### Decision text

cargo-dist (`dist init`) owns the release workflow (`.github/workflows/release.yml`), the six target triples, archives, checksums and the shell/PowerShell installers. `rayx self update` uses the `axoupdater` crate, which reads the install receipt the dist installers write; without a receipt (cargo install, development build) it prints how to update instead.

### Reasoning: dist and axoupdater are the maintained pair for exactly this (installers that edit PATH, receipts, safe in-place replacement on Windows), so the CLI writes no installer scripts or download code of its own.

### Invariants

- The generated release workflow is regenerated with `dist generate`, never hand-edited.

### Supersedes: None.
