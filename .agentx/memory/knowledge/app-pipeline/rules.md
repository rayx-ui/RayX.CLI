---
agentx:
  kind: rules
  scope: knowledge/app-pipeline
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T23:59:00+02:00
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
# App pipeline Rules

- Keep RayX xtask's app grammar, flags and error text: `app <dir> <action> <target>`, release by default, `--release` rejected, `rayx_diagnostics` only for diagnostic-harness apps and never for pack, publish or deploy.
- Generated Android, iOS and WASM entry crates depend on `rayx` with the same source the app itself uses (as `cargo metadata` reports it); never a fixed path.
- Join `target/`, `artifacts/` and `artifacts-temp/` to the selected workspace root on every target; keep the artifact layout byte-compatible with xtask for RayX apps.
- Commands work for any app that uses RayX with no configuration: no app name, slug or RayX repository path is special. `test wasm` finds the Playwright package (workspace metadata, workspace `tools-node/`, app `tests/e2e/`) and creates the app-local one from the embedded template when none exists; it never overwrites user edits.
- An app without `rayx` or `gpux-fonts` in its graph still builds for the web (the generated entry crate and the font root are optional); `tests/fixtures/threaded_wasm_app` keeps that true.
- A web app that exits at start-up shows `WebPlatform::quit` with no error: RayX swallows the launch error on wasm. Find the cause by logging it in `run_gpui_application`, not by suspecting `rayx`; compare with RayX xtask first.
- Check the target's prerequisites before building and install unprivileged ones automatically; privileged ones prompt or stop with the `rayx setup --<set>` command.
- The moved xtask app pipeline (`src/app/{android,desktop,ios,wasm,assets,descriptor,playwright}.rs`) still starts programs with `std::process::Command` through `app::process`, not `host::Runner`; this is a recorded exception to the repository rule, kept because the move had to preserve behavior. New code (`prereqs`, `fmt`, `wsl`, `setup`) goes through `Runner`; migrate the app pipeline module by module when its behavior is next changed.
- Scripts that run inside WSL use `wsl -d <name> --exec bash -lc <script>`: `--` hands the arguments to the distribution's login shell, which expands the quotes and `$HOME` first.
- `rayx` flags (`--no-install`, `--debug`, features) are read only before `--`; everything after it is the app's.
- Build the iOS entry crate from the project's own `Cargo.lock` (copy it next to the generated manifest): a fresh resolution picks crates the project never built (`backtrace` 0.3.76 does not compile against `libc` 0.2.190 for iOS). The Android and web entry crates still resolve on their own, which is verified on Windows.
- On macOS point `CC`, `CXX` and `AR` for `wasm32-unknown-unknown` at Homebrew's LLVM (`macos_llvm_env`) unless the user set them: Apple's clang has no wasm32 backend and build scripts such as `arborium-sysroot` compile C.
- Choose the iOS Simulator from `xcrun simctl list devices available` by the id group, not the last parenthesis (that is the state): reuse a booted iPhone, else the newest listed one, and `simctl bootstatus` before installing. `pack ios` takes `--simulator` and `--device` like `build ios`.
- An iOS app's Xcode Rust script phase must declare the static library as `outputFiles`, and a RayX iOS app strips dead code in Release (`DEAD_CODE_STRIPPING: YES`) so `io-surface`'s macOS-only CGL references do not fail the link. A RayX app's `main.rs` is compiled inside the generated entry crate, which only sees `rayx` and the app's own library: put Gpux-only iOS code in the library.
