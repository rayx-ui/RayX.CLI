---
agentx:
  kind: decision
  scope: decisions/setup
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - wsl
    - clone
  supersedes: []
  superseded_by: []
---

## Decision: Set WSL up from Windows and clone only the current checkout

### Context: Epic `rayx-cli-foundation` (design decision `d-wsl`), 2026-10-07 to 2026-10-08.

### Decision text

`setup::wsl` runs on Windows only and drives `wsl.exe`: `wsl --version` for WSL 2, `wsl -l -q` for the distribution, `wsl --install -d Ubuntu-24.04` when missing, and `wsl -d Ubuntu-24.04 -- id -un` to detect a missing default user. The Linux binary is the release asset `rayx-cli-<arch>-unknown-linux-musl` of the running version, downloaded inside the distribution with `curl` into `~/.local/bin`; `RAYX_WSL_BINARY` copies a local build instead through `wslpath`. The inner run is `wsl -d Ubuntu-24.04 -- ~/.local/bin/rayx setup <same flags>` with inherited stdio. `--clone` copies only the current checkout (translating `D:\...` to `/mnt/d/...` and keeping its branch) and lets Cargo fetch every dependency inside WSL.

`rayx wsl status` and `rayx wsl compact` share a `WslHost` view built from the `Lxss` registry key (distribution id, name, `BasePath` of `ext4.vhdx`) and `wsl.exe` output. Compaction runs: optional build-output deletion (confirmed), `sudo fstrim -av` inside the distribution, a confirmed `wsl --shutdown`, then one `Admin` child that runs `Optimize-VHD -Mode Full` when the Hyper-V PowerShell module exists or a generated `diskpart` script otherwise, and finally reports the `ext4.vhdx` size before and after.

### Reasoning: Dependencies such as gpux are moving to crates.io and GitHub, so Cargo resolves them in WSL like anywhere else. A CLI that followed `path` entries to clone sibling checkouts would build a second, competing dependency mechanism and break once the paths are gone; the owner ruled it out on 2026-10-07.

### Invariants

- Never enable WSL sparse disks.
- Never delete anything outside the listed build-output directories, and never without confirmation unless `--yes` was given.
- Never follow a manifest's `path` dependencies to clone, copy or download another repository.

### Supersedes: None.
