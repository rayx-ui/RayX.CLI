---
agentx:
  kind: decision
  scope: decisions/distribution
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  promoted_from: rayx-1-0
  topics:
    - distribution
    - releases
    - ci
    - release-plz
  supersedes: []
  superseded_by: []
---

## Decision: release-plz turns Conventional Commits into version tags; dist builds the release; CI runs on pull requests and main

### Context: Carried over from RayX.CLI.Old (`.agentx/memory/decisions/distribution/2026-10-14-distribution-release-plz-tags-dist-releases.md`) by epic `rayx-1-0` (design `d-release`, which supersedes it for this repository, and criterion REPO-3).

### Decision text
release-plz (git-only, `v{{ version }}` tags, `release_commits = "^(feat|fix|perf)"`, `RELEASE_PLZ_TOKEN`) keeps the release pull request and pushes the tag; dist builds and publishes the GitHub Release. The first release is 1.0.0: an initial release that release-plz tags on the first push to `main`, with no release pull request, using the hand-written `1.0.0` section of `CHANGELOG.md`. `ci.yml` runs fmt, clippy and `cargo test` on pull requests, on pushes to `main` and by hand, in `rayx-ui/RayX.CLI` only; `release-plz.yml` runs on pushes to `main` and runs no tests.

### Reasoning: Versions and notes come from the history without a release ritual. CI on `main` (new here: rayx 1.4.1 ran CI on pull requests only) proves `main` itself after each merge, and a push trigger limited to `main` still runs a pull request branch once.

### Invariants
- Tags are `v<semver>` from release-plz or the documented by-hand fallback.
- rayx stays on 1.x while the owner is its only consumer: never `!` or `BREAKING CHANGE:`.

### Supersedes: None in this repository.
