---
agentx:
  kind: knowledge
  scope: knowledge/project
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - project
  supersedes: []
  superseded_by: []
---

# Project Knowledge

- Under `cargo test` the harness forces `RUSTUP_TOOLCHAIN` on children, so fallback tests script cargo's failure.
- `android-system-image` defaults to `<platform>;google_apis` of the resolved platform pin and reports its source.
