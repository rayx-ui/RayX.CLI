---
agentx:
  kind: knowledge
  scope: knowledge/project
  status: current
  created: 2026-10-07T23:30:00+02:00
  updated: 2026-10-08T00:30:00+02:00
  topics:
    - project-discovery
    - pins
    - cargo-metadata
  supersedes: []
  superseded_by: []
---
# Project Knowledge

- On 2026-10-07 RayX builds Android against platform `android-34` with cargo-ndk API 31, while gpux's lane E pins `android-36` and build-tools `36.0.0`; both use NDK `28.0.12674087`. One installed `rayx` therefore cannot carry a single Android pin.
- On 2026-10-07 the owner pinned the threaded-WASM nightly to `nightly-2026-06-23` and the JDK to 21 in every repository (RayX.CLI defaults, RayX's `[workspace.metadata.rayx]` and Gradle builds, Gpux's testkit default and CI). Before that, RayX xtask and gpux-testkit's default used a bare `+nightly`, and RayX's Gradle builds and gpux lane E used Java 17. RayX's skill reference `wasm-toolchain-update.md` lists every place to move together.
- `wasm-bindgen-cli` must equal the `wasm-bindgen` version in the project's `Cargo.lock` (0.2.129 for gpux and RayX on 2026-10-07); a mismatch fails at bindgen time with a schema error.
