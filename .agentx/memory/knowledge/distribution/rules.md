---
agentx:
  kind: rules
  scope: knowledge/distribution
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - distribution
  supersedes: []
  superseded_by: []
---

# Distribution Rules

- Regenerate `release.yml` with `dist generate`; never hand-edit it.
- Stay on 1.x: never `!` or `BREAKING CHANGE:`; a break is `feat`/`fix` with a `Migration:` footer.
- `RELEASE_PLZ_TOKEN` is the only release secret and lives in GitHub Actions only.
