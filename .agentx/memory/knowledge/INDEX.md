---
agentx:
  kind: index
  scope: knowledge
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
  topics:
    - rayx-cli
    - taxonomy
  supersedes: []
  superseded_by: []
---
# RayX CLI Knowledge Index

Current source, tests, manifests and `docs/` establish behavior. This tree records durable routing, rules and rationale for `rayx`, the RayX command-line tool. Decisions live in the [decision index](../decisions/INDEX.md).

## Domains

- [host](host/INDEX.md): host facts (OS, native CPU architecture, WSL, distribution), the command runner with its execute/print/record modes and privilege handling, and user PATH editing.
- [project](project/INDEX.md): workspace discovery through Cargo and the version pins a project declares (`Cargo.lock`, `rust-toolchain.toml`, `[workspace.metadata.rayx]`), the gpux-checkout profile and built-in defaults.
- [setup](setup/INDEX.md): the requirement sets (base, web, test, android, ios), the plan/probe/execute engine, the per-OS installers for Windows x64/ARM64, Ubuntu/Debian and macOS, `doctor`, and `setup --wsl`.
- [app-pipeline](app-pipeline/INDEX.md): `rayx app [<dir>] build|run|pack|publish|deploy|test <target>` and `assets`, moved from RayX xtask and made project-agnostic: desktop, threaded WASM with its server and Playwright tests, Android, iOS, asset packaging; plus `rayx fmt` and the app-project contract.
- [distribution](distribution/INDEX.md): CI, cargo-dist releases for six targets with shell and PowerShell installers, `rayx self update`, and the README install instructions.
- [repository](repository/INDEX.md): repository governance: the dependency policy, commits, the relationship to RayX and Gpux, and this memory tree.

## Source Anchors

- [Repository instructions](../../../AGENTS.md)
- [First epic: rayx-cli-foundation](../../tasks/rayx-cli-foundation/requirements.json)
