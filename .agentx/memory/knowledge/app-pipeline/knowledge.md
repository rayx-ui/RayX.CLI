---
agentx:
  kind: knowledge
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
# App pipeline Knowledge

- RayX xtask on 2026-10-07 had about 15.8k lines and 114 tests; 24 tests read RayX's `apps/lab`, `apps/examples_mobile` or `crates/`. Its app pipeline needs only `anyhow`, `serde`, `serde_json` and `toml`; `syn`, `proc-macro2` and `rayx_code_theme_converter` belong to the chores that stay in RayX (`code-themes`, `component-index`, `theme`).
- There are no embedded Gradle or Xcode templates: apps own `platform/android/gradle` (with `gradlew` and an `app/build.gradle.kts` reading `build/generated/rustJniLibs` and the `-PrayxAndroid*` properties) and `platform/ios/project.yml` for XcodeGen. xtask generates the Rust entry crates under `artifacts-temp/apps/<slug>/{android,ios,wasm}/rust/`.
- xtask's `fmt` runs `cargo fmt -p <member>` per workspace member because `cargo fmt --all` exceeds the Windows command-line limit (`os error 206`) in RayX.
- The WASM server serves `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp` for shared memory; there is no `wasm-opt` step.
- xtask's desktop `pack` looked for binaries under the repository root while `build` used a cwd-relative `--target-dir target`, and desktop triples were fixed to x64 on Windows; the move fixes both.
