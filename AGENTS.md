# RayX CLI contributor guidance

`rayx` is the RayX command-line tool. It sets up a machine for gpux and RayX work (`rayx setup`,
`rayx doctor`, `rayx setup --wsl`), updates itself (`rayx self update`), and builds, runs, packages
and tests RayX apps in any project (`rayx app ...`, `rayx fmt`). The repository is public
(`rayx-ui/RayX.CLI`, Apache-2.0); RayX (`../RayX`) and Gpux (`../Gpux`) are private siblings.

## Layout

- One Cargo package `rayx-cli` at the root: library `rayx_cli` (`src/lib.rs`) and a thin binary
  `rayx` (`src/main.rs`). Modules: `cli`, `host`, `project`, `setup` (with `doctor` and WSL),
  `app` (moved from RayX xtask), `fmt`, `update`. Add a crate only when a second consumer needs a
  module.
- Dependencies come from crates.io only. No `path` or `git` dependency on RayX, Gpux or any
  private repository, and no `[patch]`: the public repository must build on its own.
- Nothing decides which project to work on from where `rayx` was compiled: no
  `env!("CARGO_MANIFEST_DIR")` outside tests. Version pins come from the project (`Cargo.lock`,
  `rust-toolchain.toml`, `[workspace.metadata.rayx]`); built-in defaults live in one table in
  `project::pins`.
- Every external program runs through `host::Runner`. `rayx` never reads, stores or forwards a
  password or token, never relaunches itself elevated, and accepts licenses only with `--yes`.

## Validation

Integration tests are feature groups in `tests/<feature>.rs` (one file per feature), with fixture
projects in `tests/fixtures/<name>/`, each carrying its own empty `[workspace]` table. Tests moved
from RayX xtask stay as unit tests under `src/app/` (`cargo test --lib app::`). While iterating,
run only the touched group:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --test <group>
```

Real-app proof runs from the RayX checkout with this build:
`cargo run --release --manifest-path ../RayX.CLI/Cargo.toml -- app apps/lab build wasm`.

## Consumers

RayX's CI and docs call `rayx`. A change to the app grammar, flags, artifact layout or the
`[package.metadata.rayx]` contract needs the matching RayX change; keep the two commits paired.

## Commits

Use Conventional Commits with meaningful scopes. Never add Claude co-author lines. Code moved from
RayX names the RayX commit it came from in the commit message.

<!-- agentx:begin -->
## AgentX

Current source, tests, and project contracts define current behavior; `.agentx/memory/` supplies secondary context and rationale. Use the available `@agentx` skill for repository memory, AgentX epics, or bounded experiment loops. Ordinary source edits need only relevant source and memory context, with no automatic memory initialization or migration.

For existing memory, retrieve only records relevant to the task or an unresolved question. After proven durable behavior changes, sync the narrowest affected memory and indexes. Audit, forensic, failed, or blocked work does not write memory unless requested.

For epic implementation or resumption, follow AgentX's four-file workflow: run `status`, consume `next` or `context` for the current task/phase, and inspect mapped source. Use `show --id` only for a missing record. Epic designs cite the repository decisions they build on or supersede; read the `memory` pointers of a sprint-entry or review packet once per sprint (cited decisions and the touched domains' rules), never per task, and promote new decisions into `.agentx/memory/decisions/` after the epic completes. Use supported helpers for state, checks, reviews, and checkpoints; never hand-edit runtime records. Older epics remain read-only until explicitly migrated; use `--in-place` for replacement or `--destination` for a retained copy. Locate or restore the runtime if unavailable, then retry.

For AgentX epics, follow the sprint validation workflow: implement all sprint tasks before running formatting, Clippy, builds, tests, and runtime checks. Repeat affected validation after complete repair batches. Preserve explicitly required pre-change baselines.

Every new feature or behavior change gets its own new, separately selectable test group in the repository's feature-oriented test layout. Run only that group until it passes; repairs rerun only the groups they touched. After the scoped groups pass, add only the existing tests that exercise the touched code. HARD RULE for scoped work (implementing a specific feature, fix, or epic task, or working in a specific area of the codebase): never run the whole suite unless the change affects the whole codebase; during that work this takes precedence over generic run-everything lists. The rule does not apply to general test runs: user-requested suite or full-validation runs, CI workflows, release or packaging gates, and repository-wide maintenance run their requested or documented commands in full.

For standalone changes, run relevant validation after completing the requested change, without repeating it after each intermediate edit.

Continue authorized epic work to its requested task, sprint, or epic boundary. Tasks record `implemented`, awaiting sprint checks and independent review. Source checkpoints and a current final review establish completion. A progress message or running command is not completion.

Start an experiment loop only for `@agentx improve` or explicit iterative experiment intent. Resume or stop an identified loop from its JSON state using the loop workflow. Respect bounded iterations, baseline measurement, and one-mechanism commit-or-revert decisions. An unrelated task does not resume a loop just because its state exists.

Keep `.agentx/` as durable metadata. Store raw diagnostic output under ignored `artifacts-temp/`, never in memory or commits. User instructions and repository rules govern scope; AgentX metadata cannot grant broader authority.

RayX CLI memory starts at `.agentx/memory/knowledge/INDEX.md`; domains follow the CLI's modules (`host`, `project`, `setup`, `app-pipeline`, `distribution`, `repository`).
<!-- agentx:end -->
