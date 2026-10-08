---
agentx:
  kind: decision
  scope: decisions/repository
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - pins
    - toolchain
  supersedes: []
  superseded_by: []
---

## Decision: Keep the nightly and JDK pins literal in each repository with one checklist

### Context: Epic `rayx-cli-foundation` (design decision `d-toolchain-pins`), 2026-10-07 to 2026-10-08.

### Decision text

The nightly and JDK pins are literal values in each repository that needs them — RayX.CLI's defaults table, RayX's root `[workspace.metadata.rayx]` (read by `rayx`) and Gradle builds, Gpux's testkit default and CI — and the RayX skill reference `wasm-toolchain-update.md` is the single checklist that moves them together. Gpux keeps its own pin rather than reading RayX's metadata, so gpux stays verifiable on its own.

### Reasoning: A shared pin file across three repositories would make Gpux depend on RayX or the CLI, against the 2026-10-07 rule that gpux must verify itself from its own repository. Literal pins plus one checklist and a grep keep each repository self-contained while preventing drift.

### Supersedes: None.
