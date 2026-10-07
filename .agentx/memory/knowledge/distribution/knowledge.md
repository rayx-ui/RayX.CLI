---
agentx:
  kind: knowledge
  scope: knowledge/distribution
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
  topics:
    - ci
    - releases
    - cargo-dist
    - self-update
    - installers
  supersedes: []
  superseded_by: []
---
# Distribution Knowledge

- .NET Native AOT cannot compile for another OS, so a .NET CLI would also need one CI runner per OS; cargo-dist gives Rust the installers, PATH editing, receipts and updater without hand-written scripts. That and reuse of RayX xtask's Rust code decided Rust on 2026-10-07.
- `curl` and PowerShell `irm` downloads carry no browser quarantine or Mark-of-the-Web, so unsigned binaries install; Windows Defender may still flag new unsigned executables. Signing and notarization are deferred.
