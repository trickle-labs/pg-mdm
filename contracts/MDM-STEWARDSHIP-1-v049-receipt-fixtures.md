# MDM-STEWARDSHIP/1 v0.49 — owner-confirmed negative receipt fixtures

**Fixture status:** Expected behavior was confirmed by the requesting owner on 2026-10-19. Executable proof is in [`tests/e2e_policy.sql`](../tests/e2e_policy.sql), run by [`scripts/run_e2e_tests.sh`](../scripts/run_e2e_tests.sh) against a freshly packaged `pg_mdm` build and the pinned patched producer. **A committed fixture is not a passing qualification**; use the exact CI run for the candidate commit and do not treat this document as release acceptance.

**Owner decision:** The requesting owner ratified the [v0.49 cross-repository addendum](MDM-STEWARDSHIP-1-escalation-v049.md); the ratification was recorded in [pg-react issue #11](https://github.com/trickle-labs/pg-react/issues/11#issuecomment-6269031900). The requester separately confirmed the three negative expected behaviors in the implementation handoff and requested their concrete fixtures. This document records their expected results, not a third-party independent review or production activation.

## Exact outcomes and identities

| Scenario | Request key (hex) | MDM outcome | Reason or error | Receipt persisted? | Control mutation? |
| --- | --- | --- | --- | --- | --- |
| Open, due case, level other than current + 1 | `d1` repeated 32 times | `STALE_CASE` | `ESCALATION_NOT_NEXT` | Exactly one; replay retains original receipt ID | None |
| Valid request on paused binding | `d2` repeated 32 times | `BINDING_PAUSED` | `BINDING_PAUSED` | Exactly one; replay retains original receipt ID | None |
| Valid request on replaced binding | `d3` repeated 32 times | `BINDING_REPLACED` | `BINDING_REPLACED` | Exactly one | None |
| Zero, negative, string, or int32-overflow level | `d4`–`d7` repeated 32 times | SQL exception, not an outcome | `MDM_POLICY_INTENT`: `level must be a positive integer` | No | None |
| Extra or missing level key | `d8`–`d9` repeated 32 times | SQL exception, not an outcome | `MDM_POLICY_INTENT`: `ESCALATE arguments must contain only level` | No | None |

Each executable *receipt* assertion checks the exact canonical JSON request body, binding and request-key identity, principal and session role, binding version, case occurrence, action and revision, outcome and reason, complete returned control JSON, matching persisted control JSON, null publication revision, valid 32-byte request digest, non-null receipt ID and timestamp, and exactly one receipt row. A before/after `to_jsonb(policy_cases_v1)` comparison asserts every case column is unchanged. Same-key replay verifies the returned receipt identity and complete control.

The malformed-level loop uses the real bound worker, case, binding and freshness values; it expects the exact parser error and verifies no receipt with each distinct request key and no change to any case field. It intentionally does not create synthetic `ACTION_DENIED` receipts for invalid requests.

## Qualification and dependencies

- The unchanged E2E script covers all fixtures, including the preexisting applied-control, deadline, reduction and replay tests. It already exercises an authentic active → paused binding transition and retains the replaced binding.
- Expected output and error strings reflect the merged MDM implementation in [PR #18](https://github.com/trickle-labs/pg-mdm/pull/18), not an invented React receipt codec.
- Before using this as a prerequisite for [pg-react #11](https://github.com/trickle-labs/pg-react/issues/11), require green `test` and `e2e` jobs for the *same final commit*, independent code review, and an approved source/artifact identity. Then link the final MDM commit and CI run on #11.
- These fixtures do not prove idle managed delivery, workload operating bounds, restart/clone/restore, the upstream [pg-trickle #1168](https://github.com/trickle-labs/pg-trickle/issues/1168) throughput gate, or [React #12](https://github.com/trickle-labs/pg-react/issues/12) joint release qualification.
