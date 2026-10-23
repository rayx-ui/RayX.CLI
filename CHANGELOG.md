# Changelog

All notable changes to `rayx` are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). It is generated from
Conventional Commits by release-plz.

## [1.0.0] - 2026-10-24

The first release of the new `rayx`, the RayX command-line tool for Native Shell (NSH) apps. It is
a new CLI with its own history; the previous one, for GPUI and gpux apps, continues as
`rayx-ui/RayX.CLI.Old` (rayx 1.4.1).

### Features
- `rayx setup` installs the base set and the web, test, iOS and Android sets for NSH, including the
  nsh the project pins; `--check` reports, `--wsl` and `--lima` set up WSL and Lima guests, and
  `--update` re-runs the recorded setups and refreshes the guests.
- `rayx doctor` reports every requirement with its fix, the pins with their sources, the nsh store
  and the guests.
- `rayx wsl status|compact|remove` and `rayx lima remove` manage the guests.
- `rayx self update` updates `rayx`, or installs the release `--release` names.
- `rayx fmt` formats the project one workspace member at a time.
- `rayx app` runs any nsh command with the project's pinned nsh, installing what the command and
  its target need first.
- `rayx nsh status|path|install|prune` shows, locates, installs and prunes the nsh builds `rayx`
  keeps per pin.
- `rayx new` creates an NSH app pinned to `rayx`'s default nsh.
- `rayx serve` serves a packaged web app.
- Release binaries for Windows x64 and ARM64, Linux x64 and ARM64 (static musl) and macOS ARM64
  and x64, with shell and PowerShell installers and GitHub artifact attestations.
