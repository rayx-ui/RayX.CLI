---
agentx:
  kind: decision
  scope: decisions/setup
  status: current
  created: 2026-10-14T19:00:00+02:00
  updated: 2026-10-14T19:00:00+02:00
  promoted_from: rayx-cli-guests
  topics:
    - wsl
    - lima
    - guests
    - migration
    - records
  supersedes:
    - 2026-10-14-setup-wsl-release-installer.md
  superseded_by: []
---

## Decision: Drive Linux guests through one guest layer, record them, refresh them and replace them side by side

### Context: Epic `rayx-cli-guests` (design decision `d-guest-layer`, with `d-lima-vm`, `d-records-doctor`, `d-setup-update`, `d-guest-migration`, `d-guest-remove`), 2026-10-14. Supersedes `d-wsl-release-installer` of `rayx-cli-releases`; the WSL rules below restate it.

### Decision text

A guest is a kind (WSL distribution on Windows, Lima instance on macOS) plus a name, driven by `setup::guest`: the host runs a bash script inside it with `wsl -d <name> --exec bash -lc <script>` or `limactl shell --workdir / <name> -- bash -lc <script>`, installs the Linux `rayx` of the running version with that release's own installer (or copies `RAYX_WSL_BINARY` / `RAYX_LIMA_BINARY`; an architecture without a Linux release fails before any download), clones only the current checkout (`--clone`, recorded in the guest's `~/.config/rayx/clones`; the legacy `wsl-clones` is still read) and runs `rayx setup <flags>` inside with inherited stdio. WSL keeps what is WSL-specific (WSL 2 check, `wsl --install -d Ubuntu-<pin>`, first user, Windows GPU first, `/mnt/<drive>` translation); `setup::lima` is macOS only: macOS 13+, Homebrew, Lima 2+ (`brew install lima`, an older Lima is an update that is asked about), then only the instance `rayx` (`limactl create --tty=false --vm-type=vz ... --mount-only=<home> --containerd=none template:ubuntu-<pin>`, then `start`), the macOS home read-only by default and sizes capped to what the Mac can spare. `--lima` is never implied by `--all`. One Ubuntu pin (24.04) lives only in `project::pins`, built in, never overridden by a project.

Every real `rayx setup` records its sets and project root in `setup.json`; `setup --wsl`/`--lima` record the guests they manage in `guests.json` (kind, name, Ubuntu, flags, clones, VM settings, `retired`, `migrating`), atomically, never on `--check`; an unreadable or unparsable record is reported and left untouched, only a missing file is empty. A guest is recorded as soon as it exists, even when the run fails afterwards. `rayx setup --update` re-runs the recorded sets once per recorded project with that project's pins and refreshes every current guest of the host (`sudo apt-get update && upgrade`, the matching Linux `rayx`, `rayx setup --update` inside); `apt-get upgrade` runs only inside guests. `rayx self update` offers that refresh with the newly installed binary.

A guest older than the pin is never upgraded in place: a new guest is made from the pinned image next to it (Lima: stop, `limactl rename --tty=false rayx rayx-<old>` recorded as retired and `migrating`, a new `rayx` with the old settings overlaid with explicit `--vm-*` flags; WSL: `Ubuntu-<pin>` next to the old distribution), the recorded clones are re-cloned from their host checkouts and the recorded flags re-run, and the old guest is kept, retired, until `rayx lima remove <name>` / `rayx wsl remove <name>` (recorded guests of the right OS only, always confirmed, `limactl delete --force` / `wsl --unregister`) removes it. An interrupted migration resumes from the record.

### Reasoning: WSL and Lima differ only in how a script runs inside and how the guest is created, so one layer serves both; a record on the host is what lets doctor, update, migration and removal act on exactly the guests `rayx` made and never on the owner's own (`gpux`). Side-by-side replacement keeps the old guest until the owner decides, so no upgrade can lose work. The real-VM proof found that `limactl rename` asks to start the instance unless given `--tty=false`.

### Invariants

- Never enable WSL sparse disks; never `do-release-upgrade`; no `sudo` on the host.
- Guests `rayx` did not record are never started, stopped, edited, renamed, refreshed or removed.
- Nothing is deleted except by `lima remove` / `wsl remove` / `wsl compact --clean`, always confirmed unless `--yes`.
- `--check` downloads, creates, records, prompts and starts nothing.
- A failed read (`limactl list`, a record) is never presented as an absent guest or an empty record.
- Never follow a manifest's `path` dependencies to clone or copy another repository.

### Supersedes: [Set WSL up from Windows, install the Linux rayx with the release installer and clone only the current checkout](2026-10-14-setup-wsl-release-installer.md).
