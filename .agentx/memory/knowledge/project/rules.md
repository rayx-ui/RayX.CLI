---
agentx:
  kind: rules
  scope: knowledge/project
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T01:30:00+02:00
  topics:
    - project-discovery
    - pins
    - cargo-metadata
  supersedes: []
  superseded_by: []
---
# Project Rules

- Find the project from the paths the user gives (`cargo locate-project --workspace`, `cargo metadata`), never from where `rayx` was compiled; no `env!("CARGO_MANIFEST_DIR")` outside tests.
- Pins belong to the project. Resolve them in this order and report the source: project (`Cargo.lock`, `rust-toolchain.toml`, `[workspace.metadata.rayx]`), the gpux-checkout profile, then the built-in defaults table in `project::pins`. No other module hard-codes a version.
- Run every discovery and probe command with `RUSTUP_AUTO_INSTALL=0`: a read-only look must not let rustup install a pinned toolchain, and discovery falls back to the manifests when cargo cannot run.
- Keep the CLI's nightly and JDK defaults equal to the pins RayX and Gpux use; move them together with the RayX skill reference `wasm-toolchain-update.md`.
