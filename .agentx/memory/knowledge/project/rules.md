---
agentx:
  kind: rules
  scope: knowledge/project
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - project
  supersedes: []
  superseded_by: []
---

# Project Rules

- Find the project from the paths the user gives, never from where rayx was compiled; no `env!("CARGO_MANIFEST_DIR")` outside tests.
- Resolve pins from the project, else `project::pins::DEFAULTS`, and report the source; no other module hard-codes a version.
- Run discovery and probe commands with `RUSTUP_AUTO_INSTALL=0`.
