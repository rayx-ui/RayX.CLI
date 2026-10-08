---
agentx:
  kind: decision
  scope: decisions/project
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - project
    - pins
  supersedes: []
  superseded_by: []
---

## Decision: Find the project through Cargo and let the project own its version pins

### Context: Epic `rayx-cli-foundation` (design decision `d-project-pins`), 2026-10-07 to 2026-10-08.

### Decision text

`project::Project::discover(start_dir)` runs `cargo locate-project --workspace --message-format plain` and `cargo metadata --format-version 1` from the selected app directory (or the current directory), resolving a relative app path against the current directory. `project::Pins::resolve(project)` applies the precedence in PIN-2: project (`Cargo.lock`, `rust-toolchain.toml`, `[workspace.metadata.rayx]`), then the gpux-checkout profile (a workspace containing `gpux-testkit`), then built-in defaults equal to the pins RayX and Gpux use (`nightly-2026-06-23`, JDK 21). Each pin carries its source for doctor.

### Reasoning: RayX xtask could hard-code versions and find its repository at compile time because it was built from the repository it served. One installed `rayx` serves projects pinned to different gpux and RayX versions (RayX builds Android against SDK 34 while gpux pins 36), so the project must own its pins. The gpux-checkout profile keeps gpux working before Gpux declares `[workspace.metadata.rayx]`, which is a follow-up.

### Invariants

- One `Pins` value flows to setup, doctor and the app pipeline; no module re-reads pins on its own.
- Built-in defaults live in one table in `project::pins`.

### Supersedes: rayx:.agentx/memory/decisions/tooling/validation/2026-07-24-tooling-validation-manifest-relative-xtask-paths.md
