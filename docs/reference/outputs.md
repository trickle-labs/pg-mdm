# Outputs, refresh results, and explanations

[Documentation home](../README.md) · Reference

Each entity publishes three ordinary tables in `mdm_out`. They are created on the first successful refresh and updated as part of the same publication transaction. Grant readers access explicitly, and use the supported actions to change the inputs and durable instructions that produce them. These outputs are a consumption surface, not a replacement for the definition, matching, or stewardship APIs.

## Golden entity table

`mdm_out.<entity>` has one row per active resolved entity. Only fields listed in `golden_values` become value columns; their types follow the validated output definition. Golden values can be null when no eligible usable value can be selected.

| Column | Type | Meaning |
| --- | --- | --- |
| `mdm_id` | `uuid`, primary key | Durable identifier assigned through the identity ledger. |
| Configured golden fields | Their declared SQL types | Selected values, preserving the chosen source representation or override. |
| `member_count` | `bigint`, not null | Number of active source members in the resolved entity. |
| `has_review` | `boolean`, not null | Whether an applicable open review is reflected in this entity's published review state. |
| `last_change_revision` | `bigint`, not null | Publication revision associated with the row's latest change. |

The system reserves `mdm_id`, `member_count`, `has_review`, and `last_change_revision`, along with PostgreSQL system-column names, so golden fields cannot collide with them. Previously published fields are append-only in retained name, order, and type. See [src/output.rs](../../src/output.rs) for the generated DDL and validation.

## Source membership table

`mdm_out.<entity>_members` connects source records to resolved identities and retains inactive membership. Filter `active` when your query means “current source members”; keeping the historical rows is useful when you need to understand where an old source reference went.

| Column | Type | Meaning |
| --- | --- | --- |
| `source_record_id` | `uuid`, primary key | Durable source-record identifier; used by pair decisions and override anchors. |
| `source_name` | `name` | Source name from the entity definition. |
| `source_id` | `jsonb` | Encoded representation of the source key. |
| `mdm_id` | `uuid` | Identity assigned to this retained membership. |
| `active` | `boolean` | Whether the source membership is active. |
| `first_membership_revision` | `bigint` | First revision recorded for the membership. |
| `last_membership_revision` | `bigint` | Latest membership revision. |
| `membership_reason` | `text` | Reason associated with the membership transition. |
| `last_change_revision` | `bigint` | Latest publication revision changing the output row. |

All membership columns are non-null. A source key and an `mdm_id` answer different questions: the former identifies a source record, while the latter identifies the entity currently resolved from one or more records. Use source-record UUIDs for stewardship arguments even when your application normally displays customer UUIDs.

## Review table

`mdm_out.<entity>_review` contains retained review occurrences, including resolved ones. A familiar issue that reappears can have another occurrence, so neither a reason code nor an issue key alone should be treated as the complete row identity.

| Column | Type | Meaning |
| --- | --- | --- |
| `review_id` | `uuid`, primary key | Identifier for the review occurrence. |
| `issue_key` | `bytea` | Deterministic issue identity. |
| `occurrence` | `integer` | Occurrence number for that issue. |
| `status` | `text` | `open` or `resolved`. |
| `severity` | `text` | Published severity classification. |
| `reason_code` | `text` | Stable reason classification for this review. |
| `subjects` | `jsonb` | Subjects involved in the issue. |
| `masked_summary` | `jsonb` | Bounded review summary. |
| `opened_revision` | `bigint` | Publication revision opening the occurrence. |
| `resolved_revision` | Nullable `bigint` | Revision resolving it, if resolved. |
| `last_change_revision` | `bigint` | Latest publication revision changing the row. |
| `concurrency_version` | `bigint` | Review version used for concurrency checks. |

All columns except `resolved_revision` are non-null. A review concurrency version is not the decision version, override version, binding version, or policy action revision. Keep each counter with the object it protects instead of substituting whichever version happens to be visible in a UI.

## Refresh result

`mdm.refresh()` and `mdm_admin.rebuild()` return JSON describing the committed operation and the work it performed. A transaction containing the call still needs to commit before another session sees its publication. The latest successful refresh outcome is also available under `mdm.describe(entity)->'publication'->'refresh'`.

| Fields | Meaning |
| --- | --- |
| `operation_id`, `entity_name` | Operation identity and named entity. |
| `changed`, `publication_revision` | Whether a semantic publication change occurred and the current revision; a no-change call can retain the revision. |
| `graph_refresh_id` | Producer graph refresh identifier. |
| `source_boundary`, `source_boundary_digest` | Captured source frontier and its digest; successful publication requires a proven boundary. |
| `node_results`, `effective_node_modes` | Per-node execution outcomes and graph modes. |
| `active_records`, `identities`, `open_reviews` | Current entity-scale counts. |
| `resolver_strategy`, `resolver_fallback_reason` | Affected or full resolution and the reason for fallback when present. |
| `affected_records`, `affected_components`, `component_checks` | Scope and resolver-work diagnostics. |
| `delta_batch_count`, `delta_row_count` | Consumed delta workload. |
| `delta_acknowledged_token`, `delta_lag` | Delta progress, nullable when no such value applies. |
| `unexpected_full_fallbacks` | Full graph behavior that should be examined against the qualified scenario. |
| `stage_timings_ms` | Stage costs, including source, evaluation, publication assembly, digest, and snapshot work. |
| `shadow_comparison` | Optional comparison metadata; when present includes affected counts, equivalence, and comparison digests. |

A small affected set does not imply that every stage is incremental. Use the timing and fallback fields together with source shape, candidate density, and retained history. The [operations guide](../how-to/operate.md#understand-where-incremental-work-still-has-a-cost) identifies the stages that still do global work and links to the retained measurement evidence.

## Explanation subjects

`mdm.explain(entity_name, subject, publication_revision, max_facts)` returns `facts`, `truncated`, `identity`, and `retained_revision_range`. `max_facts` defaults to 100 and must be between 1 and 500. The accepted subject shapes are wider than the detailed fact implementation in this preview:

| Subject kind | JSON keys | Current result boundary |
| --- | --- | --- |
| `pair` | `kind`, `left_id`, `right_id` | Detailed retained pair facts; UUID strings identify source records. Use an explicit publication revision for historical evidence. |
| `mdm_id` | `kind`, `id` | Current identity status in `identity`; not a reconstructed historical status. |
| `source_record` | `kind`, `id` | Shape accepted; current tests specify an empty fact list. |
| `golden` | `kind`, `mdm_id`, `field` | Shape accepted; current tests specify an empty fact list. Inspect outputs and definition for value selection. |
| `review` | `kind`, `id` | Shape accepted; current tests specify an empty fact list. Inspect the review table and related pair evidence. |
| `publication` | `kind`, `revision` | Shape accepted; current tests specify an empty fact list. Inspect publication metadata through `describe`. |

Unknown or missing keys are rejected. A null `publication_revision` reads retained pair facts across revisions, newest first and bounded by `max_facts`; pass a revision to restrict the response to that publication. Supplying a revision does not make other accepted subject kinds historical reconstructions. If retained evidence for a requested revision is unavailable, the call can raise `MDM_EXPLANATION_NOT_RETAINED`. A response with `truncated = true` is explicitly incomplete. These boundaries are implemented in [src/api/explain.rs](../../src/api/explain.rs) and exercised in [tests/e2e.sql](../../tests/e2e.sql).

## Policy relations

Policy integration uses versioned relations in `mdm_steward`. They supplement the three primary entity outputs and require deliberate grants. The [automation guide](../how-to/automate-stewardship.md) describes the write protocol; the full column definitions are in [src/schema.rs](../../src/schema.rs).

| Relation | Purpose and notable fields |
| --- | --- |
| `policy_cases_v1` | Case projection keyed by `case_key`, including review identity, approved metadata, permitted actions, queue/due/escalation controls, manual-assignment protection, opening/resolution times, freshness versions, evidence digest, action revision, and pending-publication state. |
| `policy_bindings_v1` | Durable policy authorization: automation role, policy digest, allowed actions and queues, bounds, binding version, and replacement history. |
| `policy_receipts_v1` | Immutable request/response history keyed by receipt ID and unique `(binding_id, request_key)`, with action, outcome, reason, action revision, request body, actor identities, and tracking-start metadata. |
| `policy_receipt_publications_v1` | Candidate receipt-to-publication observation: receipt ID, case/action revision, publication revision, state, and observation time. |

`policy_cases_v1.pending_since` is nullable. A null value can mean that an older pending decision has unknown age; do not turn that into an assumed zero duration. The receipt-publication relation records `PUBLISHED` or `SUPERSEDED_BEFORE_PUBLICATION` when tracking observes the corresponding state. Historical receipts may have no tracking row. The receipt's original `resulting_publication_revision` must not be treated as a promise of later publication.

An identical policy request with the same key replays its immutable receipt. Reusing that key with a different body returns `IDEMPOTENCY_CONFLICT`, reason `REQUEST_KEY_BODY_MISMATCH`, and a null receipt ID. `APPLIED_CONTROL` confirms a control write, while SQL errors can occur before a receipt exists. Binding version, runtime version, review version, definition version, publication revision, stewardship epoch, and action revision protect different state and must be carried separately by an integration.
