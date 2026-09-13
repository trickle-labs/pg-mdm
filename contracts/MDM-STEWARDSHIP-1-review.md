# MDM-STEWARDSHIP/1 joint-review handoff

The current MDM-side proposal is
[`MDM-STEWARDSHIP-1.json`](MDM-STEWARDSHIP-1.json), SHA-256
`babd8510203cac5b4d0486e82a76b4d306ccec9bd539c778e444bfe3ca23764e`. The
shared vectors are in [`MDM-STEWARDSHIP-1-fixture.json`](MDM-STEWARDSHIP-1-fixture.json),
SHA-256 `00d0eaad21601b0048ff7139fac435ee04f222ffc26894698bd97418ac5b8e06`.
The contract remains `proposed_for_joint_review`, and its approval fields
remain pending. This handoff records the items to settle with pg-react before
the owners approve the frozen contract.

Both projects approve this document as a joint-review handoff, as reported on
2026-09-13. That approval does not approve `MDM-STEWARDSHIP/1`. Both owners
must approve the final contract after the remaining R0/M0 decisions are closed.
Both repositories now contain these exact contract and fixture bytes, and
their stewardship conformance tests pass against them.

| Decision | Current gap | Proposed resolution for joint review |
|---|---|---|
| Policy digest | The contract checks a 32-byte `expected_policy_digest` but does not define its canonical preimage or include a policy digest vector. | Define a versioned canonical JSON policy snapshot and domain-separated SHA-256 vector; keep the digest opaque to MDM after validating length. |
| Request-key contract | Resolved on 2026-09-13. Both repositories adopted the canonical v1 encoding, domain tag, field definitions, and vector digest `3ebd5539467befabbc0492e51e49fe2dc866a225cc73a36256d0a95a06b06d4e`; both conformance tests pass. | React derives 32 bytes with length-framed SHA-256 over `pg_react/mdm-stewardship-request-key/v1` and canonical JSON v1 fields `binding_id`, `policy_revision`, `case_key`, `lifecycle_generation`, `action_revision`, `consequence_identity`, and `escalation_level`. Retries reuse the persisted key and body. |
| Idempotency conflict | Resolved on 2026-09-13. The response is explicit and both conformance tests pass. | Return `receipt_id = null`, `outcome = IDEMPOTENCY_CONFLICT`, and `reason_code = REQUEST_KEY_BODY_MISMATCH`. Do not insert a conflict receipt or change the existing receipt. After a concurrent unique-key failure, read the original row and compare request digests. |
| Receipt storage | Resolved on 2026-09-13. The receipt relation, actor source, access, and retention are now specified in the shared bytes. | Freeze `mdm_steward.policy_receipts_v1` with a 32-byte request key and digest, canonical request body, actor, action result, and creation time. MDM records the authenticated effective database role before privileged execution; React cannot supply the actor. Enforce `UNIQUE(binding_id, request_key)`, grant React `SELECT` only, and retain and dump receipts without automatic deletion. |
| Review version | The contract uses `review_version`; the MDM review row exposes `concurrency_version`. | Define `review_version` as the exact value of `mdm_out.<entity>_review.concurrency_version`; pin it with definition and latest-successful-observation versions. |
| Observation freshness | The JSON defines `stewardship_epoch`, but the intent rule should state whether an unobserved newer decision epoch blocks an action. | Deny intents while `pending_stewardship` is true, using a typed `PENDING_STEWARDSHIP` outcome; allow them again only after a successful observation. |
| Binding administration | The intent signature is fixed, but registration, pause, replacement, principal scope, and permission ownership for a binding are not fully specified. | Freeze the binding relation and administrator-only SQL surface, including who may pause or replace a binding and how a replacement invalidates old intents. |
| Shared fixture | Resolved on 2026-09-13. Both repositories carry SHA-256 `00d0eaad21601b0048ff7139fac435ee04f222ffc26894698bd97418ac5b8e06`, and both conformance tests pass. | Keep `MDM-STEWARDSHIP-1-fixture.json` byte-identical in both repositories and pin it beside the contract digest. |

The frozen contract SHA-256 is
`babd8510203cac5b4d0486e82a76b4d306ccec9bd539c778e444bfe3ca23764e`; the
fixture SHA-256 is recorded above. Both repositories have adopted these bytes,
and both conformance tests pass. Policy-digest ownership, the review-version
mapping, pending-stewardship behavior, and binding administration remain open.
The handoff approvals are recorded; contract-owner approvals remain pending
until those decisions are settled. The v0.12 contract exit criterion remains
open, and v0.13 policy projection and intents must not start.
