---
agentx:
  kind: state
  scope: knowledge/distribution
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:59:00+02:00
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

Implemented by the epic `rayx-cli-foundation` (design decision `d-release`, sprints 1 and 5). `.github/workflows/ci.yml` runs fmt, Clippy and `cargo test` on Windows, macOS and Ubuntu (toolchain from `rust-toolchain.toml` through `actions-rust-lang/setup-rust-toolchain`) plus a `web` job on Ubuntu that provisions the web and test sets with `rayx setup --web --test --yes` and runs the two `#[ignore]`d groups (`app_wasm_fixture`, `app_playwright_template`) with `-- --include-ignored`; first green run 2026-10-08 on `feature/initial-setup`. cargo-dist 0.32.0 (`dist-workspace.toml`, generated `.github/workflows/release.yml`, `[package.metadata.dist] dist = true` because the package is `publish = false`) builds `x86_64`/`aarch64` Windows MSVC (the ARM64 one through cargo-xwin in a container), Linux musl (`musl-tools`) and macOS archives with SHA-256 checksums and the shell/PowerShell installers into `~/.cargo/bin`; `pr-run-mode = "plan"`, so the first tag is the first real build (cut it as a prerelease: `aws-lc-sys` through `axoupdater` -> `reqwest` is the cross-build risk). `src/update.rs` uses `axoupdater`: `self update` runs only when an install receipt names the running executable, `--check` works without one, and `cargo install` or development builds are told how to update. Until a release exists README and `docs/src/rayx.md` install from `cargo install --git ... --branch feature/initial-setup`. Not yet verified: a tagged release and the real download. The repository is public as `rayx-ui/RayX.CLI`, so downloads need no authentication.

## Backlinks

- [CI workflow](../../../../.github/workflows/ci.yml)
- [Epic design (`d-release`)](../../../tasks/rayx-cli-foundation/design.json)
- [Epic requirements (REL)](../../../tasks/rayx-cli-foundation/requirements.json)
