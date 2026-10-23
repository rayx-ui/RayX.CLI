# RayX CLI contributor guidance

`rayx` is the RayX command-line tool. It sets up a machine for NSH and RayX work (`rayx setup`, `rayx doctor`, WSL and
Lima guests), updates itself (`rayx self update`), and runs any [Native Shell (NSH)](https://github.com/dXnSix/nsh) app
through the `nsh` CLI that the app pins (`rayx app`, `rayx new`, `rayx nsh`, `rayx serve`). The accepted design is
[docs/architecture/rayx-nsh-proposal.md](docs/architecture/rayx-nsh-proposal.md); its decision log (D1 to D15) wins
over anything older.

The repository is public (`rayx-ui/RayX.CLI`, Apache-2.0). It is a new CLI, not a continuation: the previous one, for
GPUI and gpux apps, is `RayX.CLI.Old` (`../RayX.CLI.Old`, `rayx-ui/RayX.CLI.Old`), whose machine side is ported here.
NSH comes from the fork `github.com/dXnSix/nsh` (checkout `../../dXn/nsh/nsh`); RayX (`../RayX`) is the NSH framework
whose apps rayx runs.

## Layout

- One Cargo package `rayx-cli` at the root: library `rayx_cli` (`src/lib.rs`) and a thin binary `rayx`
  (`src/main.rs`). Modules follow the CLI: `cli`, `host`, `project`, `setup` (with `doctor` and the WSL and Lima
  guests), `nsh` (the pin, the store, installing and running nsh), `app`, `new`, `serve`, `fmt`, `update`. Add a crate
  only when a second consumer needs a module.
- Dependencies must be fetchable by anyone who clones this public repository, on any machine and CI runner: crates.io;
  public git repositories over `https://` pinned to a `rev`; and `[patch]` for a recorded reason, with a comment
  directly above it linking the upstream issue or pull request. No `path` dependency on a sibling checkout, no git
  dependency on a `branch` or `tag`, no `[replace]`, and no dependency on a private repository
  (`tests/dependency_sources.rs` enforces this).
- rayx names NSH, its toolchain, and nothing above it (D14): no RayX, gpux or GPUI library and no app, whatever its
  source. It does not depend on NSH's crates either: it runs the `nsh` CLI the project pins. `tests/library_guard.rs`
  enforces it for non-test `src/` and the CLI's `Cargo.toml`/`Cargo.lock`, with an allowlist whose every entry must
  still match.
- Nothing decides which project to work on from where `rayx` was compiled: no `env!("CARGO_MANIFEST_DIR")` outside
  tests. Pins come from the project (`Cargo.lock`, `rust-toolchain.toml`, `[workspace.metadata.rayx]`, an app's
  `rayx.toml`); built-in defaults live in one table in `project::pins`.
- Every external program runs through `host::Runner`. `rayx` never reads, stores or forwards a password or token,
  never relaunches itself elevated, and accepts licenses only with `--yes`.
- rayx never runs the `nsh` on `PATH`. It runs the build of the project's pin from its store (`$RAYX_HOME/nsh`, else
  `~/.rayx/nsh`), in the directory it read the pin in, without `CARGO` in the environment.

## Validation

Integration tests are feature groups in `tests/<feature>.rs` (one file per feature), with fixture projects in
`tests/fixtures/<name>/`, each carrying its own empty `[workspace]` table. A new feature or behaviour change gets a new
group. While iterating, run only the touched group:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --test <group>
```

Real proofs, which download or build NSH and run apps, are the `proof_*` groups. Their tests are `#[ignore]`d and run
on demand: `cargo test --test proof_<name> -- --ignored`.

## Consumers

Gamify (`../../Six/Six.Gamify`) and RayX's apps run through `rayx`. A change to `rayx app`, the pins, `rayx.toml` or
the store layout needs the matching consumer change; keep them paired.

## NSH fork work

Work in the fork follows its own rules (the fork's `private/fork-docs` README): `feat/*` and `fix/*` branches that
could go upstream, and `fork/*` branches for fork-only changes such as the release workflow (D9), each in a worktree
under `../../dXn/nsh/nsh-wt/` and merged into the fork's `main` through a pull request. No AgentX or RayX files in the
fork, never a push to `upstream` (longbridge/nsh), never a `v*` tag.

## Commits

Use Conventional Commits with meaningful scopes. Never add Claude co-author or other AI attribution lines. Code ported
from RayX.CLI.Old names the commit it came from (`d006250`) in the commit message.

## Releases

release-plz turns Conventional Commits into `vX.Y.Z` tags; cargo-dist builds the binaries and the GitHub Release. The
first release is 1.0.0 (D4). `rayx` stays on 1.x while the owner is its only consumer: never mark a commit breaking
(no `!` and no `BREAKING CHANGE:` footer). A break is committed as `feat` (or `fix`) whose body states it, with a
`Migration:` footer for consumers, and ships as a minor release. Pushing, tagging and publishing need the owner's
go-ahead.

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

The first epic is `rayx-1-0` (`.agentx/tasks/rayx-1-0/`); its task `t14-memory` creates the memory tree, which then starts at `.agentx/memory/knowledge/INDEX.md`.
<!-- agentx:end -->
