Source synchronization evidence for [issue #11](https://github.com/trickle-labs/pg-mdm/issues/11) and [PR #15](https://github.com/trickle-labs/pg-mdm/pull/15).

| Artifact | Identity |
|---|---|
| Baseline source | `fe5704c652f507859af4ff1c01a28250cf6d6efe` |
| Measured candidate source | `4c2862a1280467eb6c5c18bb6409a9009f5d6581` |
| Final functional source | `ad6534b0924aae1ec6d5ec51b4907ce1bd9f3dd9` |
| Baseline adapter binary SHA-256 | `46c967d9f0f2c0fa2a2c0fd925e702a42cbc8b8cce80a428bd8abcbbc1310cdf` |
| Candidate adapter binary SHA-256 | `fe37b98b79c77d64cebabb97e3fc537a513ca71a2014bb995dca18ca41dd9b38` |
| Producer source | `d8bb8eb5d45c5c9982e173e88914037433a1ee05` |
| Producer binary SHA-256 | `edb7345ed03dca50066737d421b4643b26f870435a964fb20379c23a77983ff9` |

The final functional source changes only test assertions and trigger coordination after the measured candidate. Its rebuilt adapter binary matched the measured checksum before the retained measurements were reused. The fresh functional run executes installation, authorization, policy, source lifecycle, concurrency, recovery, restore and clone assertions; its driver is retained. The measured run separately completed all 70 full-rebuild comparisons and three additional workloads. The envelopes record both collection contexts explicitly.

The source fixture compares complete registry/active state and member keys with fixed lifecycle expectations and an independent complete source-key snapshot. It covers composite old/new keys, soft deletion/reactivation, matching continuity, cross-source limit overflow, publication rollback/retry, denied access, resnapshot, gaps, full invalidation and partitioned-source fallback. Sibling recovery checks exact pending DELETE/INSERT keys as well as both batch descriptors. The complete 14-step work sequence matched the fixed fixture. Two-session checks verify the pending writer blocks capture, registration takes a fresh snapshot after the fence, later writes remain pending and same-entity operations serialize.

Selected complete SQL plans are in `before-source-plans.json.gz` and `after-source-plans.json.gz`; `source-plan-work.json` records the exact key volumes. The baseline trace covers bootstrap at 128 records and growth to 2,048, enumerating 128/2,048 keys. The baseline code runs that enumeration unconditionally. Candidate tracing additionally covers isolated updates and no-change calls outside the timed samples: six privilege preflights return zero rows, bootstrap reads 128 keys, and synchronization receives exactly 128, 1, 1,920 and 1 keys. Neither no-change call executes source reconciliation SQL. Population growth passes the 1,920 inserted keys without enumerating the complete 2,048-key source.

All 70 candidate timed updates synchronize one key each. Their administrative rebuild oracles deliberately scan all 128/2,048 keys. The additional no-change refresh reads zero keys; isolated and dense changes at 8,192 synchronize 1 and 32 keys. The envelopes retain the complete observed work sequence, capture boundary/consumer metadata and every timing sample. These are source-input observations, separate from terminal-output EXACT qualification. Graph helper synchronization and entity-wide active counts still perform global work.

Both populations use five warmups followed by 30 measured isolated updates. Every sample compares complete publications, durable identities, provenance, digest and explanations with a full rebuild at the same source/ledger boundary. Each update affects one record/component, emits two terminal delta rows, reports zero lag and has no resolver fallback. Additional no-change, isolated 8,192-record and dense 32-record cases also match full rebuilds; the dense case affects one component and emits 560 terminal rows.

| Population | Baseline elapsed p50/p95 (ms) | Candidate elapsed p50/p95 (ms) |
|---|---:|---:|
| 128 | 21615.713 / 23226.232 | 21155.965 / 21340.936 |
| 2,048 | 23411.842 / 24600.507 | 21940.397 / 22141.370 |

| Candidate stage p95 (ms) | 128 records | 2,048 records |
|---|---:|---:|
| Source access | 6.55 | 7.00 |
| Source capture | 102.75 | 109.10 |
| Source scan | 0 | 0 |
| Source synchronization and active count | 0 | 0 |
| Graph helper synchronization | 0 | 4.00 |
| Request through result construction | 21337.60 | 22133.10 |

Stage clocks use integer milliseconds; zero means below that resolution. Capture includes graph validation, definition helper writes, source refresh and consumer reads. The helper clock includes definition and registration helper writes and overlaps capture/graph clocks. Request timing excludes final audit completion and result marshaling; SQL elapsed includes them. Do not add stage percentiles. Baseline stage clocks were not instrumented: retained bootstrap SQL plans separately measure source scans (0.374/2.733 ms), reconciliation (8.013/119.516 ms) and helper SQL, rather than claiming comparable stage percentiles.

Both runs use PostgreSQL 18.4, Rust 1.98.0, the same pinned producer and the same synthetic exact-email workload on an amd64 Docker image emulated on an arm64 host, with 10 visible processors and an approximately 8 GiB Docker VM. Host activity, cache state and emulation were not controlled. The elapsed differences are observations, not a causal speedup claim or a production latency SLO. The reports retain container/image identity, resource observations, dependencies, inputs and full raw samples; whole-run counters include setup, rebuilds, recovery and diagnostic work.

Rust library/integration/property/fixture tests, PostgreSQL integration tests, formatting, clippy, security-definer checks, all 15 archived upgrade paths and generated SQL comparison passed. The locked Linux SDK log is retained. Earlier attempts stopped at a trigger marker because AUTO selected FULL (which suppresses row triggers), and a debug continuation changed hard-coded restore revisions. Those attempts are excluded from the fresh functional result. The corrected trigger case requests differential execution and the final run starts from a fresh database.

JSON/log files are gzip-compressed; package manifests and `manifest.sha256` bind their bytes. Original local paths in envelopes describe collection locations; this directory contains the retained copies. No upstream repository was changed. GitHub CI status is reported separately on the PR.
