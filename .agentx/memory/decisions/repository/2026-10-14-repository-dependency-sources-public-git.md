---
agentx:
  kind: decision
  scope: decisions/repository
  status: current
  created: 2026-10-14T17:00:00+02:00
  updated: 2026-10-14T17:00:00+02:00
  promoted_from: rayx-cli-releases
  topics:
    - package
    - modules
    - dependencies
  supersedes:
    - 2026-10-08-repository-single-cargo-package.md
  superseded_by: []
---

## Decision: Ship rayx as one Cargo package and allow dependencies anyone can fetch

### Context: Epic `rayx-cli-releases` (design decision `d-dependency-sources`), 2026-10-14. Supersedes the package decision of `rayx-cli-foundation`, which also said "crates.io only".

### Decision text

One Cargo package `rayx-cli` at the repository root stays: library `rayx_cli` (`src/lib.rs`) and binary `rayx` (`src/main.rs`, which only calls `rayx_cli::run`), with the modules `cli`, `host`, `project`, `setup` (with `setup::wsl`), `doctor`, `update`, `app`, `fmt` and `wsl`. Integration test groups are `tests/<feature>.rs`; fixture projects are `tests/fixtures/<name>/` with their own empty `[workspace]` table. A new crate is added only when a second consumer needs a module.

What changes is where dependencies may come from: anyone who clones the public repository can build it on any machine or CI runner.

- Allowed: crates.io; public git repositories over `https://` pinned to a commit `rev` (Gpux, `rayx-ui/Gpux`, is public under Apache-2.0); `[patch]` for a recorded reason, with a comment directly above it linking the upstream issue or pull request, removed once upstream releases the change.
- Forbidden: `path` dependencies on sibling checkouts, git dependencies on a moving `branch` or `tag`, alternative registries, `[replace]`, and any dependency on the private `rayx-ui/RayX` repository or another private repository.

`tests/dependency_sources.rs` enforces it with a rule function tested on sample manifests and applied to `Cargo.toml`, `cargo metadata` and `Cargo.lock`.

### Reasoning: The foundation epic kept the CLI registry-only while Gpux was private. Gpux is public now, and a pinned public git dependency or a short-lived upstream-linked `[patch]` is reproducible for every clone, which crates.io alone cannot offer for unreleased upstream fixes. Path dependencies and private repositories are the cases that break a clean clone.

### Invariants

- A clean clone builds with network access to crates.io and public GitHub only.
- No module reads a compile-time path to decide what to work on (`env!("CARGO_MANIFEST_DIR")` only in tests).

### Supersedes: [Ship rayx as one Cargo package with a library and a thin binary](2026-10-08-repository-single-cargo-package.md).
