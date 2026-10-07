---
agentx:
  kind: rules
  scope: knowledge/repository
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
  topics:
    - repository
    - governance
    - agentx-memory
    - sibling-repositories
  supersedes: []
  superseded_by: []
---
# Repository Rules

- Take dependencies from crates.io only; no `path` or `git` dependency on RayX, Gpux or any private repository, and no `[patch]`.
- Use Conventional Commits with meaningful scopes and no Claude co-author lines. Code moved from RayX names the RayX commit it came from in the commit message (fresh history, no `git filter-repo`).
- A change that RayX's CI or docs rely on (command grammar, flags, artifact layout) is paired with the matching RayX commit.
