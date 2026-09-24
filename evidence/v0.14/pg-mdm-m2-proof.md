# pg-mdm M2 acceptance proof

**Verdict:** R1–R14 pass for the fixed v0.14.0 candidate.

## Candidate and evidence

| Item | Evidence |
| --- | --- |
| Source commit | `4889ac41485ac1282aa90b261487ba5315501d10` |
| E2E source identity | `evidence/v0.14/docker-e2e-envelope.json` records the same source commit |
| Runtime | PostgreSQL 18.4, `pg_trickle` 0.108.0, Rust 1.98.0 |
| Archived install SQL SHA-256 | `ded142f6b0e1e82c046a313cda2d992960bca4b6a10d71a4a75a77c789217724` |
| Package manifest SHA-256 | `c5471c8b137efbb27ce980559d2dc6af3e6f21570cf2c9968795ffd63023e531` |
| E2E inputs | Policy SQL `2187628aeb64fb176d11b970f4d534206a67d593190bf2cc027c3cf1966ed97d`; restore SQL `0413359762146970f9e3dab66fad6f514b3e7b61901ef62a535542bc79997ebd` |

The clean-candidate Docker run exited successfully. Its nine PASS markers are retained in [docker-e2e-summary.txt](docker-e2e-summary.txt); the complete run envelope, database log, and installed-package manifest are retained beside this report. The run recorded 70 exact-resolution equivalence checks and 30 measured samples at each of 128 and 2,048 records. Timing values are observations, not release thresholds.

Other release checks passed: 86 unit tests, 2 restore integration tests, the stewardship contract vector test, lint, security-boundary checks, upgrade-path coverage for all 14 archived versions, and the generated-install-archive comparison. The archive comparison caught and fixed a stale pgrx source-line comment; the clean-candidate E2E was rerun after that sync.

## Acceptance matrix

| ID | Verdict | Observation and oracle |
| --- | --- | --- |
| R1 | passed | Real entity-role SQL creates, replaces, activates, pauses, and rejects stale/noncanonical binding requests; exact durable/runtime rows are checked in `tests/e2e_policy.sql`. Role attributes, membership, action, limit, and digest guards are in `src/api/policy_intent.rs`. |
| R2 | passed | Actual `pgreact_mdm_worker` identity, role change, privileged-helper paths, allowed intent call, and denied human/identity calls are recorded in [pg-react-v0.48.0.md](pg-react-v0.48.0.md). |
| R3 | passed | The public typed intent path accepts the three action shapes; malformed extra keys are rejected with no receipt or control change in `tests/e2e_policy.sql`. Digest, range, reference, key, and JSON-shape validation is performed before database writes in `src/api/policy_intent.rs` and `src/policy.rs`. |
| R4 | passed | E2E compares first apply, identical retry, changed-body conflict, original receipt, and control state. The intent path checks the bound role name and OID before receipt replay in `src/api/policy_intent.rs`. |
| R5 | passed | Stale escalation levels and stale human-control revisions are rejected without overwriting controls. The writer acquires the binding row before the case row and checks all freshness tokens in `src/api/policy_intent.rs`; exact receipts and winning rows are checked in `tests/e2e_policy.sql`. |
| R6 | passed | E2E checks allowed assignment, unchanged assignment, unbound queue, and manual protection, including exact control revision and receipt fields in `tests/e2e_policy.sql`. |
| R7 | passed | E2E checks valid, earlier-than-open, and beyond-limit deadlines against exact due time, outcome, receipt, and action revision. Unknown opening time and timestamp parsing are guarded in the intent path and projection code. |
| R8 | passed | E2E checks early, due, skipped, maximum, and over-limit escalation states and exact receipts. The intent path locks the case row and validates open status, due time, next level, and binding maximum before changing controls. |
| R9 | passed | Applied, no-op, denied, and limit receipts are checked for exact result fields and null publication revision. All authenticated terminal outcomes share the receipt writer; malformed calls are checked to create no receipt. |
| R10 | passed | E2E applies within an outer transaction, verifies full control and receipt state, rolls back both, then retries; a committed identical retry returns the original receipt without a second revision increment. PostgreSQL transaction atomicity and the unique request key provide the transaction oracle. |
| R11 | passed | Entity-role SQL checks changed and no-op human controls, manual protection, and stale revisions. The API validates execution-role identity, open status, queue, escalation level, reason, and optimistic revision in `src/api/policy_intent.rs`. |
| R12 | passed | Logical restore preserves exact policy cases, bindings, receipt body, and digest; runtime rows are absent and a worker call fails before reconciliation. The restore and clone checks are in `tests/restore.sql` and the React-side actual-worker restore evidence. |
| R13 | passed | Failed drop compares complete entity, case, binding, receipt, runtime, and graph-member snapshots; successful drop asserts their removal. Upgrade, generated archive, and security-boundary gates pass. |
| R14 | passed | The joint React evidence checks work/request/receipt correlation and unchanged observation/action revisions for audit-only events; see [pg-react-v0.48.0.md](pg-react-v0.48.0.md). |

## Limits

The suite exercises stale-token behavior and the row-lock order, but it does not schedule a separate two-session race for every M2 intent versus human close/pause/replacement combination. The rollback/retry case validates idempotent recovery after rollback and replay after commit; it does not deliberately sever a client connection after the server commits. The performance samples are synthetic and do not establish production capacity or SLOs.
