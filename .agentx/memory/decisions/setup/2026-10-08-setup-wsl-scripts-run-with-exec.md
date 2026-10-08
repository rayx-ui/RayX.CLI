---
agentx:
  kind: decision
  scope: decisions/setup
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - wsl
    - quoting
  supersedes: []
  superseded_by: []
---

## Decision: Run scripts inside WSL with --exec so the login shell does not parse them

### Context: Found while implementing `rayx-cli-foundation` (2026-10-08).

### Decision text

Scripts that run inside a distribution use `wsl -d <name> --exec bash -lc <script>`. `wsl -d <name> -- ...` hands the arguments to the distribution's login shell, which expands quotes and `$HOME` before bash sees the script; `wsl status` printed "unknown" for every size until `--exec` replaced `--`.

### Reasoning: Verified against the real Ubuntu-24.04 on the development machine on 2026-10-08.

### Trade-offs accepted: The script must be a single argument; flags and paths in it are validated to contain no quotes or control characters.

### Supersedes: None.
