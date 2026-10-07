---
agentx:
  kind: knowledge
  scope: knowledge/host
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
  topics:
    - host-facts
    - command-runner
    - privilege
    - path
  supersedes: []
  superseded_by: []
---
# Host Knowledge

- WSL appends Windows PATH entries (`/mnt/c/...`) after Linux ones; a Linux tool missing from the Linux PATH silently resolves to the Windows executable (seen with `pnpm`). Ubuntu's `~/.profile` prepends `~/.local/bin` only in new login shells and only once the folder exists.
- WSL is identifiable from `WSL_DISTRO_NAME` or `microsoft` in `/proc/sys/kernel/osrelease`.
