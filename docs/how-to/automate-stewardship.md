# Automate stewardship controls

[Documentation home](../README.md) · How-to guide

This guide explains how the policy interface assigns review cases to queues, sets due dates, and escalates cases while preserving evidence and concurrency checks. Practice only with synthetic cases in an isolated environment: the current [demo preview must not enable live controls](../reference/compatibility.md). Supported actions are `ASSIGN_QUEUE`, `SET_DUE_AT`, and `ESCALATE`; they do not make pair decisions or override golden values. The examples assume an entity producing synthetic review cases, a reviewed demonstration policy, and separate administrator and automation roles. The [first tutorial](../tutorials/first-entity.md) intentionally produces no cases and is not a policy fixture.

## Establish the roles and policy boundary

Create a dedicated `NOLOGIN NOSUPERUSER NOBYPASSRLS` automation role and let a restricted service login select it. The automation role must not be the helper or entity execution role, or inherit membership that crosses those protected boundaries. As the entity execution role, use `mdm_admin.create_policy_binding()` to bind the automation role to one entity, a 32-byte policy digest, sorted distinct allowed actions, sorted distinct queues, and bounds for due dates and escalation. Grant the administrative function separately before calling it.

This example assumes an administrator has created `customer_policy_worker`, the current entity execution role has the function grant, and `policy_sha256` contains the reviewed policy's SHA-256 hex digest in `psql`. It permits only queue assignment, so it has no due-date bound and no positive escalation ceiling:

```sql
SELECT binding_id, binding_version
FROM mdm_admin.create_policy_binding(
    entity_name => 'customer',
    automation_role_name => 'customer_policy_worker',
    policy_digest => decode(:'policy_sha256', 'hex'),
    allowed_actions => ARRAY['ASSIGN_QUEUE'],
    allowed_queues => ARRAY['customer-review'],
    max_due_interval => NULL,
    max_escalation_level => 0
);
```

Retain the returned binding ID and version. Creating a binding establishes an active runtime binding for the current preloaded server; replacing a binding creates a new paused binding that needs explicit activation. The helper grants the bound role read access to the policy case, receipt, and receipt-publication relations and execution of `submit_policy_intent()`. These are table privileges on the policy projection, so review the visibility of that projection when choosing service identities; the binding's write scope does not turn a broad table grant into per-entity read isolation.

## Read one coherent case snapshot

As the automation role, read `mdm_steward.policy_cases_v1`. A decision needs the case key, current permitted actions, all freshness versions, the evidence-basis digest, and current controls. Preserve the complete snapshot used by your policy evaluator, together with the policy revision, evaluation reference, and work reference. The expected values supplied to an intent must come from that snapshot, not from separate reads taken at different points in the workflow.

```sql
SELECT case_key, entity_name, status, permitted_actions,
    assigned_queue, due_at, escalation_level, manual_assignment_protected,
    review_version, definition_version, publication_revision,
    stewardship_epoch, evidence_basis_digest, action_revision,
    opened_at, pending_stewardship, pending_since
FROM mdm_steward.policy_cases_v1
WHERE entity_name = 'customer' AND status = 'open'
ORDER BY case_key;
```

An action must be allowed both by the case and by its binding. Missing historical `opened_at` values are genuinely unknown; do not manufacture an opening time from the current clock. The `SET_DUE_AT` permission depends on the case's timing information and supported bounds. [Output reference](../reference/outputs.md#policy-relations) describes the projection and the 0.15 candidate's nullable pending-age fields.

## Submit an intent and keep the receipt

Call `mdm_steward.submit_policy_intent()` with the binding ID, a stable 32-byte request key, the case snapshot's expected values, the binding policy digest, and nonblank policy/evaluation/work references. The exact signature is in [SQL API](../reference/sql-api.md#policy-functions). For `ASSIGN_QUEUE`, the arguments are `'{"queue":"customer-review"}'::jsonb`; for `ESCALATE`, use `'{"level":1}'::jsonb`; for `SET_DUE_AT`, supply the supported `due_at` timestamp string. Unknown argument keys are rejected. Validate permissions and limits against the actual case instead of copying a demonstration value into a live request.

For a complete `psql` example, supply the following variables from your demonstration policy workflow before running the SQL. Choose a synthetic case deliberately; do not use an arbitrary first row from the projection.

| Variable | Value you supply |
| --- | --- |
| `selected_case` | The positive `case_key` of the synthetic customer case you reviewed. |
| `binding_id` | UUID returned when the matching worker binding was created. |
| `policy_sha256` | The reviewed binding policy digest, as 64 hexadecimal characters. |
| `request_key_hex` | A unique 32-byte key encoded as 64 hexadecimal characters; retain it for exact retries. |
| `policy_revision`, `evaluation_ref`, `work_ref` | Nonblank references identifying this demonstration policy, evaluation, and work item. |

Connect as the worker's restricted login and select its bound automation role. Capture all case freshness values in one query, then submit the intent using that retained snapshot. The query must return exactly one row; if `\gset` reports no row, stop and reconsider the chosen case instead of proceeding with old variables. The demonstration binding above must allow the queue `customer-review`.

```sql
\set ON_ERROR_STOP on
SELECT case_key, review_version, definition_version, publication_revision,
    stewardship_epoch, encode(evidence_basis_digest, 'hex') AS evidence_digest,
    action_revision
FROM mdm_steward.policy_cases_v1
WHERE case_key = :selected_case
  AND entity_name = 'customer'
  AND status = 'open'
  AND 'ASSIGN_QUEUE' = ANY(permitted_actions)
\gset observed_

SELECT * FROM mdm_steward.submit_policy_intent(
    binding_id => :'binding_id'::uuid,
    request_key => decode(:'request_key_hex', 'hex'),
    case_key => :observed_case_key,
    action => 'ASSIGN_QUEUE',
    arguments => '{"queue":"customer-review"}'::jsonb,
    expected_review_version => :observed_review_version,
    expected_definition_version => :observed_definition_version,
    expected_publication_revision => :observed_publication_revision,
    expected_stewardship_epoch => :observed_stewardship_epoch,
    expected_evidence_basis_digest => decode(:'observed_evidence_digest', 'hex'),
    expected_action_revision => :observed_action_revision,
    expected_policy_digest => decode(:'policy_sha256', 'hex'),
    policy_revision => :'policy_revision',
    evaluation_ref => :'evaluation_ref',
    work_ref => :'work_ref'
);
```

Inspect the returned outcome; current controls, protection, or changed observations may prevent application even though the request was well formed. Store the immutable receipt's `receipt_id`, `outcome`, `reason_code`, `action_revision`, and control result when a receipt is returned. `APPLIED_CONTROL` means the control write was applied; it is not evidence that a subsequent MDM publication has happened. In the 0.15 candidate, `policy_receipt_publications_v1` can later connect a receipt to `PUBLISHED` or `SUPERSEDED_BEFORE_PUBLICATION`. Older receipts can lack this tracking, and the original receipt is not rewritten to invent it.

When a delivery result is ambiguous, retry the **identical request with the same key**. A matching stored receipt is replayed; reusing the key with a changed body returns `IDEMPOTENCY_CONFLICT` with reason `REQUEST_KEY_BODY_MISMATCH` and a null receipt ID. If the case has changed and the policy must reevaluate it, perform a new evaluation using a fresh snapshot and a new request key. Do not overwrite the original expected versions and call that a retry. Malformed, unauthorized, or activation-blocked requests can raise SQL errors without creating a receipt, so handle those errors separately from returned terminal outcomes.

## Pause, replace, and reactivate bindings

Use `mdm_admin.set_policy_binding_state(binding_id, expected_runtime_version, state, reason)` as the entity execution role, with `state` equal to `paused` or `active`. Runtime version and binding version are different counters: record returned runtime versions, and have an authorized administrator inspect `mdm_internal.policy_binding_runtime` when recovering that state after a restart or restore. The runtime relation is an administrative recovery detail, not a table for the worker to edit.

Pause delivery before a producer or adapter upgrade, restore, clone, or policy replacement. Recheck the intended database, roles, source privileges, helper ownership, producer capabilities, and policy digest, then activate using the current runtime version. A restored logical database may have no runtime row, in which case the expected runtime version is 0; a retained row requires its actual version. PostgreSQL must have `pg_mdm` preloaded for activation. Pausing remains available when preload is missing, and a stored matching receipt can still replay while new intents fail closed.

Use `replace_policy_binding()` to change the approved digest, actions, queues, or bounds with the existing binding version. It retains the replaced binding and its receipt history, returns a new binding, and starts that replacement paused. Activate the new binding only after its environment and policy checks pass. For an executable integration fixture covering action bounds, replay, stale observations, and publication timing, consult [tests/e2e_policy.sql](../../tests/e2e_policy.sql) and the versioned [MDM-STEWARDSHIP-1 contract](../../contracts/MDM-STEWARDSHIP-1.json).

For a human control edit, `mdm_steward.set_case_controls()` uses the current action revision and a reason. Its `manual_assignment_protected` flag is part of the state automation must respect. Identity decisions and golden overrides remain in the separate [stewardship workflow](steward-records.md).
