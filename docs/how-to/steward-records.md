# Steward records and golden values

[Documentation home](../README.md) · How-to guide

Use stewardship when a person has evidence that should change how a pair is treated or which value is published. A useful decision has a subject, a reason, and an observed version. pg_mdm records that instruction durably, validates contradictions and concurrency, and applies its effect during a subsequent refresh. You need the entity's execution role and explicit grants for the steward functions you will call; direct edits to protected catalogs are not part of this workflow.

## Find the subject and inspect its evidence

Read open cases from `mdm_out.<entity>_review`, then use the membership table to resolve the referenced source-record UUIDs. `source_record_id` identifies a retained source record, while `mdm_id` identifies the resolved entity at a publication. Pair decisions and golden overrides use source-record UUIDs, so avoid copying a customer UUID into a source-record argument.

```sql
SELECT review_id, severity, reason_code, subjects, concurrency_version
FROM mdm_out.customer_review
WHERE status = 'open'
ORDER BY opened_revision, review_id;

SELECT source_record_id, source_name, source_id, mdm_id, active
FROM mdm_out.customer_members
ORDER BY source_name, source_record_id;
```

Use a pair `mdm.explain()` request to inspect the matching evidence before writing an instruction, and inspect the output plus desired definition for a golden-value decision. The current API accepts several other subject shapes but provides detailed retained facts only for pairs; an `mdm_id` request adds current identity status. The [explanation subject reference](../reference/outputs.md#explanation-subjects) makes this boundary explicit. For historical pair evidence, pass its revision explicitly and respect `MDM_EXPLANATION_NOT_RETAINED`: current evidence must not be presented as a reconstruction of a missing historical explanation.

## Record or replace a pair decision

Call `mdm_steward.decide()` with the two source-record UUIDs, a supported decision, the current version, and a nonblank reason. The accepted decision strings are `MATCH` and `NOT_MATCH`, representing a manual must-link or cannot-link constraint. A new pair begins at version 0; every accepted replacement advances its decision version. There is no public pair-decision clearing action in this version. Component-wide contradictions are checked before acceptance, so a must-link chain cannot silently defeat an existing cannot-link constraint.

The following `psql` example expects `left_record` and `right_record` to identify the pair you have reviewed, and `decision_version` to come from your previously retained steward result, or 0 for a pair with no prior decision. A review's `concurrency_version` is a separate counter and cannot be used as the pair decision version. The identifiers are deliberately supplied by the operator instead of selecting an arbitrary pair:

```sql
SELECT * FROM mdm_steward.decide(
    entity_name => 'customer',
    left_source_record_id => :'left_record'::uuid,
    right_source_record_id => :'right_record'::uuid,
    decision => 'NOT_MATCH',
    expected_version => :decision_version,
    reason => 'Reviewed source evidence identifies two distinct customers'
);
SELECT mdm.refresh('customer', 'ALLOW');
```

Retain the returned `decision_id`, `decision_version`, and `decision_epoch` in your stewardship workflow. A first action uses version 0 only when no previous decision exists; revisiting the pair does not reset its history. On a version conflict, show the latest state to the reviewer and obtain a new decision against that state. Automatically incrementing the expected version would discard the purpose of optimistic concurrency.

## Override a golden value

Choose an active source record in the intended entity as the anchor, name a field already configured for golden output, and pass a JSON value of the correct type. `to_jsonb('Acme Trading Limited'::text)` is a string value; JSON `null` is not a valid golden override. The first directive for that anchor and field uses expected version 0, while subsequent overrides and clears use the current override version.

```sql
SELECT * FROM mdm_steward.override_golden(
    'customer', :'anchor_record'::uuid, 'name',
    to_jsonb('Acme Trading Limited'::text), :override_version,
    'Name verified against the customer record'
);
SELECT mdm.refresh('customer', 'ALLOW');
```

Use `mdm_steward.clear_golden_override()` with the current version and a reason to return to the configured source policy, then refresh again. The anchor matters when memberships change: an override follows its source-record context, and an inactive anchor or conflicting directives can produce a review. [Inspect and refine a golden record](../tutorials/inspect-and-refine.md) is a complete exercise with actual identifiers and observable before-and-after results.

## Publish, then verify

A successful steward call records the instruction; it does not guarantee that a new publication has already been produced. Refresh the entity, inspect the relevant customer and membership rows, review any new open issues, and compare pair evidence at the resulting revision when investigating a match. The `pending_stewardship` information in the summary or policy projection helps distinguish a saved instruction from one reflected in a publication. Avoid editing the public output directly: the next publication is derived from source evidence and durable instructions, not from ad hoc output-table changes.

For queue assignment, due dates, and escalation rather than entity identity, use [Automate stewardship controls](automate-stewardship.md). Those case controls have their own version and receipt semantics and should not be confused with a must-link decision or a golden-value override.
