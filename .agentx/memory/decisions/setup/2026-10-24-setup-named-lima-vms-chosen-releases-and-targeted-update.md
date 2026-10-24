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
    - lima
    - releases
  supersedes: []
  superseded_by: []
---

## Decision: Name the Lima VM, never adopt one rayx did not create, install a chosen release and update chosen guests only

### Context: Carried over from RayX.CLI.Old (`.agentx/memory/decisions/setup/2026-10-15-setup-named-lima-vms-chosen-releases-and-targeted-update.md`); epic `rayx-1-0` moves the release floor to this CLI's first release (criterion PORT-6).

### Decision text
`setup --lima --vm <name>` (default `rayx`) creates and manages the VM `<name>`; a VM that exists in Lima but is not a recorded guest is refused. `--release X.Y.Z` installs exactly that published release (probed with a headers-only `curl -sIL -w`, downgrades confirmed, never remembered); 1.0.0 is the oldest release that can be named and every release has the refresh (`release::OLDEST_RELEASE`, `MIN_REFRESH_RELEASE`). `setup --update --vm <name>` refreshes only the named recorded guests.

### Invariants
- Only recorded guests and the VM being created under the requested name are touched.
- Nothing about a chosen release is recorded.

### Supersedes: None in this repository.
