---
agentx:
  kind: knowledge
  scope: knowledge/nsh
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - nsh
  supersedes: []
  superseded_by: []
---

# nsh Knowledge

- `nsh --version` prints the commit (`nsh 0.2.0 (327bb9e)`); every `fork-0.2.0-*` tag is version 0.2.0, so the store is keyed by commit.
- nsh finds its runtimes through `NSH_RUNTIME`, a distribution (`bin/../runtime/`) or a checkout, never beside a `cargo install`ed binary.
- nsh runs `cargo` without a working directory of its own and uses `$CARGO` when set.
