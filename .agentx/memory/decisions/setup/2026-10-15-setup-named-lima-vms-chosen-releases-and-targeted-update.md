---
agentx:
  kind: decision
  scope: decisions/setup
  status: current
  created: 2026-10-15T04:00:00+02:00
  updated: 2026-10-15T04:00:00+02:00
  promoted_from: rayx-cli-versions
  topics:
    - lima
    - guests
    - vm-names
    - release
    - update
  supersedes:
    - 2026-10-14-setup-guests-lima-records-and-side-by-side-migration.md
  superseded_by: []
---

## Decision: Name the Lima VM, never adopt one `rayx` did not create, install a chosen release and update chosen guests only

### Context: Epic `rayx-cli-versions` (design decisions `d-named-vms`, `d-guest-release`, `d-update-target`, 2026-10-15). Supersedes the guest-layer decision of `rayx-cli-guests`; everything in it that this text does not change still holds (one guest layer for WSL and Lima, `guests.json` and `setup.json` records, side-by-side migration, confirmed removal, `--check` changes nothing, the invariants).

### Decision text

**The Lima VM has a name.** `setup --lima --vm <name>` (default `rayx`, which keeps every earlier command, message and record) creates, starts, edits, migrates and records the VM `<name>`; one VM per project repository works. A name is 1 to 40 lowercase letters, digits and hyphens, starting with a letter and not `default` (`VmName`; a bad name exits 1 with the rule before any command). The retired VM of a migration is `<name>-<old Ubuntu version>` (a dot never occurs in a VM name), matched by that exact name only, so a name never matches another name's retired or migrating entries. Create, edit, migration and its resume, refresh, removal and doctor follow the name; the `INSTANCE` constant remains only as the default.

**A VM `rayx` did not create is never adopted.** Before any change `setup --lima` lists the instances strictly and refuses a name that exists in Lima but is not a recorded Lima guest, `rayx` included (the one exception is the VM an interrupted migration of that name created). This replaces the earlier rule that an unrecorded `rayx` was started and edited after a confirmation. Nothing is created while `guests.json` cannot be read.

**A release can be chosen and is never remembered.** `--release X.Y.Z` (`setup --lima|--wsl|--update`, `self update`) installs exactly that published release of `rayx` (the guest gets that tag's own installer; the host gets axoupdater's specific-version install), upgrade or downgrade. Existence is probed once, before anything changes, with a headers-only `curl -sIL -w '\nstatus=%{http_code}\n'` through `Runner::query`: 2xx exists, 404 or 410 is missing, anything else is unknown (curl's `-f` exit code is not used: macOS curl ends a 404 HEAD with exit 56). A downgrade asks `Install rayx X in <guest> over Y, an older release? [y/N]` (host: `Install rayx X over Y, an older release? [y/N]`); `--yes` skips, no terminal refuses, `--check` only prints. A release override and a binary override (`RAYX_*_BINARY`) never combine. The inner `setup --update` is skipped below `MIN_REFRESH_RELEASE` (1.1.0). Nothing about the choice is stored in `guests.json` or `setup.json`: a later plain `self update` goes to the latest release and a later plain `setup --update` brings guests back to the running version.

**An update can target guests.** `setup --update --vm <name>` (repeatable; WSL names too) refreshes only those recorded current guests and skips the host part; an unknown name is refused with the recorded names before any command. `self update` still offers to refresh everything.

### Reasoning: One VM per project needs the name threaded through every place that fixed `rayx`; refusing to adopt unrecorded VMs keeps `gpux`-shaped VMs unreachable by a typo, now that any name can be typed. A chosen release makes both directions testable and lets a project stay on its own `rayx`; remembering it would make a later update surprising. The real-Mac proof (2026-10-15) found that the status, not curl's exit code, is the portable signal for a missing release.

### Invariants

- Only recorded guests, and the VM being created under the requested name, are touched; `gpux` and other unrecorded VMs never.
- A name never matches another name's retired or migrating entries; the default name keeps every earlier behaviour.
- Existence of a chosen release is checked before any VM, distribution, record or install change; an explicit `--vm` target never touches other guests or the host.
- Nothing about a chosen release is recorded.

### Supersedes: [Drive Linux guests through one guest layer, record them, refresh them and replace them side by side](2026-10-14-setup-guests-lima-records-and-side-by-side-migration.md).
