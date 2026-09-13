# MDM-STEWARDSHIP/1 joint-review handoff

The current MDM-side proposal is
[`MDM-STEWARDSHIP-1.json`](MDM-STEWARDSHIP-1.json), SHA-256
`25dc84f8ae1f09374318b55be27540c3e195eb51cad7b9cd11fb135039e801db`.
It remains `proposed_for_joint_review`; neither owner has approved it. This
handoff records the exact items to settle with pg-react before adopting one
contract and fixture digest. It is not an approval record.

| Decision | Current gap | Proposed resolution for joint review |
|---|---|---|
| Policy digest | The contract checks a 32-byte `expected_policy_digest` but does not define its canonical preimage or include a policy digest vector. | Define a versioned canonical JSON policy snapshot and domain-separated SHA-256 vector; keep the digest opaque to MDM after validating length. |
| Request-key contract | The MDM draft says the worker supplies 32 bytes, but does not say whether React derives the key or how it persists it across retries. | Treat it as an opaque 32-byte idempotency key generated once per logical request and reused byte-for-byte on retries; if React requires derivation, publish the exact domain, preimage, and vector instead. |
| Idempotency conflict | The conformance prose requires a conflict outcome for a reused key with a changed body, but `IDEMPOTENCY_CONFLICT` is absent from `intent.outcomes`. | Add `IDEMPOTENCY_CONFLICT` as a terminal receipt outcome and define that the prior receipt remains unchanged. |
| Receipt storage | Retention and dump behavior are stated, but the relation columns, uniqueness, read permissions, and exact retained body are not frozen. | Freeze a receipt table with `receipt_id`, `binding_id`, 32-byte `request_key`, 32-byte request digest, canonical request body, outcome/reason, case and revision results, control result, and creation time; enforce `UNIQUE(binding_id, request_key)`, deny direct React table access, and retain/dump without automatic deletion. |
| Review version | The contract uses `review_version`; the MDM review row exposes `concurrency_version`. | Define `review_version` as the exact value of `mdm_out.<entity>_review.concurrency_version`; pin it with definition and latest-successful-observation versions. |
| Observation freshness | The JSON defines `stewardship_epoch`, but the intent rule should state whether an unobserved newer decision epoch blocks an action. | Deny intents while `pending_stewardship` is true, using a typed `PENDING_STEWARDSHIP` outcome; allow them again only after a successful observation. |
| Binding administration | The intent signature is fixed, but registration, pause, replacement, principal scope, and permission ownership for a binding are not fully specified. | Freeze the binding relation and administrator-only SQL surface, including who may pause or replace a binding and how a replacement invalidates old intents. |
| Shared fixture | The canonical vectors are embedded in the proposal, but pg-react has not adopted a copy or digest of the exact MDM fixture. | Publish the approved contract and conformance fixture bytes in both repositories and run the same vectors in both CI suites. |

After both owners agree, update the JSON and vectors, compute the final contract
and fixture SHA-256 values, copy those exact bytes to pg-react, pass both
repositories' tests against them, and record each owner's name and approval
date. Until then, the contract exit criterion remains open and v0.13 policy
projection/intents must not start.
