---
agentx:
  kind: rules
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
# Distribution Rules

- Regenerate the release workflow with `dist generate`; never hand-edit it.
- Leave a `rayx` not installed by the release installers (no dist receipt) in place and tell the user how to update it.
