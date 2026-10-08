---
agentx:
  kind: rules
  scope: knowledge/distribution
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-09T00:30:00+02:00
  topics:
    - ci
    - releases
    - cargo-dist
    - self-update
    - installers
  supersedes: []
  superseded_by: []
---
# Distribution Rules

- Regenerate the release workflow with `dist generate`; never hand-edit it.
- Leave a `rayx` not installed by the release installers (no dist receipt) in place and tell the user how to update it.
- CI (`ci.yml`) runs on pull requests and by hand (`workflow_dispatch`), never on pushes: a push trigger runs a PR branch twice and every merge again (owner, 2026-10-09). `release.yml` is dist-generated and runs on version tags, plus a `dist plan` check on pull requests.
