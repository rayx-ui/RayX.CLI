---
agentx:
  kind: decision
  scope: decisions/app-pipeline
  status: current
  created: 2026-10-08T12:00:00+02:00
  updated: 2026-10-08T12:00:00+02:00
  topics:
    - proof
    - rayx
    - dtcg
  supersedes: []
  superseded_by: []
---

## Decision: Prove the real RayX Test Suite spec against a consistent sibling set

### Context: Found while implementing `rayx-cli-foundation` (2026-10-08).

### Decision text

The headed WebGPU spec of APP-4 runs through `scripts/prove-rayx-spec.py`, which builds `artifacts-temp/rxproof/` from `git archive` of the RayX checkout and of `RayX.Dtcg` at `main`, with links to the other siblings, and runs `rayx app apps/test_suite test wasm --headed --webgpu ...` there. No checkout is touched.

### Reasoning: On 2026-10-08 the owner's `RayX.Dtcg` checkout was on its DTCG 2025.10 conformance branch, whose token runtime rejects RayX's `theme.styles.tokens.json` (`reference type mismatch`), so every RayX web app stopped at start-up (`WebPlatform::quit`, the launch error is swallowed on wasm). RayX's own xtask failed identically, so the failure was the sibling revision, not `rayx`; with Dtcg `main` the spec passed (2 passed).

### Trade-offs accepted: The proof is one commit behind the owner's live checkouts by design; when RayX adopts the conformance branch, point `--dtcg-revision` at it.

### Supersedes: None.
