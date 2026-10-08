---
agentx:
  kind: decision
  scope: decisions/app-pipeline
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - playwright
    - template
  supersedes: []
  superseded_by: []
---

## Decision: Embed a Playwright package template and write it once into the app

### Context: Epic `rayx-cli-foundation` (design decision `d-playwright-template`), 2026-10-07 to 2026-10-08.

### Decision text

The Playwright package template lives in `src/app/templates/playwright/` and is embedded with `include_str!` from inside the package (no file outside the crate). It is rendered with the project's Playwright pin and written once into `<app>/tests/e2e/`; later runs never overwrite user edits. The WebGPU launch flags come from one table in the template, matching what was verified on 2026-10-06 (Playwright 1.59.1, Chromium 147).

### Reasoning: A Playwright package must be an ancestor of the spec files for `@playwright/test` to resolve, so a package kept in a per-user cache cannot run specs that live in the app. Writing the package into the app once makes `test wasm` work with no configuration and leaves the app owning its tests, which is also what `rayx new` will generate later.

### Supersedes: None.
