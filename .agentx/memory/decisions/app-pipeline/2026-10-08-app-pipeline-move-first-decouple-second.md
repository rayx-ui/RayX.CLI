---
agentx:
  kind: decision
  scope: decisions/app-pipeline
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - app
    - move
    - project-context
  supersedes: []
  superseded_by: []
---

## Decision: Move the xtask app pipeline first, then decouple it from RayX

### Context: Epic `rayx-cli-foundation` (design decision `d-app-move`), 2026-10-07 to 2026-10-08.

### Decision text

The xtask app modules move into `src/app/` first as they are (fresh history, provenance in the commit message), then their couplings are replaced:

- `fs_util::repo_root()` and every caller take an explicit `ProjectContext { workspace_root, metadata, pins, host }` built by `project::Project::discover`;
- `workspace_paths::gpux_root()` becomes a `cargo metadata` lookup of the `gpux-fonts` package; `root_patch_path` reads the selected workspace's root manifest;
- generated entry manifests take the `rayx`/`rayx_devkit` dependency from the app's own resolved dependency (`cargo metadata` `dependencies[].source`/`path`/`req`) and the pinned `wasm-bindgen`;
- `+nightly` becomes `+<web-toolchain>`;
- desktop triples come from `HostFacts`; every path (`target`, `artifacts`, `artifacts-temp`) is joined to the workspace root;
- the WASM test path resolves the Playwright package, specs and projects as APP-6 describes, creating the app-local package from the embedded template when none exists, and the `RayXTestSuite` slug check goes; `test` on native targets runs the app package's `cargo test`;
- before building, the app command asks `setup` for the target's sets in probe mode and installs missing unprivileged items automatically (APP-7), prompting or stopping for privileged ones;
- the release re-exec in `main.rs` is dropped.

The existing `[package.metadata.rayx]` keys and the `rayx.assets.toml` schema are kept unchanged and become the documented app-project contract in `docs/app-project.md`.

### Reasoning: Moving first and decoupling second keeps the diff reviewable and lets the moved tests prove nothing broke before behavior changes. The generated entry crates' fixed `crates/rayx` path dependency is the main blocker for other projects; copying the app's own resolved dependency is the only spelling that is correct for path, git and registry sources alike.

### Invariants

- Artifact locations and generated-file layouts stay byte-compatible with xtask for RayX apps (APP-4).
- Android ABI collection, size-optimized release mode, Gradle invocation and generated mobile entry crates behave as the RayX decisions below describe; this move changes where the code lives and how it finds the project, not those behaviors.
- No app name, slug or RayX repository path is special anywhere in the pipeline.

### Supersedes: None.
