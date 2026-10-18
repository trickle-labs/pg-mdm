# PROVEN: GitHub issue #9 — cheaper publication and local lookups

Requirements: 5/5  
Counterexamples tested: 8 scenario groups, plus 70 refresh/full-rebuild equivalence checks.

Contract: [contract.md](/Users/grove/projects/pg-mdm2/.p2p/work/optimizations-20260907-publication/contract.md) v1  
Contract snapshot: SHA-256 `0ebcb1a6dbfcf769fa934130be479205f4073ab41caa42f3872cba724e6cd319`  
Source issue: [source-issue.md](/Users/grove/projects/pg-mdm2/.p2p/work/optimizations-20260907-publication/source-issue.md), SHA-256 `14d69479d4b99f65d280a254b2ea85c68b43be0cbc491e0c0bb209226f59e2e3`.

Parent context: Parent v1 and indexed snapshot SHA-256 `f09a262000f5c2615fce31399ea3548720c56a0a5b2619bb5f379b885463f1f5`; original approved snapshot SHA-256 `ec56d75b7929dbb73c3ff8a63698333af9dc67d7c06528a0f9d4672f6fc41b66`. S1 maps to parent R4/R5/R6/R10, baseline/local R1, and inherited R11–R14. The approved route is direct to `main`, with no integration branch or sibling prerequisite; approved section SHA-256 `b4179a3d6ff560a64bc73a33639a9c6083c05dd17dee3556321886cecdafaa90`. The indexed parent snapshot adds decomposition metadata to the original snapshot; no semantic promise change was found.

Candidate: `snapshot:sha256:65e7ae1b104557276a4081a5a541cb88b0cf400248ab7f28cf601d0698f50ff5`, based on `9a07830d062b31c108880071c27124e2f09c446a`, at [candidate workspace](/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/runtime/workspace).

Candidate stability: Unchanged. All eight tracked-file hashes match [candidate-final.json](/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/implementation/post/candidate-final.json), SHA-256 `76f2368f7865f9d4c6acb7c977a9821ccff89c396e6febbc7793b657070e0455`. The workspace remained read-only, with the same eight modifications and no untracked files.

Contract stability: Unchanged; the exact contract hash was rechecked.

Verification context: `/root/prove_issue9_fixed`; Rust/Cargo 1.99.0, cargo-pgrx 0.18.0, Docker 29.8.1, PostgreSQL 18.4. Both baseline and candidate E2E images used PostgreSQL 18.4. The candidate image ID was `sha256:fce54186d6aa77db267d70bd0f41f38513d716edf1f97bfd8f04826088cdca13`; the clean baseline image ID was `sha256:957ff8afca82879c126c54692dbd16212c3c168b6b2486bf78d5790c6fa2bf64`. The [implementation evidence correction](/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/implementation/report-correction.md), SHA-256 `13dd9a229cc5c3b4d33804bdcc3d104b1f8022cd68efed8d3b6b378136235b01`, supersedes the original report’s incorrect PostgreSQL 18.6 statement and excludes the stale cached baseline trace.

## Outcome

All five requirements are supported for this fixed candidate. The clean base trace verifies one-row-per-call publication behavior; the candidate trace verifies 128-row batches. Exact public and durable projections, injected failure/retry behavior, golden-selection outputs, and resolver oracles passed their respective checks.

Membership lookup, member-count aggregation, and empty cannot-link work reduction are established by inspecting the fixed base and candidate source paths and checking exact outputs and independent tests. There are no direct runtime operation counters for membership probes or `cannot_cross` pair visits, so no such counts are claimed. The observed timings are not a production SLO.

The candidate E2E SQL, incremental qualification, compiler canary, and physical recovery assertions completed. Its runner then exited 1 at the final dirty-tree evidence-retention guard; no full retained-E2E pass is claimed. The clean baseline trace runner stopped at its later physical-backup stage, but the exact candidate `tests/e2e.sql` replayed against the clean base image and exited 0. The initial cached baseline trace is excluded.

## Requirement verdicts

| ID | Observation and oracle | Evidence reference | Verdict |
|---|---|---|---|
| R1 | Nested SPI traces show 128 records produced 128 INSERTs per target on base (640 total) versus one 128-row INSERT per target on candidate (5 total). At 2,048 records, base produced 2,048 per target (10,240 total) versus sixteen 128-row INSERTs per target (80 total). The five targets are golden provenance, identity registry, memberships, entity output, and member output. All marked sessions had matching duration/query-text record counts and no backend errors. The exact 128-row fixture asserted 128 durable memberships and 128 public member rows for one identity, with exact entity and identity projections. The same E2E SQL exited 0 against base; candidate SQL assertions completed before the final runner guard. Failure injection compared publication state and acknowledgement, then retried successfully. | [Clean baseline summary](/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/implementation/baseline/runtime-trace-clean/summary.md), SHA-256 `785e2d49f079799d457a8d1167c90e2aaa16ac4055922fccb63d3cacc1e3ea04`; [baseline parsed counts](/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/implementation/baseline/runtime-trace-clean/parsed.json), SHA-256 `7f2279aeb39b5c6037440398ad6cc0ca4cffb5d1d279d99e1e6c742cd2541271`; [candidate trace summary](/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/implementation/post/e2e-spi-trace-summary-4.md), SHA-256 `cbb145458d8bb1d860203cbe2c1d86b44c48dee2a8b6dcaea9a8fb099c47a00c`; [base exact E2E replay log](/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/implementation/baseline/runtime-trace-clean/exact-e2e-replay.log), SHA-256 `389d6d26f278e5035ca79db906371a12e6f162610c8756fd1ff2d7fbed9b1ffb`; candidate E2E output: `implementation/post/e2e-post-edit-raw-4.log`. | proven |
| R2 | `evaluation::select_golden` builds an active-membership index once and retrieves membership and sort key by source-record ID, instead of scanning the full membership vector for each golden row. Exact policy-order and equal-override/tie/provenance tests passed, as did resolver property tests. Full output assertions provide the result oracle. No runtime membership-probe counter is available; the work-shape judgment comes from direct source-path inspection. | Candidate `src/evaluation.rs`; focused commands `policies_follow_their_declared_order` (1 passed), `equal_overrides_use_oldest_then_directive_id_and_provenance_is_stable` (1 passed), and `rtk cargo test --test resolver_property_tests --features pg18 --offline` (2 passed); E2E output logs cited under R1. | proven |
| R3 | `persist_outputs` computes member counts in one membership aggregation. Exact projections cover the 128-member, one-identity fixture and singleton populations at 128 and 2,048 records. The qualification performed 70 exact refresh/full-rebuild equivalence checks across those populations. No direct aggregation-operation counter is claimed; the reduction is established by source-path inspection and exact public/durable output comparisons. | Candidate `src/api/refresh.rs`; qualification and full-rebuild assertions in `tests/incremental_qualification.sql`; candidate and base E2E evidence cited under R1. | proven |
| R4 | `cannot_cross` returns immediately for an empty cannot-link set before either member iterator. The 256-record test joins two components with a manual bridge and compares the complete `Resolution` to the independent `resolver::oracle::resolve`. The 128-record no-NOT_MATCH refresh asserted the exact public and durable projection. Non-empty automatic prohibition, manual contradiction, and generated-graph oracle checks passed. The result’s `component_checks` field is not treated as a pair-visit counter. | Candidate `src/resolver.rs`; `large_components_with_empty_cannot_links_match_set_oracle` (1 passed), `generated_small_graphs_match_oracle_and_ignore_input_order` (1 passed), `cannot_links_reject_automatic_edges_and_manual_contradictions_fail` (1 passed), and resolver property tests (2 passed); exact fixture assertions in `tests/e2e.sql`. | proven |
| R5 | The reproducible clean baseline was built with Docker cache disabled from the fixed base, and the build asserted the base `src/api/refresh.rs` SHA-256 `df974d1457fa597e727862fb6e28ed8c1a63962cae113a551faaaf6b823973d6`. Paired work observations establish the publication call reduction in R1. Base and candidate exact E2E projections and failure/retry assertions passed against the same fixture; qualification also compared refresh output against full rebuilds. For membership and resolver work, the paired observation is source-path inspection plus exact outputs/oracles, not direct runtime operation counts. No numeric SLO is promised. | [Clean baseline manifest](/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/implementation/baseline/runtime-trace-clean/manifest.json), SHA-256 `353cc288a3870adc92fa2c3dfaac997e0e19ae1de0800d4495fea48fe276fa4c`; clean baseline summary and exact replay log cited under R1; [report correction](/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/implementation/report-correction.md). | proven |

## Unresolved gaps

None. Runtime membership-probe and resolver pair-visit counters were not collected; source inspection, exact output comparisons, and independent oracles establish those work-shape requirements. No numerical timing guarantee is claimed.

## Repairs needed

None.

Fresh `/prove` is required after any repair.

The separate full review is complete for this candidate. Matching review and proof reports complete the acceptance evidence once the enclosing workflow saves and rereads them.

Next steps:

1. No further action is required unless publication is wanted. If so, prepare a draft preview with `/publish-pr <candidate handoff>; review .p2p/work/optimizations-20260907-publication/review.md; proof .p2p/work/optimizations-20260907-publication/proof.md; target main; draft only`.
