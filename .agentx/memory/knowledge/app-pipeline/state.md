---
agentx:
  kind: state
  scope: knowledge/app-pipeline
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-07T23:30:00+02:00
  topics:
    - app-commands
    - wasm
    - android
    - ios
    - assets
    - fmt
    - app-project-contract
  supersedes: []
  superseded_by: []
---
# App pipeline State

## Purpose And Usage

Read before changing app builds, generated entry manifests, the WASM server or Playwright runs, mobile packaging, asset staging, `rayx fmt`, or the `[package.metadata.rayx]` contract.

## Current Technical Shape

Implemented by the epic `rayx-cli-foundation` (sprint 4; design decisions `d-app-move`, `d-fmt`, `d-cli-surface`). RayX xtask's app modules live in `src/app/` (copied in commit `bbb3bb6` from RayX `d69d375e`, then decoupled) and take a `ProjectContext` (`context.rs`: project, pins, host) from real discovery; `rayx app` grammar and flags are unchanged, plus `--no-install`.

- `descriptor.rs`: `AppDescriptor::resolve(context, dir)`; entry crates for Android, iOS and WASM copy the app's own `rayx`/`rayx_devkit` dependency source (`dependency_toml`), omit `rayx` when the app has none, and repeat the selected workspace's root `[patch]` for the IndexedDB storage crates.
- `prereqs.rs`: `required_sets(action, target)`, `ensure_plan`: missing unprivileged silent steps install with one notice; steps needing sudo/admin, a license answer (`Step::prompts`, or a `winget` install without `--accept-package-agreements`) are offered in a terminal or stop with `rayx setup --<set>`; `Step::optional_for_builds` (Android Studio) is never waited for. `--no-install` stops. `run_unchecked` skips the step for tests.
- `playwright.rs` + `templates/playwright/`: package resolution (workspace metadata `tools-node`, `tools-node/`, app `tests/e2e/`, else created from the template, never overwritten), spec and project selection (`playwright_specs`/`playwright_projects`), `pnpm` found by absolute path, `RAYX_APP_URL`/`RAYX_WASM_URL`/`RAYX_WEBGPU_REQUIRED`. `test host|windows|linux|macos` runs `cargo test -p <package>` from the workspace root; android/ios say "not supported yet".
- `src/fmt.rs`: `rayx fmt [--check]` per workspace member through `Runner`.
- Tests: unit tests (`cargo test --lib app::`, 100+) and groups `app_project`, `app_prereqs`, `app_test_command`, `app_playwright_template`, `app_wasm_fixture` (the last two are `#[ignore]`d without the web and test sets; run with `-- --include-ignored`), `fmt_command`, over `tests/fixtures/{app-workspace,threaded_wasm_app,fmt-workspace}` and `tests/common`. RayX source-policy tests (Lab and Examples Mobile layout) were left behind for RayX to adopt.
- Real-RayX proof (APP-4, 2026-10-08): Lab and Test Suite WASM, Lab host and Examples Mobile Android build through `rayx`; the headed WebGPU spec passes only against `RayX.Dtcg` `main` (`scripts/prove-rayx-spec.py`): the conformance branch's token runtime rejects RayX's theme tokens, so no RayX web app starts, with `rayx` or RayX xtask alike.

`docs/app-project.md` documents the app-project contract for a later `rayx new`.

## Backlinks

- [Epic design (`d-app-move`)](../../../tasks/rayx-cli-foundation/design.json)
- [RayX xtask command grammar](../../../../../RayX/xtask/src/command.rs)
- [RayX xtask app metadata and entry manifests](../../../../../RayX/xtask/src/app.rs)
- [RayX xtask WASM build, server and tests](../../../../../RayX/xtask/src/wasm.rs)
- [RayX xtask guide](../../../../../RayX/docs/src/xtask.md)
- 2026-06-29 current [RayX app-directory xtask decision](../../../../../RayX/.agentx/memory/decisions/platform/packaging-targets/2026-06-29-platform-packaging-targets-app-directory-xtask.md)
