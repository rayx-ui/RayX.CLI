---
agentx:
  kind: decision
  scope: decisions/setup
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  promoted_from: rayx-1-0
  topics:
    - setup
    - android
    - windows
  supersedes: []
  superseded_by: []
---

## Decision: Install Android Studio through its own wizard on Windows

### Context: Carried over unchanged from RayX.CLI.Old (`.agentx/memory/decisions/setup/2026-10-08-setup-android-studio-through-its-wizard.md`).

### Decision text
The step runs `winget install --id Google.AndroidStudio --exact --interactive` unelevated, so the installer's wizard raises its own UAC prompt (a silent install reports success and installs nothing), and it is `optional_for_builds`: app commands do not wait for an IDE.

### Trade-offs accepted: The owner answers prompts in the terminal and the wizard; licenses are still accepted only with `--yes`.

### Supersedes: None in this repository.
