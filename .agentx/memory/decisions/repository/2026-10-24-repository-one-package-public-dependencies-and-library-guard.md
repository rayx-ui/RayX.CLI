---
agentx:
  kind: decision
  scope: decisions/repository
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  promoted_from: rayx-1-0
  topics:
    - repository
    - dependencies
    - guard
  supersedes: []
  superseded_by: []
---

## Decision: One package, public dependencies, and no library or app above NSH

### Context: Carried over from RayX.CLI.Old (`.agentx/memory/decisions/repository/2026-10-19-repository-library-agnostic-guard.md`) and narrowed by epic `rayx-1-0` for D14 (design `d-package`).

### Decision text
One Cargo package `rayx-cli` (library `rayx_cli`, thin binary `rayx`). Dependencies come from crates.io or public `https` git repositories pinned to a `rev` (`tests/dependency_sources.rs`). rayx names NSH, its toolchain, and nothing above it: no RayX, gpux, GPUI or NSH crate in `Cargo.toml` or `Cargo.lock`, no RayX app crate, gpux, GPUI, GameX or Gamify name and no `env!("CARGO_MANIFEST_DIR")` in non-test `src/`, checked by `tests/library_guard.rs` with an allowlist whose entries must still match.

### Reasoning: rayx runs the `nsh` CLI a project pins, so a release of rayx is never welded to one NSH commit or to any library built on it.

### Supersedes: None in this repository.
