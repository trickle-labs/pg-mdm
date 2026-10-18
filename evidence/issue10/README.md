# Issue #10 qualification

Adapter delivery: [pg-mdm PR #14](https://github.com/trickle-labs/pg-mdm/pull/14).
Producer repair: [pg-trickle PR #1171](https://github.com/trickle-labs/pg-trickle/pull/1171).

## Qualified artifacts

| Artifact | Identity |
|---|---|
| Comparison source | `3b55bc92a54156b697e3e7a99f00b8cbfa31651d` |
| Adapter runtime source | `9ce848931c0152db408f993c40cbb67cf3f20f4b` |
| Qualified adapter binary SHA-256 | `46c967d9f0f2c0fa2a2c0fd925e702a42cbc8b8cce80a428bd8abcbbc1310cdf` |
| Baseline adapter binary SHA-256 | `383b4be673b8fb109cf28ca935e2b2a4753a7ef07847af164ce614ed21bf0b46` |
| Admitted producer source | `d8bb8eb5d45c5c9982e173e88914037433a1ee05` |
| Complete producer source archive SHA-256 | `b8d4c59ca3770c305659255e09adde303b4d220228392c415ae363aa7be0a2da` |
| Qualified producer binary SHA-256 | `edb7345ed03dca50066737d421b4643b26f870435a964fb20379c23a77983ff9` |

The complete functional run used source `307cbaea` plus patch SHA-256
`c26f56fad04c6bad043700c2eba4bf7dd785532df2716d57e13055045535f3a1`.
Its runtime source matches `9ce84893`; only subsequent README wording and
generated SQL source-position comments differ. A clean rebuild after adding
package-only Cargo release cleanup reproduced the exact qualified adapter
binary and the archived generated SQL. Later evidence additions do not change
runtime source. The producer PR delivery merge includes newer upstream main;
the admitted backport above remains pinned independently.

## Correctness and recovery

- All 141 adapter host tests, formatting, clippy, boundary checks, archived
  upgrades and generated-SQL comparison passed.
- The complete Docker E2E runner exited 0: main and policy suites; five warm-up
  and 30 measured isolated updates at each of 128 and 2,048 records; full
  publication, identity, provenance, history, explanation and digest rebuild
  comparisons for all 70 updates; concurrency; authorization; atomic failure
  and durable acknowledgement retry; physical backup recovery; logical
  restore/rebind; clone isolation and compiler canary adoption.
- Each isolated update affected exactly one complete record/component, with
  two terminal delta rows, zero lag and no fallback. Additional workloads
  passed complete rebuild equivalence: no-change at 2,048 skipped with zero
  affected work; an isolated update at 8,192 affected one record/component;
  a dense 32-record change at 8,192 affected one component and produced 560
  delta rows. Every extra returned zero lag.
- Producer: 2,610 unit tests and zero-warning lint passed. Its optimized
  admitted artifact passed exact graph 5, legacy compiler-v9 1, public-delta 9,
  keyless 7, duplicate 10 and graph conformance 4 checks.

The original 0.108.3 binary could report DIFFERENTIAL while omitting a split
deletion. The #1166 control failed closed with FULL_INVALIDATION: keyless scan
deltas were missing from old-side snapshots, and reconciliation repaired stale
rows outside bounded capture. Registering those deltas repaired splits;
unmatched changes additionally required distinguishing proven empty applies
from legacy minimum-one graph work signals. The repair preserves those work
signals and all missing-capture/reconciliation invalidation guards. Snapshot
lookups distinguish source relations with reused aliases and preserve bag
multiplicity. `terminal-traces.json.gz` retains actual graph contracts and
terminal observations; `causal-controls.json` records immutable controls.

Bootstrap, rebuild, transitions, invalidations, gaps, resnapshots and uncertain
limits retain safe full recovery. The functional run used default JIT on;
its timing samples must not be compared with the separate JIT-off profile.
In that default profile, elapsed p95 was 32,771 ms at 128 records and
24,950 ms at 2,048; graph refresh p95 was 32,688 ms and 24,734 ms respectively.
The remaining graph cost is substantial. These observations do not establish
that JIT alone caused the difference between profiles.

## Paired measurements

Both sides passed all 70 complete rebuild comparisons and three extras.
Both sides use the same patched producer,
PostgreSQL 18.4 amd64 image on an arm64 Docker host, two CPUs, 4 GiB memory,
JIT off, 4 MiB work_mem, two parallel workers per gather and auto_explain
analysis with node timing off. Each side retains all 70 raw samples and
complete rebuild comparisons plus no-change, larger and dense workloads.

The archived measurement runner labeled its strategy `differential` without
setting the producer GUC. Those historical runs used the default `auto`;
`paired-summary.json` corrects that label and retains the original value.
The recorded node actions and complete rebuild comparisons remain unchanged.
The CI repair now explicitly pins differential execution for qualification
and resets the fixture role afterward; it does not reinterpret these timings
as measurements of the newly pinned profile.
The pinned fixture passed all 70 rebuild comparisons and three extras in a
JIT-off correctness run. A control inherited FULL and failed the old affected
assertion; the pinned fixture passed that same update and full-rebuild oracle.

The table uses only the 30 measured samples per population; percentiles were
independently recomputed from the raw samples. Stage p95 values are not additive.

| Adapter | Records | Elapsed p50 ms | Elapsed p95 ms | Graph p95 ms | Resolution p95 ms | Publication p95 ms |
|---|---:|---:|---:|---:|---:|---:|
| Before | 128 | 769.852 | 840.113 | 775.850 | 6 | 31.550 |
| After | 128 | 803.048 | 871.892 | 830.750 | 4 | 16.100 |
| Before | 2048 | 1431.137 | 1576.306 | 1206.150 | 77 | 259.850 |
| After | 2048 | 1265.969 | 1327.384 | 1196.650 | 21 | 63.550 |

Before, every measured mutation used full resolution with
`delta_full_invalidation`, affecting the entire population. After, every
mutation used affected resolution for one complete record/component, with
two delta rows, zero lag and no fallback. Elapsed p95 increased 3.8% at 128
and decreased 15.8% at 2,048. Graph refresh remains the dominant cost.

| Additional workload | Before elapsed ms | After elapsed ms | Before affected records | After affected records |
|---|---:|---:|---:|---:|
| No change, 2048 | 1169.768 | 1114.105 | 0 | 0 |
| Isolated change, 8192 | 3902.408 | 2588.061 | 8192 | 1 |
| Dense 32-record change, 8192 | 3849.413 | 2858.203 | 8192 | 32 |

Extras are single observations. Both no-change refreshes skipped resolution.
The candidate's dense change affected one component and captured 496 evidence
inserts plus 64 golden delete/insert rows; the baseline invalidated evidence
and captured only the 64 golden rows. Both produced the same complete rebuilt
publication, with zero lag.

Actual installed graph contracts show the baseline repeats candidate
generation in pair statistics and pairs. The candidate's raw stage feeds one
deduplicated set, read by both consumers. Refresh history records 60 executions
of each candidate stage across measured mutations. Pair-statistics p95 fell
from 60.125 ms to 8.063 ms; the added raw/set stages and enriched pair work
remain visible in `paired-summary.json` rather than being omitted from totals.

Auto-explain captured candidate-block scans during initial population and
population growth, separately from measured mutations. At 128 records,
executed block scan operators fell from four across statistics/pairs to two
in the shared raw stage, visiting 512 versus 256 rows. At the 2,048-record
population load, scans fell from 14 to nine, visiting 28,672 versus 18,432
rows. The selected full SQL/plans and window-matching method are retained in
`plan-work.json.gz`. These are population-load observations, not scan counts
for all steady-state samples. Producer delta/reconciliation work can execute
multiple scans within one graph stage.

Whole-run database counters include population loads, all warmups, 70 full
rebuild oracles and three extras. Buffer hits fell from 39,965,598 to
32,715,913; tuples returned fell from 45,490,374 to 35,360,779. Temporary bytes
increased from 218,234,880 to 240,750,592 and blocks read increased from
10,825 to 12,579. These counters are not per-refresh memory or I/O bounds.

An initial setup failure and a stale compiled baseline attempt are excluded.
The accepted run asserts both exact binary checksums and graph shapes before
timing. Source mtimes can precede a shared Cargo target cache after switching
branches; the E2E builder now cleans only the adapter release package before
compiling. The complete baseline source tree was checked against the fixed
comparison commit.

No production latency SLO is claimed. Source synchronization and global
prior-state loading remain costs outside this child. Long upstream soak and
longevity runs were not repeated. GitHub CI status is reported on the PRs.

## Retained evidence

Compressed JSON files retain their full content and can be read with Python's
standard `gzip` and `json` modules. `manifest.sha256` binds the retained files.
`paired-before.json.gz` and `paired-after.json.gz` retain complete results,
contracts and refresh histories; `paired-summary.json` contains independently
checked percentiles, stage executions and resource metadata. Compressed SQL
inputs and the measurement/extraction scripts document the exact local method.
The local manual proof additionally retains complete runner/PostgreSQL logs,
source archive, producer package and package manifest. It does not alter the
historical P2P controller or acceptance-contract records.
