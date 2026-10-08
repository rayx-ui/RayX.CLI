# Using `rayx app` in a project

`rayx app` builds, runs, packages, publishes, deploys and tests an app that uses RayX. It works in
any Cargo workspace: nothing is read from the RayX repository or from where `rayx` was built. This
page is the contract between a project and the tool.

```text
rayx app [<app-dir>] <action> <target> [options]
```

`<app-dir>` is a path absolute or relative to the current directory and defaults to `.`.

| Action | Targets | What it does |
| --- | --- | --- |
| `build` | `host`, `windows`, `linux`, `macos`, `mobile`, `ios`, `android`, `wasm` (alias `web`) | Builds the app for the target. |
| `run` | the same | Builds and runs the app (WASM: serves it). |
| `pack` | the same | Builds and copies the app and its content to `artifacts/`. |
| `publish` | the same | Stages like `pack`; store upload and signing are not automated. |
| `deploy` | `android` | Installs the APK on a device or emulator. |
| `test` | `host`, `windows`, `linux`, `macos`, `wasm` | Runs the app's tests (below). |
| `assets` | `generate`, `validate`, `list` | Manages the asset catalog. |

Builds are release builds; `--debug` (or `--development`) selects a debug build. `--features`,
`--all-features` and `--no-default-features` select the app's Cargo features, and everything after
`--` goes to the app. `--no-install` turns the prerequisite step off.

## What the project provides

- **A Cargo workspace.** The workspace root and its members come from `cargo metadata` of the
  selected app. `artifacts/` (packaged output), `artifacts-temp/` (generated files) and `target/`
  are created under that root for every target.
- **`rayx` as a dependency of the app**, when the app uses RayX. The generated Android, iOS and
  WebAssembly entry crates depend on `rayx` (and on `rayx_devkit` when the app's `devkit` feature
  enables it) from the same source the app uses: its path, its git repository and revision, or its
  registry version. An app that does not depend on `rayx` gets entry crates without it.
- **The versions it wants.** `Cargo.lock` (`wasm-bindgen`), `rust-toolchain.toml` (the Rust
  channel) and `[workspace.metadata.rayx]` pin the tools; anything not pinned has a built-in
  default. See `rayx doctor` for the resolved values.

### `[workspace.metadata.rayx]`

| Key | Meaning |
| --- | --- |
| `tools-node` | The directory, relative to the workspace root, of the shared Playwright package. |
| `web-toolchain`, `node`, `jdk`, `playwright` | Tool versions. |
| `android-platform`, `android-ndk`, `android-ndk-api`, `android-build-tools`, `android-system-image` | Android SDK pins. |
| `xcode` | An exact Xcode version. |

`[patch]` entries in the workspace root manifest are repeated in the generated WebAssembly crate
when the app needs them (the IndexedDB storage crates).

### `[package.metadata.rayx]`

| Key | Meaning |
| --- | --- |
| `app_name`, `app_slug` | The slug used in `artifacts/apps/<slug>`; defaults to the package name. |
| `native_library`, `android_native_library` | The native library name when it differs from the crate's. |
| `app_content_root` | The directory holding bootstrap content such as `app_settings.json`. |
| `asset_roots`, `assets_roots`, `assets_root`, `wasm_manifest_path` | Asset catalog locations. |
| `android_application_id`, `android_activity` | Android launch component. |
| `ios_project`, `ios_scheme`, `ios_bundle_id`, `ios_app_name` | iOS project names. |
| `diagnostic_harness` | Marks an app that may use the `rayx_diagnostics` feature (never for `pack`, `publish` or `deploy`). |
| `playwright_specs` | The spec files or directories `test wasm` runs. |
| `playwright_projects` | The Playwright projects `test wasm` runs. |

## Artifacts

| Command | Output |
| --- | --- |
| `build wasm` | `artifacts/apps/<slug>/wasm/<profile>/`: `index.html`, the wasm-bindgen output and `assets/`. |
| `pack <desktop target>` | `artifacts/apps/<slug>/<target>/<profile>/`: the executable, `assets/` and the app content. |
| `build android` | `artifacts/apps/<slug>/android/`: the APK. |
| generated files | `artifacts-temp/apps/<slug>/`. |

`run wasm` serves the build with `Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp`, which the threaded build needs.

## Prerequisites

Before it builds, an app command checks what the target needs: the base set for every target, plus
the web set for `wasm` (and the test set for `test wasm`), the Android set for `android`, and the
iOS set for `ios`. What can be installed without privileges (rustup toolchains, components and
targets, `wasm-bindgen-cli`, `cargo-ndk`, the Playwright package and its browser, SDK packages) is
installed on the spot with a one-line notice. What needs `sudo`, administrator rights or a license
answer is offered in an interactive terminal; otherwise the command stops and prints the exact
`rayx setup --<set>` command. `--no-install` reports what is missing and stops.

## Tests

`rayx app <dir> test <target>` works for any app, with no configuration.

- **`test host|windows|linux|macos`** runs `cargo test -p <app package>` from the workspace root
  with the selected features and profile. Arguments after `--` go to the test binary.
- **`test wasm`** builds the app, serves it with cross-origin isolation and runs Playwright against
  it. The address reaches the specs in `RAYX_APP_URL`; `--webgpu` sets `RAYX_WEBGPU_REQUIRED=1`,
  which makes the template's fixture fail unless the app initialized graphics with the browser's
  WebGPU; `--headed` shows the browser.
- **`test android` and `test ios`** are not supported yet.

### Where the Playwright package lives

The first that exists wins:

1. `[workspace.metadata.rayx] tools-node`;
2. `tools-node/` under the workspace root, shared by the workspace's apps;
3. the app's own `tests/e2e/`.

When none exists, `rayx` creates `<app>/tests/e2e/` from its embedded template and tells you to
commit it. The template is a complete package: `package.json` pinning `@playwright/test` to the
Playwright pin with pnpm as the package manager, `playwright.config.ts` with Chromium projects at
device scale factor 1 and 2 and the verified per-OS WebGPU launch flags, a fixture that waits for
the app's canvas, an example smoke spec and a `.gitignore`. Files that exist are never overwritten.

### Which specs and projects run

- Specs: the files named with `--playwright-test <spec>...` (a `spec.ts:<line>` selector runs the
  test containing that line); else `playwright_specs`; else, in a shared package, the directory
  named like the app (`apps/test_suite` runs `test_suite/`); else, in an app-local package, every
  spec.
- Projects: `playwright_projects`, else every project of the config.

## Formatting

`rayx fmt [--check]` formats the selected workspace one member at a time (`cargo fmt -p <member>`)
so the command line stays under the Windows limit, and lists the members that fail.
