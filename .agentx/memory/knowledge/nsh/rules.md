---
agentx:
  kind: rules
  scope: knowledge/nsh
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - nsh
  supersedes: []
  superseded_by: []
---

# nsh Rules

- Never run the `nsh` on `PATH`; run the pin's build from the store, in the directory the pin was read in, without `CARGO` in the environment.
