# Repaired optimization acceptance

Verdict: **PASS for the repaired assembled runtime.** F1 and G1 are resolved. Parent #8 can close after this repair is merged and its required checks pass.

Subject runtime: `2523414d2c1e57cc8c0b73a8017de6f5941d21e7`. This candidate contains all four deliveries for [parent #8](https://github.com/trickle-labs/pg-mdm/issues/8) and the publication repair. The contract remains v1, SHA-256 `f09a262000f5c2615fce31399ea3548720c56a0a5b2619bb5f379b885463f1f5`; the [earlier negative review](../acceptance-review.md) remains retained for its original candidate.

This is an author-context self-check using independent expected rows, resolver/candidate oracles, raw durable snapshots and actual SQL plans. It does not claim independent human approval or legacy P2P/controller admission.

## Corrections

F1 is repaired in the shared publication path. It reads the complete entity's published review flag through SQL `EXISTS`, after scoped review persistence. Affected publication synchronizes the flag on other identities and advances their `last_change_revision` only if that flag changes. No complete review collection is decoded for this operation. The flag query and set-based output scan remain global work.

The new `tests/scoped_state.sql` checks fail on the unfixed binary at reactivation and pass on the repaired binary. They check all historical-case flags against the complete review set, compare entity/member/review publications from the same original ledger and source state using a savepoint, and assert fixed expected rows and review occurrence/revision/version tuples while opening the first review, updating an unrelated component, resolving the last review and doing nothing. Entity rows' path-specific `last_change_revision` is excluded from the cross-strategy comparison and separately asserted in the fixed transition cases.

G1 is replaced by fresh paired traces, not by relabeling the missing historical files. The baseline adapter was rebuilt from exact pre-S1 source `9a07830d062b31c108880071c27124e2f09c446a`; all 188 restored Git blobs matched. Both sides use PostgreSQL 18.4, Rust 1.98.0 and the identical admitted producer binary `edb7345ed03dca50066737d421b4643b26f870435a964fb20379c23a77983ff9`. The baseline adapter binary is `a75fd36be39c02133dfb5b62df358478fe6636fcaba44402a5d6ba33c4c2d8f3`; the repaired binary is `ebdbd8a8c290e2a3bc934a4ba2f5c637cbba0baa1127d049f56603075598f59c`.

The common `publication-probe.sql` compares complete registry, membership, golden-provenance, public entity and public member rows with fixed fixture expectations at both populations. Every selected SPI INSERT plan is retained; the extractor asserts the complete per-target sequence of input-row volumes, not only a summed call count.

| Records | Before calls per target | After calls per target | Before total | After total |
|---:|---:|---:|---:|---:|
| 128 | 128 | 1 | 640 | 5 |
| 2048 | 2048 | 16 | 10240 | 80 |

The five targets are registry, memberships, golden provenance, entity output and member output. Each baseline call receives one row; each repaired call receives 128. These are full singleton bootstrap observations, using one golden field and unique email keys; they are not affected-refresh latency measurements. Review batching is covered by current source inspection and exact non-empty/empty review lifecycle fixtures.

## Parent requirements

Every row below is supported by the completed fresh run, current source review and retained independent observations on this assembled runtime. Prior child verdicts alone are not used as proof.

| ID | Evidence and assessment |
|---|---|
| R1 | Retained before/after observations in issue #10–#12 plus the replacement S1 paired trace. The fresh runner records all required source/graph/loading/assembly/digest/copy/publication stages, 5 warmups and 30 measurements at both populations, and larger/no-change/dense workloads. Resource and timing limitations are stated below. |
| R2 | The original terminal-invalidation causal controls and immutable producer mappings remain in `evidence/issue10`. The actual repaired producer is present in this candidate and re-exercised through the production graph. |
| R3 | The fresh isolated mutation/rebuild comparisons require one affected record/component, two terminal rows, no fallback and zero lag. Mutation and recovery fixtures retain complete component/publication expectations and safe full recovery. |
| R4 | The replacement actual SPI traces and exact five-target fixtures verify bounded write batches at 128/2048. Empty/partial/review batches, typed/quoted output and rollback cases run through the production suite. |
| R5 | Current golden selection builds one active-membership lookup and retrieves sort keys through it. Exact policy/tie/override/provenance tests and resolver properties pass. No direct runtime probe counter is claimed. |
| R6 | One member-count aggregation remains. The new exact public rows prove consistent review flags and correct revisions for unrelated components across first/last-review transitions and no-op refresh. |
| R7 | Source lifecycle and work-sequence assertions, changed old/new keys, unsupported-source fallback, global limits and real source/writer fences exercise the assembled source-capture path. Issue #11's paired source plans remain retrievable. |
| R8 | Scoped decoding retains closed historical/control relationships, inactive anchors, remote override dependencies, recurrence and retirement. Fresh raw snapshots independently reconstruct all canonical digests and verify complete public flags. Global assembly/digest/history work stays separate. |
| R9 | Current compiled pairs and exact statistics consume one shared deduplicated candidate set. Exact compiler fixtures, independent all-channel candidate oracles and the dense production workload verify completeness and overflow handling. |
| R10 | The empty cannot-link path returns before member crossing. Complete results match the set resolver for large empty-constraint components, generated graphs, non-empty prohibitions and contradictions. |
| R11 | The fresh production suite and independent digest checks preserve durable/public/current/historical state. The new same-original-boundary comparison closes the derived-output blind spot in the former no-op rebuild oracle. |
| R12 | Complete failure-state/ack comparisons, retry, logical/physical recovery, restore/rebind, clone isolation and later-source-write cases exercise the repaired transaction, including the new output update. |
| R13 | Global count/block checks remain before publication. Candidate, comparator and resolver limits plus production overflows verify exact failures and unchanged complete publication. |
| R14 | Authorization and entity/source fences are unchanged. Current denied-role/source tests, real concurrent-session schedules, helper boundary checks and generated-schema verification cover the repaired path. |

## Candidate binding and checks

The qualified image and full runner use the clean runtime commit above with an empty source patch. The Linux SDK source, regression and lockfile hashes match that candidate. Subsequent retention changes add evidence/documentation and reconcile five source-position comments in the archived SQL to the actual already-qualified generated package. Executable SQL is identical; no other runtime, fixture, contract or dependency change is admitted by the verification script.

This evidence pack retains the full-run envelope/log, raw current history cases, unit/SDK logs, both package manifests, every selected paired SPI plan, source/runtime bindings and checksums. `verify-repaired-evidence.py` checks these identities and exact sample/work sequences; the matrix above records the source review as well.

Fresh checks passed: 142 host unit/property tests, 142 pinned Linux unit/property tests, the PostgreSQL/pgrx runner (142 tests), formatting, zero-warning clippy, security boundaries, all 15 archived upgrade paths, generated-SQL equality and the complete production E2E runner. The new regression fails on the unfixed binary and passes on the repaired one. All 70 refresh/full-rebuild comparisons, all 11 independent canonical digests and every exact public review flag pass; larger, dense, no-change, concurrency, failure/retry, physical/logical recovery, restore/rebind and clone cases pass. The evidence verification script also passes.

The source-archive comparison initially caught five manually mis-offset debug comments. Commit `804273502586d09f23654b943ffa739393a416b5` reconciles them to the actual qualified package; security/upgrade checks and exact comparison then passed. Executable SQL and the qualified library are unchanged. The earlier negative report remains intact.

## Reproduce

Run from a clean checkout of this repair, whose runtime/fixture files match the qualified runtime. Retained local images are `pg_mdm:issue8-baseline`, `pg_mdm:issue8-fixed` and `pg_mdm:issue8-sdk`. The original base is restored with `git archive 9a07830d062b31c108880071c27124e2f09c446a`; build its adapter with the current pinned builder recipe, then layer that package using `baseline-runtime.Dockerfile`. The images hold the installed packages; package manifests and source/runtime bindings are retained here.

```sh
rtk proxy env PG_MDM_E2E_IMAGE=pg_mdm:issue8-repeat PG_MDM_E2E_EVIDENCE_DIR=/tmp/issue8-evidence scripts/run_e2e_tests.sh
rtk proxy python3 -B evidence/issue8/repaired/verify-repaired-evidence.py
```

For the paired probe, create isolated containers from the baseline and repaired images, mount this directory as `/work` and `evidence/issue8` as `/review`, wait for database readiness and execute `/review/bootstrap.sql`. For each population, run `/work/publication-probe.sql` with psql variables `population=128` or `2048`, `entity_name=publication_probe_N`, `source_table=publication_source_N` and `source_relation=public.publication_source_N`. Capture raw `rtk proxy docker logs` plus each probe log, then run `extract-publication-traces.py baseline` and `extract-publication-traces.py candidate` from the collected directory. The extractor reads `SIDE-server.log` and `SIDE-probe-N.log` and retains all main-target plans, including original SQL, bound parameters, PID, timestamp and complete plan text. The original build and runtime-layer logs are retained.

## Environment and limits

Both replacement trace containers run amd64 Linux on an aarch64 Docker host, with ten visible processors and an 8,319,504,384-byte VM. Containers have no separate CPU/memory cap. JIT is disabled for the paired diagnostic fixture by its database setting; auto_explain analyzes nested statements with node timing off. The same fixed timestamps and source data are used on both sides. Actual fixture rows and every plan are retained with source/package identities and commands.

The full runner uses its existing qualification profile. A manual `CHECKPOINT` completed a paced checkpoint during the isolated physical-backup test after qualification; logical source/publication state and the recovery assertions were unchanged. Background SDK compilation and unrelated host activity overlap portions of this run. Latency differences across these collections are not causal speedup/regression estimates; no production SLO or end-to-end delta-proportional claim is made. Previous measurements and their comparability limitations remain in the child evidence. Global graph/helper/count, review-flag scan, assembly, digest and history-copy costs remain.

Existing inconsistent current flags are normalized by the next changed publication; no-op refresh continues to leave output rows untouched.

The original missing S1 traces have not been recovered; the new independently checked pair replaces that observation path. No upstream soak, production capacity test, historical-storage rewrite or new dependency was introduced. No merge or issue closure is performed by this evidence collection.
