---
agentx:
  kind: index
  scope: decisions
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - decisions
    - rayx
  supersedes: []
  superseded_by: []
---

# rayx Decision Index

Durable architecture choices for `rayx`. Current source remains the first evidence. The decisions of RayX.CLI.Old that still hold were carried over, adapted, when epic `rayx-1-0` created this tree.

## Domains

- host
  - [Describe the host with fixture probes and run every program through one Runner](host/2026-10-24-host-facts-probes-and-one-runner.md)

- project
  - [Pins come from the project or the built-in defaults table](project/2026-10-24-project-pins-from-project-or-defaults.md)

- setup
  - [Plan setup once and use the plan for setup, setup --check and doctor](setup/2026-10-24-setup-one-plan-for-setup-check-and-doctor.md)
  - [Run scripts inside WSL with --exec so the login shell does not parse them](setup/2026-10-24-setup-wsl-scripts-run-with-exec.md)
  - [Install Android Studio through its own wizard on Windows](setup/2026-10-24-setup-android-studio-through-its-wizard.md)
  - [Drive WSL and Lima guests through one guest layer, record them, refresh them and replace them side by side](setup/2026-10-24-setup-guest-layer-records-refresh-and-replacement.md)
  - [Name the Lima VM, never adopt one rayx did not create, install a chosen release and update chosen guests only](setup/2026-10-24-setup-named-lima-vms-chosen-releases-and-targeted-update.md)

- distribution
  - [Release with cargo-dist and update with axoupdater](distribution/2026-10-24-distribution-cargo-dist-and-axoupdater.md)
  - [release-plz turns Conventional Commits into version tags; dist builds the release; CI runs on pull requests and main](distribution/2026-10-24-distribution-release-plz-tags-dist-releases.md)

- repository
  - [One package, public dependencies, and no library or app above NSH](repository/2026-10-24-repository-one-package-public-dependencies-and-library-guard.md)
  - [Organize rayx memory by the CLI's modules](repository/2026-10-24-repository-memory-taxonomy.md)
