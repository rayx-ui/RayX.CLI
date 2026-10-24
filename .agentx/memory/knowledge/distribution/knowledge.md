---
agentx:
  kind: knowledge
  scope: knowledge/distribution
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - distribution
  supersedes: []
  superseded_by: []
---

# Distribution Knowledge

- A machine with the old rayx 1.4.1 sees 1.0.0 as older and does not move by `self update`; the 1.0.0 installer replaces the binary and keeps the records.
- cargo-dist 0.33.0 is not on crates.io; `dist generate --check` needs a dist built from its `v0.33.0` tag.
