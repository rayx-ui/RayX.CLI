# Changelog

All notable changes to `rayx` are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). It is generated from
Conventional Commits by release-plz.

## [1.2.0] - 2026-10-14

### Features
- *(update)* install a chosen release with rayx self update --release ([#5](https://github.com/rayx-ui/RayX.CLI/pull/5))

## [1.1.0] - 2026-10-14

### Documentation
- *(verify)* record the v1.0.0 release and the macOS install
- *(verify)* list the deferred Windows, WSL and RayX CI checks
- *(verify)* record the first RayX CI run with the release-installed rayx
- *(verify)* state what the earlier RayX master runs actually showed
- *(readme)* document Lima guests, records, update, migration and guest removal
- *(verify)* record the Lima guest proof on this Mac
- *(verify)* record the host refresh and the deviations of the Lima proof
- *(verify)* list the owner-run Windows and release checks of the guest epic as open rows
- *(verify)* record the smoke test of the repaired Lima flow on this Mac
- *(verify)* add a PowerShell runbook for the owner-run WSL rows
- *(verify)* use a made-up name for the removal refusal row
- *(verify)* record the WSL guest layer proof on Windows
- *(verify)* explain the 1.0.0 refresh failure and defer the release rows
- *(verify)* fix a doubled word

### Features
- *(setup)* pin the guest Ubuntu and move WSL onto a guest-generic layer
- *(setup)* add rayx setup --lima for a Lima Ubuntu VM on macOS
- *(setup)* size and change the Lima VM, record guests and report them in doctor
- *(setup)* add rayx setup --update and offer the refresh after rayx self update
- *(setup)* migrate guests older than the pinned Ubuntu side by side and remove guests

### Fixes
- *(setup)* pass --tty=false to limactl rename so a migration cannot wait on a prompt
- *(setup)* keep guests honest across migration, failure and --check

### Tests
- *(release)* follow the guest installer into setup::guest
- *(guest)* make the pin-literal scan independent of the path separator
- *(lima)* read the Lima volume command line independently of the path separator

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
