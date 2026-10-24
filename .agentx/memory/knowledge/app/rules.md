---
agentx:
  kind: rules
  scope: knowledge/app
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T06:00:00+02:00
  topics:
    - app
  supersedes: []
  superseded_by: []
---

# App Rules

- Pass nsh's arguments through unchanged (D12); read only the command name, `--target` and `--browser`.
- `RAYX_NSH` is a path, made absolute against the directory rayx started in; never search it on `PATH`.
- `rayx new` never replaces the file it pins an app in.
- On Windows, keep Ctrl+C from rayx with a handler function, never the inherited null form, so nsh still gets it.
