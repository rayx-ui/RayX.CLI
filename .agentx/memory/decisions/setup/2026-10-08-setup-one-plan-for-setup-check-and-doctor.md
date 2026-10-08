---
agentx:
  kind: decision
  scope: decisions/setup
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - setup
    - doctor
    - plan
  supersedes: []
  superseded_by: []
---

## Decision: Plan setup once and use the plan for setup, setup --check and doctor

### Context: Epic `rayx-cli-foundation` (design decision `d-setup-plan`), 2026-10-07 to 2026-10-08.

### Decision text

`setup::plan(host, pins, project, sets) -> Plan` expands the selected requirement sets into ordered `Step { id, set, title, probe, install: Vec<CommandSpec>, privilege, fix_hint }`. The install knowledge for each OS family lives in code tables under `setup::{linux, windows, macos}` plus cross-platform `setup::{rust, web, test_tools, android, ios}`. `setup::execute(plan, runner, mode)` probes each step, skips satisfied ones, runs the rest, and re-probes. `doctor` evaluates the same plan for every applicable set in probe-only mode and renders text or JSON (`serde_json`).

Workstation pieces are ordinary steps of their sets: Xcode through `xcodes`/`mas`, the Simulator runtime through `xcodebuild -downloadPlatform iOS`, Android Studio through winget/cask/snap, emulator, system image and a default AVD, emulator acceleration, and the `gpu` set's per-vendor driver steps. GPU detection uses OS facilities (DXGI through the `windows` crate, `lspci`/`vulkaninfo`, `system_profiler`) rather than linking a GPU library into the CLI.

### Reasoning: The original problem was four drifting copies of the same requirement lists (LINUX-WSL.md, the gpux skill, CI and the Windows script). Planning from one table, and printing that plan for agents, removes the copies and lets the user run the `sudo` commands themselves. The step contents come from today's sources: LINUX-WSL.md sections 2–3, the gpux skill's Windows reference and `install-windows-prereqs.ps1` (Windows ARM64 needs), gpux CI's package lists, and RayX xtask's `setup-android`/`setup-wasm`.

### Invariants

- One step table feeds `setup`, `setup --check` and `doctor`; there is no second requirement list.
- A probe never installs anything.
- Steps with `privilege: Root` are batched (one `sudo apt-get install` for all missing packages).

### Supersedes: None.
