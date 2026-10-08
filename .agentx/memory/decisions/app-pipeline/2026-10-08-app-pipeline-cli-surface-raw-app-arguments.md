---
agentx:
  kind: decision
  scope: decisions/app-pipeline
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - cli
    - app-grammar
  supersedes: []
  superseded_by: []
---

## Decision: Parse the top level with clap and hand rayx app its arguments raw

### Context: Epic `rayx-cli-foundation` (design decision `d-cli-surface`), 2026-10-07 to 2026-10-08.

### Decision text

`clap` (derive) parses the top level: `setup`, `doctor`, `self update`, `fmt`, `app`, `--help`, `--version`. `app` takes its arguments as raw trailing values and hands them to the moved xtask parser unchanged, so the app grammar, its error messages and its tests stay exactly as they are.

### Reasoning: Re-modelling the app grammar in clap would change accepted spellings and error text that RayX docs, CI and agents rely on, and would rewrite well-tested parsing for no user benefit. Top-level commands are new, so clap gives them help and errors for free.

### Supersedes: rayx:.agentx/memory/decisions/platform/packaging-targets/2026-06-29-platform-packaging-targets-app-directory-xtask.md
