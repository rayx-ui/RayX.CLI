---
agentx:
  kind: knowledge
  scope: knowledge/app
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T06:00:00+02:00
  topics:
    - app
  supersedes: []
  superseded_by: []
---

# App Knowledge

- clap swallows a leading `--` after a trailing positional, so `rayx app` captures nsh's words through an external subcommand.
- `nsh init` writes `[workspace]` into a Rust app's `Cargo.toml` and names its crate `nsh-app-<slug>`; its conflict list omits `.cargo/config.toml` and `rayx.toml`.
- `nsh package --target web --out <dir>` writes the site to `<dir>/<name>`; `nsh run --target web` prints the page URL and opens no browser; nsh boots an iOS simulator when none is booted.
