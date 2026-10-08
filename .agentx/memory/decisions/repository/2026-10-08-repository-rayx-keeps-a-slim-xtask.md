---
agentx:
  kind: decision
  scope: decisions/repository
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - rayx-migration
    - xtask
  supersedes: []
  superseded_by: []
---

## Decision: RayX keeps a slim xtask for its chores and calls rayx for everything else

### Context: Epic `rayx-cli-foundation` (design decision `d-rayx-migration`), 2026-10-07 to 2026-10-08.

### Decision text

RayX keeps a slim root `xtask/` for repository chores only (`code-themes`, `component-index`, `theme`). Everything an app project needs runs through `rayx`. RayX CI installs a pinned `rayx`; docs, READMEs, agent instructions, the RayX skill and the two `tools-node` scripts call `rayx`. The RayX decisions superseded here are recorded at epic-end promotion.

### Reasoning: The chores operate on RayX's own source (`crates/rayx_theme`, the component sources, the code-theme converter crate) and depend on RayX-internal crates, so they cannot live in a public, project-agnostic CLI. New projects never need them.

### Supersedes: rayx:.agentx/memory/decisions/runtime/workspace/2026-06-28-runtime-workspace-root-xtask-orchestration.md
