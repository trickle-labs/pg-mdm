Scoped prior-state loading evidence for [issue #12](https://github.com/trickle-labs/pg-mdm/issues/12), S4 of parent #8 (R8; state/history R1 and applicable R11–R14).

| Input | Identity |
|---|---|
| Tested runtime source | `8f362be3080cc643a41670badfef29fcbea9c119` |
| Baseline image source | `ad6534b0924aae1ec6d5ec51b4907ce1bd9f3dd9` |
| Baseline adapter binary SHA-256 | `fe37b98b79c77d64cebabb97e3fc537a513ca71a2014bb995dca18ca41dd9b38` |
| Candidate adapter binary SHA-256 | `7a5bba5ab269af0d9fb52f1cf518ab2f46694106a1a09ddb15173b4c0c7f941e` |
| Producer source | `d8bb8eb5d45c5c9982e173e88914037433a1ee05` |
| Producer binary SHA-256 | `edb7345ed03dca50066737d421b4643b26f870435a964fb20379c23a77983ff9` |

The baseline's runtime/build/test trees match main base `bdca799`; the intervening changes contain retained evidence. Both runs use the checksum-pinned repaired producer in `DEPENDENCIES.md`. The candidate's actual EXACT ranges, affected execution and safe full recovery were requalified, rather than inferred from issue #10's status.

The complete fresh E2E runner passed on the tested binary. Its host-side collector was repaired during recovery testing: the 2048-record diagnostic session has tracing disabled, so there are 71 traced isolated calls (36 at 128, 35 at 2048), alongside all 70 sample/rebuild comparisons. The patch also asserts exact first-membership history, review occurrence/revision/version tuples and the retained retirement tombstone. `collector.patch`, its SHA-256 and the executing collector SHA-256 are retained in the envelope. The collector permits this separately recorded patch only when it is the sole changed file; runtime, SQL and fixture changes still require the existing candidate identity checks. The subsequent retention commit adds collection code and evidence without changing Rust, SQL or fixtures.

The baseline collection ran the unchanged image's `tests/e2e.sql`, then `tests/incremental_qualification.sql` in `foundation`, with `producer_artifact_sha256=b8d4c59ca3770c305659255e09adde303b4d220228392c415ae363aa7be0a2da`. The candidate collection used `scripts/run_e2e_tests.sh` with the evidence directory and PostgreSQL-log capture enabled. Collection locations in the envelope describe the original ignored local storage; this directory retains the corresponding artifacts.

Each population uses five warmups and 30 measured single-record updates. Every sample reports one affected record/component, two terminal delta rows, zero lag and no fallback, and compares complete output, durable state, digest and explanations with a full rebuild. Resolver properties separately compare affected splicing with full evaluation from the same original ledger through generated edge, removal/reactivation and golden-value histories. Eleven production cases exercise exact components, aliases/splits, control-only changes, inactive/remote overrides, recurring reviews, retirement/reactivation and current/historical explanations. An independent Python oracle reconstructs canonical publication digests from the captured raw durable rows, including Unicode and escaping.

Actual SPI plans in `before-state-plans.json.gz` and `after-state-plans.json.gz` show the registry, membership and prior-golden readers returning 128/2048 rows before, and one row each for isolated affected updates after. The candidate also reads one membership ID for closure. The 71 traced isolated calls match the complete expected decoded-row dictionary: one registry row, membership and golden row; one closure ID; zero aliases, splits, reviews, facts and overrides. Historical aliases/splits remain in complete publication assembly. The history fixture separately observes scoped resolved reviews, inactive anchors and one remote override representative per field. Override edits can legitimately require an entity-wide scope because inactive-anchor review semantics have global dependencies.

| Candidate stage, p50 / p95 (ms) | 128 records | 2,048 records |
|---|---:|---:|
| Evaluation state loading | 2.571 / 3.326 | 5.913 / 7.048 |
| Complete merged-state construction | 8.876 / 9.605 | 117.100 / 125.906 |
| Digest generation | 0.500 / 0.560 | 9.557 / 10.949 |
| Snapshot copying/publication | 3.327 / 4.382 | 36.512 / 40.143 |
| Paired full-evaluation state loading | 2.760 / 3.801 | 25.165 / 29.266 |

The paired full loader is the same-boundary rebuild, not an instrumented pre-change timing. Loading includes current source/evidence inputs as well as prior state; copying includes new snapshot rows and retained rows. Stage percentiles are independent and must not be added. Raw samples retain every stage separately.

| Population | Before elapsed p50 / p95 (ms) | After elapsed p50 / p95 (ms) |
|---|---:|---:|
| 128 | 21095.348 / 22965.377 | 21287.727 / 23210.786 |
| 2,048 | 22552.226 / 24704.300 | 23959.599 / 26613.601 |

No end-to-end speedup was observed. Complete semantic assembly, digest generation, snapshot copying, graph/helper synchronization and entity-wide counts retain global work. Background builds overlapped portions of the baseline; host load, caches and emulation were uncontrolled, and candidate tracing adds observations. These elapsed differences are not a causal speedup/regression estimate or a production SLO.

The additional cases also match full rebuilds: no change at 2048 skips evaluation; an isolated update at 8192 affects one record; a dense block of 32 equal emails at 8192 affects one component, with exactly 496 pairs and 560 terminal delta rows. The fixture uses default limits (100,000 active/component records, 1,000,000 candidate/automatic edges and component checks, 10,000 block records). The environment is PostgreSQL 18.4, Rust 1.98.0, amd64 Linux emulated on an arm64 host, ten visible processors and an approximately 8 GiB VM. `measurement-summary.json`, `identities.json` and the envelope retain environment, package and workload identities.

Host and pinned Linux Rust suites passed (142 tests each); the Linux pgrx test runner, formatting, clippy, security checks, all 15 upgrade paths and generated-schema comparison passed. Fresh production E2E additionally passed denied-role/source access, entity-wide limits, injected rollback/retry with exact state/ack checks, concurrency/fences, logical and physical recovery, restore/rebind and clone isolation. `after-runner.log.gz` retains the completed runner's pass lines; `after-e2e.log.gz` retains database assertions and work traces.

To repeat the candidate and extract its actual prior-state plans from a clean checkout:

```sh
rtk proxy env PG_MDM_E2E_IMAGE=pg_mdm:issue12-repeat PG_MDM_E2E_EVIDENCE_DIR=/tmp/issue12-evidence PG_MDM_E2E_LOG_CAPTURE=/tmp/issue12-postgres.log scripts/run_e2e_tests.sh
rtk proxy python3 -B evidence/issue12/extract-state-plans.py after /tmp/issue12-postgres.log /tmp/issue12-evidence/docker-e2e.log /tmp/issue12-plans.json.gz
```

The extractor's `before` mode accepts the baseline server and qualification logs. Package manifests and `manifest.sha256` bind the retained bytes. Historical storage was not rewritten. This is S4 implementation/runtime evidence; it does not assert legacy P2P approval/controller admission or final assembled acceptance of parent #8.
