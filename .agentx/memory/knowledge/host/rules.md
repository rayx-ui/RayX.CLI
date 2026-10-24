---
agentx:
  kind: rules
  scope: knowledge/host
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - host
  supersedes: []
  superseded_by: []
---

# Host Rules

- Detect the native CPU architecture, never the process's: an x64 rayx emulated on Windows ARM64 installs ARM64 tools.
- Run every external program through `host::Runner`; `Print` mode has no side effects, PATH edits included.
- Never read, store or forward a password or token, and never run rayx itself elevated.
- Keep PATH edits idempotent: one marked block per shell profile, `REG_EXPAND_SZ` preserved on Windows.
