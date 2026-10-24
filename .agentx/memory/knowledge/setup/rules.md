---
agentx:
  kind: rules
  scope: knowledge/setup
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  topics:
    - setup
  supersedes: []
  superseded_by: []
---

# Setup Rules

- Keep one step table per requirement; setup, `--check`, doctor and app prerequisites all read it.
- Probe before installing and re-probe after; a second `rayx setup` executes nothing.
- Accept licenses only with `--yes`; never let a probe or `--check` mutate the machine.
- Never install a nightly, `rust-src`, XcodeGen, `cargo-ndk`, Vulkan tools or GPU drivers (D3, D15).
- Touch only recorded guests; ask before an outdated requirement is updated, never before a first install.
