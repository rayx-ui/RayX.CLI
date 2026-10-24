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
    - guests
    - wsl
    - lima
    - records
  supersedes: []
  superseded_by: []
---

## Decision: Drive WSL and Lima guests through one guest layer, record them, refresh them and replace them side by side

### Context: Carried over from RayX.CLI.Old (`.agentx/memory/decisions/setup/2026-10-14-setup-guests-lima-records-and-side-by-side-migration.md` and its successor of 2026-10-15) and ported by epic `rayx-1-0` (design `d-port`, criterion PORT-4).

### Decision text
A guest is a kind (a WSL distribution on Windows, a Lima VM on macOS) and a name, driven by `setup::guest`: rayx installs the Linux `rayx` of the chosen version with that release's installer from `rayx-ui/RayX.CLI`, clones only the current checkout and runs `rayx setup <flags>` inside. `setup.json` records the sets and project roots of real runs; `guests.json` records the guests (kind, name, Ubuntu, flags, clones, VM settings, retired, migrating), both in schema version 1 at their 1.x locations, written atomically and never on `--check`. `setup --update` re-runs the recorded setups and refreshes every current guest; a guest older than the Ubuntu pin is replaced side by side and the old one kept, retired, until `rayx lima|wsl remove`.

For NSH: `--wsl` runs no Windows-side GPU step, recorded flags never contain `--gpu` (an old record's is dropped when read), and `wsl compact --clean` also removes `.nsh/` build output.

### Invariants
- Guests rayx did not record are never started, edited, refreshed or removed.
- Nothing is deleted except by `lima remove`, `wsl remove` and `wsl compact --clean`, always confirmed unless `--yes`.
- A failed read is never presented as an absent guest or an empty record.

### Supersedes: None in this repository.
