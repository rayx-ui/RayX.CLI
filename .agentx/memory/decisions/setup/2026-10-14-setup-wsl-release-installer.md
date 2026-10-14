---
agentx:
  kind: decision
  scope: decisions/setup
  status: current
  created: 2026-10-14T17:00:00+02:00
  updated: 2026-10-14T17:00:00+02:00
  promoted_from: rayx-cli-releases
  topics:
    - wsl
    - clone
    - installer
  supersedes:
    - 2026-10-08-setup-wsl-clones-only-the-current-checkout.md
  superseded_by: []
---

## Decision: Set WSL up from Windows, install the Linux rayx with the release installer and clone only the current checkout

### Context: Epic `rayx-cli-releases` (design decision `d-wsl-release-installer`), 2026-10-14. Supersedes `d-wsl` of `rayx-cli-foundation` for the Linux binary; the rest is restated.

### Decision text

`setup::wsl` runs on Windows only and drives `wsl.exe`: `wsl --version` for WSL 2, `wsl -l -q` for the distribution, `wsl --install -d Ubuntu-24.04` when missing, and `id -un` to detect a missing default user. Scripts inside the distribution run with `--exec bash -lc`.

The Linux `rayx` is the running version installed by that release's own installer: when `$HOME/.rayx/bin/rayx --version` does not print `rayx <version>`, the distribution runs `set -o pipefail; curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rayx-ui/RayX.CLI/releases/download/v<version>/rayx-cli-installer.sh | env -u RAYX_HOME sh && test -x "$HOME/.rayx/bin/rayx"`. The installer picks the musl archive of the architecture, installs into `~/.rayx/bin`, edits the shell profiles and writes the receipt, so `rayx self update` works inside WSL. An architecture without a Linux release fails before any download. `RAYX_WSL_BINARY` copies a local build to `$HOME/.rayx/bin/rayx` through `wslpath` (no receipt). The inner run is `$HOME/.rayx/bin/rayx setup <same flags>` with inherited stdio.

`--clone` copies only the current checkout (translating `D:\...` to `/mnt/d/...` and keeping its branch) and lets Cargo fetch every dependency inside WSL. `rayx wsl status` and `rayx wsl compact` share a `WslHost` view of the `Lxss` registry key and `wsl.exe` output; compaction runs the optional build-output deletion, `fstrim`, a confirmed `wsl --shutdown`, and `Optimize-VHD` or `diskpart` in one elevated child.

### Reasoning: The release already ships a verified, receipt-writing installer for every Linux architecture, so the CLI keeps no download and extraction code of its own and WSL gets self update for free. A CLI that followed `path` entries to clone sibling checkouts would build a second dependency mechanism (owner, 2026-10-07).

### Invariants

- Never enable WSL sparse disks.
- Never delete anything outside the listed build-output directories, and never without confirmation unless `--yes` was given.
- Never follow a manifest's `path` dependencies to clone, copy or download another repository.
- `--check` downloads and installs nothing.

### Supersedes: [Set WSL up from Windows and clone only the current checkout](2026-10-08-setup-wsl-clones-only-the-current-checkout.md).
