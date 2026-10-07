---
agentx:
  kind: rules
  scope: knowledge/setup
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T00:45:00+02:00
  topics:
    - setup
    - doctor
    - wsl
    - windows-arm64
    - installers
  supersedes: []
  superseded_by: []
---
# Setup Rules

- Keep one step table per requirement; `setup`, `setup --check` and `doctor` all read it. Never add a second requirement list in docs, CI or another module.
- Probe before installing and re-probe after; a second `rayx setup` run executes nothing.
- Batch root steps (one `sudo apt-get install` for every missing package) and let agents use `--check`, which prints the exact commands for the owner to run.
- Accept SDK licenses and installer agreements only when the user passed `--yes`; otherwise the tool's own prompt reaches the user.
- Support Ubuntu/Debian only among Linux distributions for now; others fail `setup` with an actionable message while `doctor` still reports.
- Never discover, clone, copy or download a project's dependencies (gpux included) by following its manifest's `path` entries; Cargo resolves dependencies from crates.io or git. `setup --wsl --clone` copies only the current checkout (owner, 2026-10-07).
