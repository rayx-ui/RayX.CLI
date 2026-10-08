---
agentx:
  kind: decision
  scope: decisions/setup
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - android
    - winget
    - uac
  supersedes: []
  superseded_by: []
---

## Decision: Install Android Studio through its own wizard on Windows

### Context: Found while implementing `rayx-cli-foundation` (2026-10-08).

### Decision text

winget starts the Android Studio installer silently (`/S`), which cannot elevate itself: run unelevated it reports "Successfully installed" and installs nothing. An elevated child of `rayx` has no usable console for winget's license question either. The step runs `winget install --id Google.AndroidStudio --exact --interactive` unelevated, so the installer wizard asks for administrator rights through its own UAC prompt, and the step is marked `optional_for_builds`: app commands do not wait for an IDE.

### Reasoning: Found on 2026-10-08 while provisioning the validation machine: the silent install exited 0 and left nothing on disk (`0 changes to ARP`), and an `Admin` Runner child showed an empty `cmd.exe` window that could not be answered.

### Trade-offs accepted: The owner answers the license and UAC prompts in two places (terminal and wizard). Licenses are still never accepted without `--yes`.

### Supersedes: None.
