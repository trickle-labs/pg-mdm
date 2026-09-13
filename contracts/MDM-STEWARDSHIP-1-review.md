# MDM-STEWARDSHIP/1 joint sign-off record

The approved MDM-side contract is
[`MDM-STEWARDSHIP-1.json`](MDM-STEWARDSHIP-1.json), SHA-256
`2161ce22b9d924ff3f3e6d8acde62fed01b6b1c1c4d7ba0fd4d5eb6e396e0fcf`. The
shared vectors are in [`MDM-STEWARDSHIP-1-fixture.json`](MDM-STEWARDSHIP-1-fixture.json),
SHA-256 `900f365533bab13480fcb88ab8e8b7956beb4f14a8cfc92cd46770b2e26ef3e4`.
The contract is approved; this record pins the owner approvals and test evidence
to those exact bytes.

The user reported approval by the pg-mdm M0 contract owner and pg-react R0
adapter owner on 2026-09-13. Their approvals apply to the contract SHA-256 and
fixture SHA-256 above. Personal names were not supplied; the approving roles
are recorded as the owner identifiers.

Both repositories contain identical contract and fixture bytes. The MDM
stewardship contract test and pg-react conformance test pass against them.

| Decision | Sign-off status | Frozen contract terms |
|---|---|---|
| Policy digest | Resolved on 2026-09-13. React owns digest generation; the shared contract pins its preimage and vector `741ea9560a69ba3185eaa34760ba38d473aa43daa8e8c530c6c6c2ce867c2614`. | Hash canonical JSON v1 containing `canonical_encoding_version`, `policy_revision`, and the complete normalized `policy_package`, using domain tag `pg_react/mdm-stewardship-policy/v1`. MDM treats the result as opaque and compares its 32 bytes with the active binding. |
| Request-key contract | Resolved on 2026-09-13. Both repositories adopted the canonical v1 encoding, domain tag, field definitions, and vector digest `3ebd5539467befabbc0492e51e49fe2dc866a225cc73a36256d0a95a06b06d4e`; both conformance tests pass. | React derives 32 bytes with length-framed SHA-256 over `pg_react/mdm-stewardship-request-key/v1` and canonical JSON v1 fields `binding_id`, `policy_revision`, `case_key`, `lifecycle_generation`, `action_revision`, `consequence_identity`, and `escalation_level`. Retries reuse the persisted key and body. |
| Idempotency conflict | Resolved on 2026-09-13. The response is explicit and both conformance tests pass. | Return `receipt_id = null`, `outcome = IDEMPOTENCY_CONFLICT`, and `reason_code = REQUEST_KEY_BODY_MISMATCH`. Do not insert a conflict receipt or change the existing receipt. After a concurrent unique-key failure, read the original row and compare request digests. |
| Receipt storage | Resolved on 2026-09-13. The receipt relation, actor source, access, and retention are now specified in the shared bytes. | Freeze `mdm_steward.policy_receipts_v1` with a 32-byte request key and digest, canonical request body, actor, action result, and creation time. MDM records the authenticated effective database role before privileged execution; React cannot supply the actor. Enforce `UNIQUE(binding_id, request_key)`, grant React `SELECT` only, and retain and dump receipts without automatic deletion. |
| Review version | Resolved on 2026-09-13. `review_version` is pinned to the public review row's `concurrency_version`. | Use the exact `mdm_out.<entity>_review.concurrency_version` for the case's `review_id`; pin it with the definition and latest-successful-observation versions. |
| Observation freshness | Resolved on 2026-09-13. Pending cases fail closed with a typed outcome. | Deny intents while `pending_stewardship` is true with `PENDING_STEWARDSHIP`; allow them only after a successful observation clears the flag. |
| Binding administration | Resolved on 2026-09-13. The binding relation, role checks, administrator surface, pause, replacement, and restore behavior are frozen. | `mdm_steward.policy_bindings_v1` allows one active binding per scope. The existing MDM administrator path registers, pauses, and replaces it; React has no direct relation access. Replacements use a fresh ID and old bindings fail closed. |
| Shared fixture | Resolved on 2026-09-13. Both repositories carry SHA-256 `900f365533bab13480fcb88ab8e8b7956beb4f14a8cfc92cd46770b2e26ef3e4`, and both conformance tests pass. | Keep `MDM-STEWARDSHIP-1-fixture.json` byte-identical in both repositories and pin it beside the contract digest. |

The frozen contract SHA-256 is
`2161ce22b9d924ff3f3e6d8acde62fed01b6b1c1c4d7ba0fd4d5eb6e396e0fcf`; the
fixture SHA-256 is recorded above. Both repositories adopted these bytes and
passed conformance tests. The contract is approved by both owner roles, closing
the v0.12 contract exit criterion and unblocking v0.13 policy projection and
intents.
