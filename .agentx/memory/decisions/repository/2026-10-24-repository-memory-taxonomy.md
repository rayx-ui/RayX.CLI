---
agentx:
  kind: decision
  scope: decisions/repository
  status: current
  created: 2026-10-24T03:30:00+02:00
  updated: 2026-10-24T03:30:00+02:00
  promoted_from: rayx-1-0
  topics:
    - repository
    - agentx-memory
    - taxonomy
  supersedes: []
  superseded_by: []
---

## Decision: Organize rayx memory by the CLI's modules

### Context: Epic `rayx-1-0` creates this repository's memory once the machine side is ported (task t14, design `d-memory`), carrying over the decisions of RayX.CLI.Old that still hold.

### Alternatives considered: RayX.CLI.Old's taxonomy with `app-pipeline` (its pipeline is gone); one flat domain; per-OS setup domains.

### Reasoning: Domains follow the CLI's modules, which is how changes are scoped: `host`, `project`, `setup` (sets, engine, doctor, guests), `nsh` (pin, store, installs, running nsh), `app` (`rayx app`, `new`, `serve`), `distribution` (CI, releases, self update) and `repository` (governance and this tree). The old app-pipeline decisions stay behind with the pipeline.

### Trade-offs accepted: `nsh` and `app` describe planned shapes until their sprints land, and gain source backlinks then. No subdomains yet.

### Supersedes: None.
