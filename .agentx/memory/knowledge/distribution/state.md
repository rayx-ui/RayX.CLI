---
agentx:
  kind: state
  scope: knowledge/distribution
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-14T23:30:00+02:00
  topics:
    - ci
    - releases
    - cargo-dist
    - self-update
    - installers
  supersedes: []
  superseded_by: []
---
# Distribution State

## Purpose And Usage

Read before changing CI, release targets, the installers, or how `rayx` updates itself.

## Current Technical Shape

Implemented by the epic `rayx-cli-foundation` (design decision `d-release`, sprints 1 and 5). `.github/workflows/ci.yml` runs fmt, Clippy and `cargo test` on Windows, macOS and Ubuntu (toolchain from `rust-toolchain.toml` through `actions-rust-lang/setup-rust-toolchain`) plus a `web` job on Ubuntu that provisions the web and test sets with `rayx setup --web --test --yes` and runs the two `#[ignore]`d groups (`app_wasm_fixture`, `app_playwright_template`) with `-- --include-ignored`; first green run 2026-10-08 on `feature/initial-setup`. cargo-dist 0.33.0 (`dist-workspace.toml`, generated `.github/workflows/release.yml`, `[package.metadata.dist] dist = true` because the package is `publish = false`) builds `x86_64`/`aarch64` Windows MSVC, Linux musl (`musl-tools`) and macOS archives with SHA-256 checksums, GitHub artifact attestations and the shell/PowerShell installers. The installers put `rayx` in `$RAYX_HOME/bin` or `~/.rayx/bin` (not rustup's `~/.cargo/bin`), with an `env` script next to it and the receipt `rayx-cli-receipt.json` that `axoupdater` reads. `aarch64-pc-windows-msvc` builds on a hosted `windows-2022` runner with host `x86_64-pc-windows-msvc` (`[dist.github-custom-runners]`), not the cargo-xwin container. `pr-run-mode = "upload"`: every pull request builds and uploads all six targets (proven on PR 1, run 2026-10-14: ARM64 PE `rayx.exe`, static musl binaries, checksums match). Versions come from release-plz (`release-plz.toml`, `.github/workflows/release-plz.yml`, the only workflow that runs on pushes to `main`): git-only (`git_only`, `publish = false`), tag `v{{ version }}`, no GitHub Release (dist creates it from the tag), `release_commits = "^(feat|fix|perf)"` so docs, CI and `chore(agentx)` commits start no release, and a git-cliff `[changelog]` with `## [X.Y.Z] - YYYY-MM-DD` headings that dist reads for the release notes. It authenticates with the fine-grained PAT secret `RELEASE_PLZ_TOKEN` so the release PR gets CI and the tag starts `release.yml`; `docs/releasing.md` has the flow, the token and the by-hand fallback. `Cargo.toml` is at 1.0.0 with a hand-written first `CHANGELOG.md` section; with no tag release-plz treats the package as an initial release and tags it on the first `main` push. GitHub lists `release-plz.yml` only once it is on `main`. `src/update.rs` uses `axoupdater`: `self update` runs only when an install receipt names the running executable, `--check` works without one, and `cargo install` or development builds are told how to update. `src/release.rs` (epic `rayx-cli-versions`) owns what a release is: `release::parse` (`X.Y.Z`, optional `v`, no prerelease), ordering, `installer_url`, `MIN_REFRESH_RELEASE` (1.1.0) and `exists` (a headers-only `curl -sfIL` of the installer through `Runner::query`; exit 22 is `Missing`). `rayx self update --release X.Y.Z [--yes] [--check]` installs exactly that release through axoupdater's specific-version request (upgrade or downgrade, receipt required, downgrade confirmed, nothing remembered) and offers the refresh only when the new release has it. The README install section uses the release installers (latest and pinned). Not yet verified: a tagged release and the real download. The repository is public as `rayx-ui/RayX.CLI`, so downloads need no authentication.

## Backlinks

- 2026-10-14 current [Decision: release-plz tags, dist builds](../../decisions/distribution/2026-10-14-distribution-release-plz-tags-dist-releases.md)
- [CI workflow](../../../../.github/workflows/ci.yml)
- [Epic design (`d-release`)](../../../tasks/rayx-cli-foundation/design.json)
- [Epic requirements (REL)](../../../tasks/rayx-cli-foundation/requirements.json)
