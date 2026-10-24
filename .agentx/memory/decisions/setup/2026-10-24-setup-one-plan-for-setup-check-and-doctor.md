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
    - plan
    - doctor
  supersedes: []
  superseded_by: []
---

## Decision: Plan setup once and use the plan for setup, setup --check and doctor

### Context: Carried over from RayX.CLI.Old (`.agentx/memory/decisions/setup/2026-10-08-setup-one-plan-for-setup-check-and-doctor.md`); epic `rayx-1-0` ported the engine and adapted the sets for NSH (design `d-port`, `d-nsh-sets`).

### Decision text
`setup::plan(env, sets)` expands the sets (`base`, `web`, `test`, `ios`, `android`; no GPU set, D15) into ordered steps, each with a probe and an install. `setup::execute` probes, skips what is met, installs the rest and re-probes; `setup --check` prints each step's status and exact commands; `doctor` probes every applicable set and renders text or JSON. App commands reuse the same steps for their prerequisites.

### Reasoning: One table keeps setup, `--check`, doctor and the app prerequisites from drifting apart.

### Invariants
- One step table; no second requirement list in docs, CI or another module.
- A probe never installs anything; `--check` executes, installs, records and downloads nothing.
- Missing apt packages install in one batched `sudo apt-get install`.

### Supersedes: None in this repository.
