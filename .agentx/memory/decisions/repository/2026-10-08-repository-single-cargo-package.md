---
agentx:
  kind: decision
  scope: decisions/repository
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - package
    - modules
  supersedes: []
  superseded_by: []
---

## Decision: Ship rayx as one Cargo package with a library and a thin binary

### Context: Epic `rayx-cli-foundation` (design decision `d-single-package`), 2026-10-07 to 2026-10-08.

### Decision text

One Cargo package `rayx-cli` at the repository root: library `rayx_cli` (`src/lib.rs`) and binary `rayx` (`src/main.rs`, which only calls `rayx_cli::run`). Modules:

- `cli` — top-level parsing;
- `host` — host facts, the command runner, PATH editing;
- `project` — workspace discovery and pins;
- `setup` — requirement sets, plans, installers; `setup::wsl`;
- `doctor` — the report;
- `update` — self update;
- `app` — the pipeline moved from RayX xtask (`app`, `args`, `assets`, `package_assets`, `desktop`, `wasm`, `android`, `ios`, `node_tools`, `process`, `fs_util`);
- `fmt`.

Integration test groups are `tests/<feature>.rs`; fixture projects are `tests/fixtures/<name>/`, each with its own empty `[workspace]` table so they never join the package.

### Reasoning: Nothing outside the CLI consumes its modules yet, so splitting into `rayx_host`/`rayx_setup`/`rayx_app` crates would add manifests and version coupling without a second user. RayX's own rule is that a crate must earn its boundary; the same rule applies here. The library target exists so integration tests can drive internals with fakes.

### Invariants

- Dependencies come from crates.io only; no `path`, `git` or `[patch]`.
- No module reads a compile-time path to decide what to work on.
- A new crate is added only when a second consumer needs a module (for example a future `rayx new` template crate or a shared library with gpux-testkit), not in advance.

### Supersedes: None.
