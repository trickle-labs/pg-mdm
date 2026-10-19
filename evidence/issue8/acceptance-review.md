# Issue #8 final acceptance review

Verdict: **CHANGES NEEDED — keep #8 open.** All seven implementation areas are present, but a same-boundary comparison disproves public-output equivalence. The original publication baseline also has a retention gap.

Subject: merged main `d0c512ecac3d9393b1c199a405ec538ecd8f1eec`, [issue #8](https://github.com/trickle-labs/pg-mdm/issues/8), contract v1 SHA-256 `f09a262000f5c2615fce31399ea3548720c56a0a5b2619bb5f379b885463f1f5`. [Frozen contract](contract.md) and [original source](source-snapshot.md) are retained byte-for-byte. Source SHA-256: `2ee03b51093184affdcd41d4041e6f6298060573418ae20435431c4d1e46ef23`.

This is an author-context self-check with independent fixture expectations and a fresh database experiment. It does not claim an independent human review, authenticated reviewer identity, or legacy P2P approval/controller admission. No requirement or exclusion was changed. Closing children was not used as an acceptance oracle.

## Candidate and evidence binding

- The merged candidate and clean tested PR commit `ff570616e8f86c7ebc3c81dfb21f8ea83b57b38e` have the identical complete Git tree; the tree hash is recorded in [evidence-checks.json](evidence-checks.json).
- [Successful CI run 39152559274](https://github.com/trickle-labs/pg-mdm/actions/runs/39152559274) ran that clean tree. Its [retained envelope](ci-pr/e2e-evidence/docker-e2e-envelope.json.gz) reports no working-tree changes, an empty candidate patch, and a passed complete runner. Its [runner log](ci-e2e-runner.log.gz) is retained here. The merged commit's separate CI run was still in progress when reviewed; it is not counted as passed.
- Runtime, SQL, fixtures, Cargo inputs and control/configuration paths match the separately qualified runtime `8f362be3080cc643a41670badfef29fcbea9c119`. The fresh reproduction used its retained image `pg_mdm:issue12-final`, with installed adapter SHA-256 `7a5bba5ab269af0d9fb52f1cf518ab2f46694106a1a09ddb15173b4c0c7f941e` and producer SHA-256 `edb7345ed03dca50066737d421b4643b26f870435a964fb20379c23a77983ff9`.
- Producer source is the qualified, pinned `d8bb8eb5d45c5c9982e173e88914037433a1ee05`, not an inference from a release name or merged upstream PR.
- [check-evidence.py](check-evidence.py) rechecked every entry in the issue #10/#11/#12 manifests, gzip integrity, exact clean-CI fixture hashes, all 70 sample identities, all 71 isolated scoped traces, 11 independent canonical digests, and the issue #12 elapsed/stage percentiles from raw samples. It also identifies three retained examples of the publication mismatch below.

## Requirement matrix

“Supported” records evidence for the row's primary outcome on the assembled implementation. It does not waive the shared public-output failure in F1.

| ID | Verdict | Observation and evidence |
|---|---|---|
| R1 | Incomplete retention | Raw before/after samples at 128/2048, five warmups and 30 measurements each, plus larger/no-change/dense cases are retained in issue #10–#12. Current clean CI records all required stage keys. State-stage percentiles were independently recomputed. Comparability limits are explicit. The original S1 publication baseline SPI traces are unavailable: G1. |
| R2 | Supported | Issue #10's causal controls distinguish omitted split deletion, keyless snapshot registration, reconciliation invalidation and conservative graph-work signals. Exact dependency identities and terminal observations are retained in `evidence/issue10/causal-controls.json`, the corresponding control logs and `terminal-traces.json.gz`. The assembled candidate pins and exercises the repaired producer. |
| R3 | Supported primary outcome | The clean assembled CI envelope contains all 70 qualified samples and larger/no-change/dense results. Isolated updates use one affected record/component, two terminal rows, no fallback and zero lag. Existing production mutation/recovery assertions cover connect/split, insert/delete, invalidation, gaps, transitions and resnapshot. Publication parity has the counterexample in F1. |
| R4 | Implemented; retention incomplete | Current `refresh.rs` batches registry, membership, golden and entity/member/review writes in chunks of 128, retaining conditional updates and typed arguments. [Actual assembled-runtime SPI plans](publication-plans.json.gz) show one INSERT into each of the five main targets for the 128-member fixture, whose complete rows are asserted by `tests/e2e.sql`. Before-S1 source `9a07830` has per-row loops. The claimed historical paired runtime baseline cannot be independently checked here: G1. |
| R5 | Supported | `evaluation::select_golden` builds one active membership map and reads both membership and sort key from it. Fresh tests include exact policy order, overrides/ties/provenance and generated histories. This replaces the baseline's repeated membership scan; no runtime probe counter is claimed. |
| R6 | Failed | `persist_outputs` computes member counts in one aggregation and the exact singleton/multi-member fixtures pass. Its `has_review` value disagrees between affected and full publication from the same boundary: F1. |
| R7 | Supported primary outcome | `grant_refresh_access`, `capture_source_input` and `sync_source_records` preserve privilege preflights, fenced complete capture, changed old/new keys and full-scan recovery. The clean assembled runner executes the exact lifecycle work sequence, source/registry snapshots, cross-source overflow, partition fallback, pending-writer fence and same-entity serialization checks. Issue #11 retains selected before/after SQL plans. |
| R8 | Supported primary outcome | Current SQL loaders use closed record/identity scopes, with historical membership/control expansion and safe full fallback. All 71 isolated traces match the exact expected dictionary: one registry, membership and golden row, one closure ID, and zero other decoded prior rows. Historical/control fixture assertions and independent canonical digest checks pass. Global assembly/digest/copy costs remain separately visible. F1 affects the derived public flag. |
| R9 | Supported | `graph_spec::compile` builds one raw stage and deduplicated candidate-set stage; pairs and exact statistics consume that set. Fresh candidate/compiler fixtures and independent all-pairs properties pass across channel kinds/overlap. The dense production case contains the expected 496 candidate pairs and 560 terminal rows; issue #10 retains actual shared-stage graph/plan observations. |
| R10 | Supported | `resolver::cannot_cross` returns before either iterator when the set is empty. The fresh 256-record component test compares the complete result to the independent set resolver; generated graphs and non-empty prohibition/manual contradiction tests pass. No pair-visit counter is inferred from `component_checks`. |
| R11 | Failed | Canonical state/digests, identity history, explanations and generated semantic projections have substantial passing evidence. Complete public equivalence nevertheless fails in the independent same-boundary branch experiment: F1. The existing post-refresh rebuild oracle cannot detect this derived-output difference. |
| R12 | Supported | Current publication/ledger/history writes and both source/terminal acknowledgements remain inside the transaction. The clean assembled runner passes exact failure-state/ack comparisons, retry, delta recovery, logical/physical restore, clone isolation and later-source-write checks. |
| R13 | Supported | Current code retains entity-wide active/candidate counts, block overflow validation and full fallback when affected/global limits are uncertain. Fresh candidate/resolver/comparator limit tests and the assembled E2E overflow/unchanged-publication probes pass. |
| R14 | Supported | Authorization and entity/source fences remain before publication. The assembled runner passes denied-role/source access and real concurrent-writer/refresh schedules. A fresh `scripts/check_security_definer.sh` run passed. |

## F1 — affected publication gives a different public review flag

Severity: high. Requirements: R6 and R11, with the shared publication-equivalence constraint applying to R3/R8.

From the same original ledger and source boundary, reactivating record 1 gives these complete `(name, member_count, has_review)` projections:

| Row | Affected refresh | Full rebuild |
|---|---|---|
| Record 3 | `["Record 3",2,true]` | `["Record 3",2,true]` |
| Record 5 | `["Record 5",1,true]` | `["Record 5",1,true]` |
| Second | `["Second",2,false]` | `["Second",2,true]` |

The [exact result](reproduced-comparison.json), [failing log](review-flag-branches.log.gz), [SQL reproduction](review-flag-branches.sql) and [exit statuses](reproduction-status.json) are retained. Setup and all existing scoped-state assertions exit 0; the new independent comparison exits 3 with `R6/R11: same-boundary affected and full public review flags differ`.

The experiment uses a transaction/savepoint: insert and affected-refresh, capture the public projection, roll back to the original boundary, repeat the identical insert with full rebuild, compare, then roll back. The input timestamp is fixed. It does not use the affected result as the full branch's starting ledger. The same false flag is already present in the retained `reactivation`, `retirement` and `retired_reactivation` cases from issue #12.

Root cause: `src/api/refresh.rs:2730` counts only the reviews passed into `persist_outputs`. Affected evaluation supplies scoped reviews (`:4507`, `:3282`), while full evaluation supplies the complete set. The scoped component can therefore publish false while the full path publishes true. The scoped publication also leaves unrelated entity rows untouched when the entity-wide review flag changes. This pattern predates S4's loader rewrite; the review does not attribute its introduction solely to PR #16.

Why green tests missed it: `tests/scoped_state.sql:81` and `tests/incremental_qualification.sql:246` rebuild **after** affected publication and expect `changed = false`. A full rebuild whose semantic projection is unchanged skips `persist_outputs`. Comparing public tables before and after that no-op preserves the erroneous flag. The canonical digest includes semantic reviews, not this derived public column, so digest agreement also cannot detect it.

Smallest correction: make affected publication preserve the complete published review flag and update every output row whose flag must change, preserving the established revision rules. Add the same-original-boundary public-output comparison to the production suite, including an unrelated component with an open review and a transition that opens/resolves the last review. The test must fail on this candidate before the correction and compare exact expected rows afterward.

## G1 — original paired publication trace cannot be retrieved

Requirements: R1/R4. This is an evidence gap, not proof that batching is broken.

The retained [S1 historical proof](s1-historical-proof.md) claims five-target totals of 640→5 calls at 128 singleton records and 10,240→80 at 2048. It cites raw baseline and candidate traces under `/Users/grove/.p2p/executions/pg-mdm2-8c61405403627466/optimizations-20260907-publication/implementation/`. That directory is absent here, and the raw traces are not in the tracked evidence directories. The historical report cannot substitute for inspecting those observations.

The current assembled implementation's five INSERT plans are now retained, and the original source's per-row call structure was inspected at `9a07830d062b31c108880071c27124e2f09c446a`. Those establish the implementation/work shape. They do not recreate the missing measured baseline. Recover and verify the original trace artifacts, or retain a replacement paired diagnostic trace bound to the original base and repaired assembled candidate. Existing latency samples need not all be repeated solely for this gap.

## Checks and reproduction

- Fresh host unit/integration/property suites: **142 passed, 0 failed**; [raw log](unit-tests.log.gz).
- Fresh boundary checker: **passed**.
- Fresh isolated database: all original scoped-state assertions **passed**; [fixture log](fixture-assertions.log.gz). Cleanup was deliberately deferred to retain the fixture for the independent comparison.
- Fresh same-boundary public-output assertion: **failed as recorded above**.
- Clean identical-tree CI and retained evidence were inspected and rebound; a second full local E2E run was not repeated for this review.

Run from the repository root at the subject commit. The reproduction uses the retained image identified above; rebuilding that exact runtime with the pinned dependencies is also valid.

```sh
rtk proxy python3 -B evidence/issue8/check-evidence.py
rtk proxy python3 -B evidence/issue8/prepare-reproduction.py
rtk docker run --detach --name pg-mdm-issue8-repro -e POSTGRES_PASSWORD=postgres -v "$PWD/evidence/issue8:/review:ro" pg_mdm:issue12-final
rtk docker exec pg-mdm-issue8-repro pg_isready -U postgres
rtk docker exec pg-mdm-issue8-repro psql -X -v ON_ERROR_STOP=1 -U postgres -f /review/bootstrap.sql
rtk docker exec pg-mdm-issue8-repro psql -X -v ON_ERROR_STOP=1 -U postgres -f /review/fixture-prefix.sql
rtk docker exec pg-mdm-issue8-repro psql -X -v ON_ERROR_STOP=1 -U postgres -f /review/review-flag-branches.sql
rtk docker rm -fv pg-mdm-issue8-repro
```

Wait for `pg_isready` to succeed before the SQL commands. The branch-comparison command is expected to fail on this candidate. The generated fixture prefix contains every original assertion and omits only final fixture cleanup.

Performance observations remain synthetic and environmentally limited. Issue #12 shows no end-to-end speedup; #8 promises no numerical SLO, so that observation alone does not fail acceptance. Global graph/helper/count, assembly, digest and history-copy work remains. No historical-storage rewrite, upstream soak, production capacity claim, runtime repair, tracker change or closure was performed by this review.

Close #8 only after F1 is repaired and its exact regression passes on the repaired assembled candidate, G1 is resolved, and the R1–R14 verdicts are refreshed against that candidate. This negative review remains evidence for the current subject; a later successful review must identify its new subject and the corrections.
