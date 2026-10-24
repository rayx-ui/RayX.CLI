---
agentx:
  kind: rules
  scope: knowledge/repository
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - repository
  supersedes: []
  superseded_by: []
---

# Repository Rules

- Take dependencies anyone can fetch: crates.io, public `https` git pinned to a `rev`, `[patch]` only with an upstream link above it.
- Name NSH and nothing above it; depend on no NSH crate.
- Pair a change to `rayx app`, the pins, `rayx.toml` or the store layout with the consumers' change.
