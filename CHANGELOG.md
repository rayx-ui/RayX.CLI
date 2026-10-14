# Changelog

All notable changes to `rayx` are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). It is generated from
Conventional Commits by release-plz.

## [1.0.0] - 2026-10-14

The first release of `rayx`, the RayX command-line tool: one prebuilt binary that sets up a machine
for gpux and RayX work and builds, runs, packages and tests any RayX app.

### Features

- `rayx setup` installs everything gpux and RayX development needs (base, `--web`, `--test`,
  `--android`, `--ios`, `--gpu`, `--wsl`), asks before updating an outdated requirement, and a second
  run installs nothing. `--check` only reports what is missing.
- `rayx doctor` reports every requirement of the selected sets with the version found and the exact
  fix, plus the GPU adapters, the desktop session and the WSL virtual disks; `--json` is for tools.
- `rayx wsl status` and `rayx wsl compact` show the WSL distributions and give unused virtual disk
  space back to Windows; `rayx setup --wsl` installs WSL 2 with Ubuntu and the Linux `rayx` in it.
- `rayx self update` updates an installed `rayx` to the latest release (`--check` only looks).
- `rayx app` builds, runs, packages, publishes, deploys and tests a RayX app for host, WASM, Android
  and iOS targets in any project, and `rayx fmt` formats the project one workspace member at a time.
- Version pins come from the project (`Cargo.lock`, `rust-toolchain.toml`,
  `[workspace.metadata.rayx]`) before the built-in defaults.

### Build and CI

- Release binaries for Windows x64 and ARM64, Linux x64 and ARM64 (static musl) and macOS ARM64 and
  x64, with SHA-256 checksums, GitHub artifact attestations and shell and PowerShell installers that
  install into `~/.rayx/bin` (`$RAYX_HOME/bin` when set).
- CI on Windows, macOS and Ubuntu, and release automation with release-plz and cargo-dist.
