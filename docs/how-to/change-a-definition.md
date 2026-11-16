# Preview and publish a definition change

[Documentation home](../README.md) · How-to guide

Use this procedure to change an entity's matching, cleaning, golden-value policy, or limits while keeping the currently published result available for readers. You need the entity's execution role, a complete proposed definition, and representative examples of the behavior you expect to change. pg_mdm keeps a desired definition separate from the active publication: `mdm.create()` records the proposal, previews examine it, and a successful refresh activates it and publishes its result.

## 1. Read the current state

Connect as the restricted application login, select the entity's existing execution role, and inspect both the summary and expanded definition. In `psql`, the final query below stores the desired version in `:current_desired_version`, so a subsequent write can reject a concurrent definition edit.

```sql
SELECT jsonb_pretty(mdm.describe('customer', 'summary'));
SELECT jsonb_pretty(mdm.describe('customer', 'definition'));
SELECT (mdm.describe('customer')->>'desired_version')::bigint AS desired_version
\gset current_
```

Save the existing definition and relevant public rows with your normal change-review artifacts. For an integration, persist the returned definition and version in its own change workflow. Do not assume the active version and desired version are equal: someone may already have registered a proposal that has not yet been published.

## 2. Register the complete proposed definition

Use the same full `mdm.entity(...)` construction you used to create the entity, with the intentional changes applied. Pass the previously observed `desired_version` as `expected_version` and a useful comment. New entities require a null expected version; existing entities require their current desired version even when the definition is unchanged. A canonical no-op returns `changed = false` and retains the definition version.

For a small runnable example on an entity whose current limits are lower, this statement changes just the candidate-pair limit by editing the expanded definition. It illustrates the version check without repeating a large constructor expression:

```sql
SELECT entity_name, desired_version, changed
FROM mdm.create(
    definition => jsonb_set(
        mdm.describe('customer', 'definition'),
        '{limits,max_candidate_pairs}',
        '2000000'::jsonb
    ),
    expected_version => :current_desired_version,
    comment => 'Raise the candidate-pair ceiling after reviewing workload evidence'
);
```

This example increases a resource bound; it is not a performance recommendation. For a matching change, keep the full definition in a reviewed SQL file so reviewers can see the evidence groups, candidate predicates, and output policies together. If a version conflict occurs, read the current definition and reconcile the actual changes rather than retrying with a guessed larger number.

## 3. Preview before publication

Begin with validation, then inspect a sample or a deliberately selected group of source records. Preview modes have different purposes and accepted options:

```sql
SELECT jsonb_pretty(mdm.preview('customer', 'validation'));
SELECT jsonb_pretty(mdm.preview('customer', 'sampled', '{"sample_size":25}'::jsonb));
```

For a scoped preview, obtain real UUIDs from the existing member or customer table and pass `source_record_ids` or `mdm_ids` in a JSON object. For example, this statement previews the customer currently identified by the tutorial email:

```sql
SELECT mdm.preview(
    'customer', 'scoped',
    jsonb_build_object('mdm_ids', jsonb_build_array(mdm_id::text))
)
FROM mdm_out.customer
WHERE email = 'accounts@acme.example';
```

Previews do not publish the desired result, and data-bearing previews use evidence from the last successful publication; later source changes may still be pending. A sample is a learning and review aid, not proof that every production record is safe. Scoped work may expand to include the closure needed for a coherent decision, but omitted records can still change the full-entity result. Inspect the returned `data_as_of`, limitations, warnings, and selected subjects before you proceed. The [SQL reference](../reference/sql-api.md#mdmpreview) gives exact option bounds.

## 4. Publish and inspect the result

Run the refresh as the execution role. Keep `ALLOW` so bootstrap, graph transitions, and recovery can use the complete full path when needed:

```sql
SELECT jsonb_pretty(mdm.refresh('customer', 'ALLOW'));
SELECT jsonb_pretty(mdm.describe('customer', 'summary'));
```

Check the active and desired versions, publication revision, output rows, review cases, source-boundary completeness, and any fallback diagnostics. If the refresh fails, the previous committed publication remains available; correct the proposal or its environment before retrying. When using an explicit transaction, remember that the result becomes visible to other sessions only when the transaction commits.

## Respect output and identity contracts

Existing golden output fields are append-only in their retained order and types. Removing, reordering, or changing the type of an existing published field is not an ordinary compatible definition edit. Source-key identity contracts and retained histories impose additional restrictions. Plan a separate migration when you need a different output contract, and use [Restore and rebind](operate.md#restore-and-rebind) only for the derived role and relation bindings it is designed to repair. Rebinding does not redefine a source key or erase an identity history.

**Next:** [Inspect outputs and refresh metadata](../reference/outputs.md), or [operate a canary rollout](operate.md#roll-out-a-compiler-or-producer-change).
