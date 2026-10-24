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
    - wsl
  supersedes: []
  superseded_by: []
---

## Decision: Run scripts inside WSL with --exec so the login shell does not parse them

### Context: Carried over unchanged from RayX.CLI.Old (`.agentx/memory/decisions/setup/2026-10-08-setup-wsl-scripts-run-with-exec.md`).

### Decision text
Scripts inside a distribution run as `wsl -d <name> --exec bash -lc <script>`. `wsl -d <name> -- ...` hands the arguments to the login shell, which expands quotes and `$HOME` before bash sees them.

### Trade-offs accepted: A script is a single argument; paths and flags in it are checked for quotes and control characters.

### Supersedes: None in this repository.
