---
agentx:
  kind: decision
  scope: decisions/repository
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
  topics:
    - agentx-memory
    - taxonomy
  supersedes: []
  superseded_by: []
---

## Decision: Organize RayX CLI memory by the CLI's module boundaries

### Context: The repository was created on 2026-10-07 with no source; its first epic, `rayx-cli-foundation`, defines the modules (`host`, `project`, `setup` with `doctor` and WSL, `app` with `fmt`, `update` with the release setup). Memory had to exist before implementation so sprint packets can route to domains.

### Alternatives considered: Copy RayX's runtime/ui/platform/tooling taxonomy; one flat domain until code exists; one domain per planned module; per-OS domains for setup.

### Reasoning: Domains follow the epic's module ownership, which is also how future changes will be scoped: `host` (machine facts, command execution, PATH), `project` (discovery and pins), `setup` (requirement sets, installers, doctor, WSL), `app-pipeline` (the app commands moved from RayX xtask, `fmt`, the app-project contract), `distribution` (CI, releases, self update) and `repository` (governance and this tree). RayX's taxonomy describes an application stack and has no place for installers; one flat domain would mix unrelated rules; per-OS setup domains would split one step table. Seeded rules and knowledge come from the owner's 2026-10-07 decisions and the research behind them, not from code.

### Trade-offs accepted: State files describe planned shapes and link to the epic and to the RayX and Gpux sources being replaced until the CLI's own source exists; implementation sprints add source backlinks. Subdomains are left out until a domain grows enough to need them. Excluded: `examples`, `ui` and `runtime` (no app code here) and a `validation` domain (validation rules live in `AGENTS.md` until they need more).

### Supersedes: None.
