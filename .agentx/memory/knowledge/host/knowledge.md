---
agentx:
  kind: knowledge
  scope: knowledge/host
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - host
  supersedes: []
  superseded_by: []
---

# Host Knowledge

- The real UAC prompt cannot run in tests; the `fake_sudo` tests of `command_runner` run on Linux and macOS only.
- `Runner::query` runs for real in `Print` mode; a query cannot be privileged or interactive.
