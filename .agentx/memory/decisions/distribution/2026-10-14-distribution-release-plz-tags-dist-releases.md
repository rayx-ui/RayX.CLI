---
agentx:
  kind: decision
  scope: decisions/distribution
  status: current
  created: 2026-10-14T17:00:00+02:00
  updated: 2026-10-14T17:00:00+02:00
  promoted_from: rayx-cli-releases
  topics:
    - release
    - versioning
    - release-plz
  supersedes: []
  superseded_by: []
---

## Decision: release-plz turns Conventional Commits into version tags; dist builds the release

### Context: Epic `rayx-cli-releases` (design decisions `d-release-plz` and `d-dist-release-build`), 2026-10-14. Builds on [Release with cargo-dist and update with axoupdater](2026-10-08-distribution-cargo-dist-and-axoupdater.md); that decision stays current.

### Decision text

On every push to `main`, release-plz (`release-plz.toml`, `.github/workflows/release-plz.yml`) keeps a release pull request with the version bump and a `CHANGELOG.md` section from git-cliff, and pushes the tag `vX.Y.Z` when the version in `Cargo.toml` has no tag (the merged release pull request, or the initial `1.0.0`). It runs git-only (`git_only`, `publish = false`) with `git_release_enable = false`: cargo-dist, started by the tag, builds the six targets, checksums, attestations and installers and creates the GitHub Release with the changelog section as notes. Only `feat`, `fix` and `perf` commits start a release (`release_commits`); `chore(agentx)` and `chore(release)` are dropped from the changelog. Changelog headings are `## [X.Y.Z] - YYYY-MM-DD`, the form dist reads.

release-plz authenticates with the fine-grained PAT secret `RELEASE_PLZ_TOKEN` (resource owner `rayx-ui`, only `RayX.CLI`, Contents and Pull requests read/write), because events made with the default `GITHUB_TOKEN` start no workflows. dist installs into `~/.rayx/bin` (`$RAYX_HOME/bin` first) with attestations, builds `aarch64-pc-windows-msvc` on `windows-2022`, and `pr-run-mode = "upload"` proves every target on each pull request. The first release is `v1.0.0`; the by-hand fallback is `git tag vX.Y.Z`.

### Reasoning: Versions and notes should come from the commit history without a manual release ritual, and dist stays the single owner of the build and the GitHub Release.

### Invariants

- Tags have the form `v<semver>` and come from release-plz or the documented by-hand fallback, never from another tool.
- `RELEASE_PLZ_TOKEN` is the only release secret; it lives in GitHub Actions, never in `rayx` or the repository.
- `ci.yml` stays pull-request only; `release-plz.yml` is the one workflow that runs on pushes to `main`, and it runs no tests.
- `release.yml` changes only through `dist generate`; no `allow-dirty`.

### Supersedes: None.
