---
agentx:
  kind: rules
  scope: knowledge/repository
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-14T17:00:00+02:00
  topics:
    - repository
    - governance
    - agentx-memory
    - sibling-repositories
  supersedes: []
  superseded_by: []
---
# Repository Rules

- Take dependencies anyone who clones the public repository can fetch: crates.io, public `https://` git repositories pinned to a commit `rev` (Gpux is public), and `[patch]` only with a comment directly above it linking the upstream issue or PR (remove it once upstream releases). No `path` dependency on a sibling checkout, no git `branch`/`tag`, no alternative registry or `[replace]`, no dependency on RayX or another private repository (`tests/dependency_sources.rs`).
- Use Conventional Commits with meaningful scopes and no Claude co-author lines. Code moved from RayX names the RayX commit it came from in the commit message (fresh history, no `git filter-repo`).
- A change that RayX's CI or docs rely on (command grammar, flags, artifact layout) is paired with the matching RayX commit.
