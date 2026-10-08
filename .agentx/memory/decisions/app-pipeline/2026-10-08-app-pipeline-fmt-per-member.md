---
agentx:
  kind: decision
  scope: decisions/app-pipeline
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - fmt
  supersedes: []
  superseded_by: []
---

## Decision: Format the workspace one member at a time

### Context: Epic `rayx-cli-foundation` (design decision `d-fmt`), 2026-10-07 to 2026-10-08.

### Decision text

`fmt` moves from xtask unchanged in behavior (one `cargo fmt -p <member>` per workspace member from `cargo metadata --no-deps`), runs on the discovered workspace, and accepts `--check` in any position.

### Reasoning: See the epic design and the code it describes.

### Supersedes: None.
