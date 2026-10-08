---
agentx:
  kind: index
  scope: decisions
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - decisions
    - rayx-cli
  supersedes: []
  superseded_by: []
---

# RayX CLI Decision Index

Durable architecture choices for `rayx`. Current source remains the first evidence to inspect. The owner's 2026-10-07 product decisions (Rust, a public repository, slim RayX xtask) were promoted here when the epic `rayx-cli-foundation` completed.

## Domains

- host
  - [Describe the host with fixture probes and run every program through one Runner](host/2026-10-08-host-facts-and-command-runner.md)

- project
  - [Find the project through Cargo and let the project own its version pins](project/2026-10-08-project-discovery-and-pin-precedence.md)

- setup
  - [Run scripts inside WSL with --exec so the login shell does not parse them](setup/2026-10-08-setup-wsl-scripts-run-with-exec.md)
  - [Install Android Studio through its own wizard on Windows](setup/2026-10-08-setup-android-studio-through-its-wizard.md)
  - [Plan setup once and use the plan for setup, setup --check and doctor](setup/2026-10-08-setup-one-plan-for-setup-check-and-doctor.md)
  - [Set WSL up from Windows and clone only the current checkout](setup/2026-10-08-setup-wsl-clones-only-the-current-checkout.md)

- app-pipeline
  - [Parse the top level with clap and hand rayx app its arguments raw](app-pipeline/2026-10-08-app-pipeline-cli-surface-raw-app-arguments.md)
  - [Embed a Playwright package template and write it once into the app](app-pipeline/2026-10-08-app-pipeline-embedded-playwright-package-template.md)
  - [Format the workspace one member at a time](app-pipeline/2026-10-08-app-pipeline-fmt-per-member.md)
  - [Move the xtask app pipeline first, then decouple it from RayX](app-pipeline/2026-10-08-app-pipeline-move-first-decouple-second.md)
  - [Prove the real RayX Test Suite spec against a consistent sibling set](app-pipeline/2026-10-08-app-pipeline-real-rayx-proof-uses-a-consistent-sibling-set.md)

- distribution
  - [Release with cargo-dist and update with axoupdater](distribution/2026-10-08-distribution-cargo-dist-and-axoupdater.md)

- repository
  - [Organize RayX CLI memory by the CLI's module boundaries](repository/2026-10-07-repository-memory-taxonomy.md)
  - [Keep the nightly and JDK pins literal in each repository with one checklist](repository/2026-10-08-repository-literal-toolchain-pins-with-one-checklist.md)
  - [RayX keeps a slim xtask for its chores and calls rayx for everything else](repository/2026-10-08-repository-rayx-keeps-a-slim-xtask.md)
  - [Ship rayx as one Cargo package with a library and a thin binary](repository/2026-10-08-repository-single-cargo-package.md)
