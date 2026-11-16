# Inspect and refine a golden record

[Documentation home](../README.md) · Tutorial 2 · About 20 minutes

In the [first lesson](first-entity.md), you combined two Acme records and watched a source edit change the published name without changing the customer ID. Now imagine that a steward has checked the customer's registered name and wants applications to display it while the source systems catch up. You will inspect the matching evidence and configured value policy, record an explicit override with a reason, publish it, and then clear the override to return to the source policy. Along the way, you will see why recording an instruction and publishing a result are separate steps.

## Before you start

Use the same disposable `pg_mdm_tutorial` database after completing the first lesson. Acme should currently be named `Acme Trading Ltd`, its email should be `accounts@acme.example`, and the latest publication revision should be 2. Open a new administrator `psql` session if you closed the previous one. With Docker, use `docker exec -it pg-mdm-tutorial psql -X -U postgres -d pg_mdm_tutorial`; keep the session open for the whole exercise because it stores several identifiers in `psql` variables.

Enable predictable output, return to the installation administrator, and grant only the two new steward actions used by this lesson:

```sql
\set ON_ERROR_STOP on
\pset format unaligned
\pset fieldsep |
\pset footer off
RESET ROLE;
RESET SESSION AUTHORIZATION;

GRANT USAGE ON SCHEMA mdm_steward TO tutorial_admin;
GRANT EXECUTE ON FUNCTION mdm_steward.override_golden(text, uuid, text, jsonb, bigint, text)
    TO tutorial_admin;
GRANT EXECUTE ON FUNCTION mdm_steward.clear_golden_override(text, uuid, text, bigint, text)
    TO tutorial_admin;

SET SESSION AUTHORIZATION tutorial_login;
SET ROLE tutorial_admin;
```

These grants let our existing action role record an override. They do not give it permission to edit protected internal tables. In a real installation you would review the same role boundary before allowing a user-facing stewardship application to expose these actions.

## 1. Find the customer and its source anchor

An override attaches to a source record, so it can follow that record through later identity changes. Query the CRM member of Acme and capture both its source-record identifier and its current customer identifier:

```sql
SELECT m.source_record_id AS anchor, c.mdm_id AS customer_id
FROM mdm_out.customer AS c
JOIN mdm_out.customer_members AS m USING (mdm_id)
WHERE c.email = 'accounts@acme.example'
  AND m.source_name = 'crm'
  AND m.active
\gset acme_

SELECT c.name, m.source_name, m.active
FROM mdm_out.customer AS c
JOIN mdm_out.customer_members AS m USING (mdm_id)
WHERE c.mdm_id = :'acme_customer_id'::uuid
ORDER BY m.source_name;
```

Expect the two source memberships below. `\gset` stores the identifiers without printing them; the concrete UUIDs differ between fresh databases.

```text
name|source_name|active
Acme Trading Ltd|crm|t
Acme Trading Ltd|erp|t
```

## 2. Inspect the matching evidence and value policy

Find Acme's billing source-record ID, then ask for an explanation of the pair at the current publication revision. Pair explanations are the detailed evidence surface supported by this version. The API also accepts a golden-value subject, but currently returns an empty fact list for it; inspect the published values and definition when reviewing a value policy.

```sql
SELECT source_record_id AS anchor
FROM mdm_out.customer_members
WHERE mdm_id = :'acme_customer_id'::uuid AND source_name = 'erp' AND active
\gset billing_

SELECT (mdm.describe('customer')->'publication'->>'publication_revision')::bigint AS revision
\gset before_

SELECT jsonb_pretty(mdm.explain(
    entity_name => 'customer',
    subject => jsonb_build_object(
        'kind', 'pair', 'left_id', :'acme_anchor', 'right_id', :'billing_anchor'),
    publication_revision => :before_revision
));

SELECT jsonb_pretty(mdm.describe('customer', 'definition')->'golden_values');
```

Read the pair's returned facts alongside the `same_email` rule from the first lesson. Then inspect `golden_values`: both fields use `prefer_source`, and CRM precedes ERP in the definition's canonical source-name order. Together, the pair evidence and configured value policy explain the two decisions you made: why these source records belong together, and how their published values are chosen. The response contains database-specific identifiers, so inspect its facts rather than comparing the whole JSON byte for byte. The `truncated` flag tells you when a bounded explanation omits further pair facts.

## 3. Record a reviewed name

The registered name for this exercise is `Acme Trading Limited`. Record that value against the CRM source anchor with `expected_version => 0`, meaning no previous override exists for this anchor and field. Capture the returned version so that clearing the override later uses the actual version written by the server:

```sql
SELECT override_version
FROM mdm_steward.override_golden(
    entity_name => 'customer',
    anchor_source_record_id => :'acme_anchor'::uuid,
    field_name => 'name',
    value => to_jsonb('Acme Trading Limited'::text),
    expected_version => 0,
    reason => 'Registered company name checked for this tutorial'
)
\gset saved_

SELECT name FROM mdm_out.customer
WHERE mdm_id = :'acme_customer_id'::uuid;
```

The name is still `Acme Trading Ltd`. You have saved a durable instruction, but readers continue to see the previous complete publication. The override did not update either source table, and it did not publish half of a new customer result.

## 4. Publish and verify the override

Refresh the entity, then compare the result with the customer ID you captured at the start:

```sql
SELECT mdm.refresh('customer', 'ALLOW')->>'changed' AS changed;

SELECT name, member_count, mdm_id = :'acme_customer_id'::uuid AS same_id
FROM mdm_out.customer
WHERE email = 'accounts@acme.example';
```

Expect `changed` to be `true`, followed by:

```text
name|member_count|same_id
Acme Trading Limited|2|t
```

The published name now reflects the override you recorded, and its returned directive version gives your application an audit reference. The customer still has two members and the same identity because you changed a value-selection instruction, not the matching evidence. The explicit publication revision captured in step 2 continues to identify the earlier pair evidence; changing the current output does not rewrite that retained observation.

## 5. Return to the source policy

Clear the directive using its saved version and a reason, then refresh again. This is another versioned action: it preserves the instruction's history instead of deleting a row from an internal table.

```sql
SELECT override_version
FROM mdm_steward.clear_golden_override(
    entity_name => 'customer',
    anchor_source_record_id => :'acme_anchor'::uuid,
    field_name => 'name',
    expected_version => :saved_override_version,
    reason => 'Tutorial complete; return to the configured source policy'
);

SELECT mdm.refresh('customer', 'ALLOW')->>'changed' AS changed;

SELECT name, member_count, mdm_id = :'acme_customer_id'::uuid AS same_id
FROM mdm_out.customer
WHERE email = 'accounts@acme.example';
```

The new override version is 2, `changed` is `true`, and the customer is once again `Acme Trading Ltd`, with two members and `same_id = true`. You have completed the full stewardship cycle: inspect evidence, record a reasoned instruction, publish it, verify its effect, and explicitly return control to the automatic policy.

If you repeat the lesson without resetting its database, version 0 will no longer be correct; clearing a directive does not erase its version history. Use a fresh learning environment, or retain and reuse the returned directive versions as described in [Steward records](../how-to/steward-records.md). When you are ready to apply this pattern to your own data, continue with [Model sources](../how-to/model-sources.md) and [Preview a definition change](../how-to/change-a-definition.md).
