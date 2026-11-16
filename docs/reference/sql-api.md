# SQL API reference

[Documentation home](../README.md) · Reference

This page covers the public `mdm`, `mdm_admin`, and `mdm_steward` functions in the current 0.15.0 SQL archive. Signatures use PostgreSQL argument types and defaults; they are reference notation, not standalone executable statements. Call a function with `SELECT`, and grant only the actions required by the calling role. The [security model](../explanation/security.md) explains why both the authenticated login and selected role matter.

Core actions operate on a named entity through its bound execution role. Constructors build JSON definitions. Administrative and steward actions require separate schema and function grants. Functions in `mdm_internal` and `mdm_graph` are implementation surfaces; their presence in the extension does not make them an application API.

## Definition constructors

### mdm.source()

```text
mdm.source(
    name text,
    relation regclass,
    source_id text[],
    mode text,
    fields jsonb,
    row_changed_at text DEFAULT NULL,
    soft_delete_when jsonb DEFAULT NULL,
    authority jsonb DEFAULT '{}'::jsonb
) RETURNS jsonb
```

Builds a source declaration. `source_id` is a nonempty array of key columns; `mode` is `tracked` or `soft_delete`; `fields` maps logical field names to physical column names. A soft-delete source requires a supported `soft_delete_when` object. `row_changed_at` names a source timestamp column, and `authority` is an object with string values. Constructors return JSON; registration and source-contract validation happen when the full entity is created. See [source contracts](definitions.md#source-contracts).

### mdm.field()

```text
mdm.field(
    name text,
    type text,
    cleaner text,
    cleaner_options jsonb DEFAULT '{}'::jsonb,
    display text DEFAULT 'masked'
) RETURNS jsonb
```

Builds a logical field. Supported cleaners and logical types are listed in [Definition reference](definitions.md#cleaners). `display` is `full`, `masked`, or `hashed` and defaults to `masked`. `cleaner_options` is a JSON object; the current cleaner option is `max_length`. Display policy is not access control for raw golden output columns.

### mdm.match()

```text
mdm.match(
    name text,
    fields text[],
    comparison text,
    strength text,
    evidence_group text,
    threshold integer DEFAULT NULL,
    candidate jsonb DEFAULT NULL
) RETURNS jsonb
```

Builds a match rule. `comparison` is `exact`, `fuzzy`, or `normalized_levenshtein`; `fuzzy` is an alias for the normalized Levenshtein comparator. `strength` is `identity`, `strong`, or `supporting`. Exact comparison does not accept a threshold; fuzzy comparison requires an integer threshold from 0 through 10000. Strong and non-exact identity rules require candidates, supporting rules cannot own candidates, and an exact identity rule can have its candidate inferred. See [candidate predicates](definitions.md#candidate-predicates).

### mdm.golden_value()

```text
mdm.golden_value(
    field text,
    policy text,
    sources text[] DEFAULT NULL
) RETURNS jsonb
```

Selects the policy for one published field. Policies are `prefer_source`, `latest`, `first_non_null`, and `most_common`. `prefer_source` requires a nonempty `sources` list. In the current implementation that list filters eligibility; source priority comes from canonical source-name order, not the list order. Only declared golden fields become value columns in the public entity table.

### mdm.entity()

```text
mdm.entity(
    name text,
    sources jsonb[],
    fields jsonb[],
    matches jsonb[],
    golden_values jsonb[],
    preset text DEFAULT NULL,
    limits jsonb DEFAULT '{}'::jsonb,
    execution_role text DEFAULT NULL
) RETURNS jsonb
```

Builds and locally validates the complete entity JSON. `sources`, `fields`, and `matches` must be nonempty, and names within each category must be unique. `preset` accepts `person`, `company`, or `product` and pins metadata to version 1; it does not fill in your domain-specific definition. `limits` contains supported positive bounds. `execution_role`, when supplied, must satisfy the current caller and binding checks; normal usage leaves it unset and uses the selected execution role.

## Core entity actions

### mdm.create()

```text
mdm.create(
    definition jsonb,
    expected_version bigint DEFAULT NULL,
    comment text DEFAULT NULL
) RETURNS TABLE (
    operation_id uuid,
    entity_name text,
    desired_version bigint,
    changed boolean,
    definition_digest bytea,
    artifact_digest bytea
)
```

Creates a new entity or records a new desired definition. For a new entity, leave `expected_version` null. For an existing entity, pass its current **desired** version, including for a no-op. A canonical definition change increments the desired version; an identical definition returns `changed = false`. The definition and compiled artifact digests are returned as `bytea`. Creation does not publish the output: preview the desired definition and refresh to activate it. See [Change a definition](../how-to/change-a-definition.md).

### mdm.describe()

```text
mdm.describe(
    entity_name text,
    format text DEFAULT 'summary'
) RETURNS jsonb
```

Returns either a `summary` object or the expanded `definition`. The summary includes desired and active versions, execution role, digests, selected semantics, graph state, source contracts, decisions, and publication metadata. The action requires the entity's bound execution role. The latest refresh diagnostics are nested under `publication.refresh`.

### mdm.preview()

```text
mdm.preview(
    entity_name text,
    mode text DEFAULT 'validation',
    options jsonb DEFAULT '{}'::jsonb
) RETURNS jsonb
```

Examines the desired definition without publishing it. `validation` accepts an empty options object. `sampled` accepts only `sample_size`, an integer from 1 through 1000, default 25. `scoped` accepts `source_record_ids` and/or `mdm_ids`, each an array of distinct UUID strings, with at least one nonempty array. Data-bearing previews use evidence from the last successful publication; later source changes may be pending. A scoped result can be exact for its selected subjects while omitted records still change the full-entity result. Read the returned `data_as_of`, evidence level, and limitations.

### mdm.refresh()

```text
mdm.refresh(
    entity_name text,
    full_policy text DEFAULT 'ALLOW'
) RETURNS jsonb
```

Refreshes the graph, resolves entities, and publishes a complete result transactionally. `full_policy` is `ALLOW` or `ERROR`; use `ALLOW` for normal operation, including bootstrap, transitions, rebuilds, and recovery. `ERROR` is a qualification constraint for controlled steady-state checks. The JSON result contains publication and diagnostic metadata described in [Output reference](outputs.md#refresh-result). A successful no-change refresh can retain the prior publication revision.

### mdm.explain()

```text
mdm.explain(
    entity_name text,
    subject jsonb,
    publication_revision bigint DEFAULT NULL,
    max_facts integer DEFAULT 100
) RETURNS jsonb
```

Returns a bounded object containing `facts`, `truncated`, `identity`, and `retained_revision_range`. **Detailed retained facts are currently implemented for pair subjects.** An `mdm_id` subject reports current identity status; accepted `source_record`, `golden`, `review`, and `publication` subjects currently return empty fact lists. Supply `publication_revision` explicitly when investigating retained pair evidence; the accepted argument does not make the other subject kinds historical reconstructions. See the exact [subject shapes and limits](outputs.md#explanation-subjects).

## Installation and lifecycle actions

### mdm_admin.verify_installation()

```text
mdm_admin.verify_installation() RETURNS text
```

Checks the protected helper owner, caller safety, and required producer capabilities. It returns the committed operation UUID **as text**, not the literal string `MDM_OK`; that is the success code recorded with the operation. Ordinary caller rules apply: both authenticated and selected roles must be safe. The installation administrator grants execution to the action role after running `configure_helper.sql`.

### mdm_admin.recompile()

```text
mdm_admin.recompile(
    entity_name text
) RETURNS jsonb
```

Recompiles and reinstalls the current desired definition's graph artifacts for the current compiler and producer contract. It does not replace the desired definition with a new business policy. Run a subsequent refresh and verify output semantics. This action needs an explicit administrator-schema function grant and the entity's execution role; see [Compiler rollout](../how-to/operate.md#roll-out-a-compiler-or-producer-change).

### mdm_admin.rebuild()

```text
mdm_admin.rebuild(
    entity_name text,
    full_policy text DEFAULT 'ALLOW'
) RETURNS jsonb
```

Runs the full resolver path while retaining the durable identity ledger. It returns refresh JSON and should normally use `ALLOW`. A full rebuild is useful for recovery and comparison with affected evaluation; it is not equivalent to dropping and recreating the entity.

### mdm_admin.rebind()

```text
mdm_admin.rebind(
    entity_name text
) RETURNS jsonb
```

Reconstructs derived role and source-relation bindings after logical restore or a planned compatible replacement. This action requires a **superuser-authenticated session** with the stored execution role selected, unlike ordinary entity actions. It validates portable names and retained key contracts without rewriting definitions, artifacts, or digests. Follow [Restore and rebind](../how-to/operate.md#restore-and-rebind).

### mdm_admin.drop_entity()

```text
mdm_admin.drop_entity(
    entity_name text,
    confirm text
) RETURNS jsonb
```

Destructively removes the entity through its lifecycle operation. `confirm` must equal `entity_name`, and the caller must be its execution role with the explicit function grant. Review dependent consumers and preserve required history before using it; this is not a refresh or rebuild mechanism.

## Identity and value stewardship

### mdm_steward.decide()

```text
mdm_steward.decide(
    entity_name text,
    left_source_record_id uuid,
    right_source_record_id uuid,
    decision text,
    expected_version bigint,
    reason text
) RETURNS TABLE (
    operation_id uuid,
    decision_id uuid,
    decision_version bigint,
    decision_epoch bigint
)
```

Records a versioned manual pair instruction using **source-record UUIDs**. Accepted strings are uppercase `MATCH` and `NOT_MATCH`; this version has no public pair-decision clearing action. Use expected version 0 only when the pair has no prior decision, otherwise use the retained current decision version. `reason` must contain non-whitespace text. Returns the operation, decision identifier, new decision version, and decision epoch. A later refresh publishes its effect. A review concurrency version is not a decision version.

### mdm_steward.override_golden()

```text
mdm_steward.override_golden(
    entity_name text,
    anchor_source_record_id uuid,
    field_name text,
    value jsonb,
    expected_version bigint,
    reason text
) RETURNS TABLE (
    operation_id uuid,
    override_id uuid,
    override_version bigint,
    decision_epoch bigint
)
```

Records a golden directive anchored to a source record and field. The JSON value must be compatible with the configured field; JSON null is rejected. Use expected version 0 for the first directive, otherwise the current override version, and provide a nonblank reason. The directive and its returned version survive a later clear. Refresh to publish the new selection. See [the complete override tutorial](../tutorials/inspect-and-refine.md).

### mdm_steward.clear_golden_override()

```text
mdm_steward.clear_golden_override(
    entity_name text,
    anchor_source_record_id uuid,
    field_name text,
    expected_version bigint,
    reason text
) RETURNS TABLE (
    operation_id uuid,
    override_id uuid,
    override_version bigint,
    decision_epoch bigint
)
```

Appends a clearing directive for the anchor and field, using the current override version and a nonblank reason. It returns the new override version rather than deleting the history. Refresh afterward to return selection to the configured policy, subject to any other applicable directives.

## Policy functions

### mdm_admin.create_policy_binding()

```text
mdm_admin.create_policy_binding(
    entity_name text,
    automation_role_name text,
    policy_digest bytea,
    allowed_actions text[],
    allowed_queues text[],
    max_due_interval interval,
    max_escalation_level integer
) RETURNS TABLE (
    binding_id uuid,
    binding_version bigint
)
```

Binds a separate safe automation role to an entity, a 32-byte policy digest, sorted distinct allowed actions and queues, and action-specific bounds. Supported actions are `ASSIGN_QUEUE`, `ESCALATE`, and `SET_DUE_AT`. A new binding starts active in the current runtime and requires pg_mdm preload. The helper grants the worker reads on the policy projection and intent execution. See [Automate stewardship](../how-to/automate-stewardship.md).

### mdm_admin.replace_policy_binding()

```text
mdm_admin.replace_policy_binding(
    binding_id uuid,
    expected_version bigint,
    policy_digest bytea,
    allowed_actions text[],
    allowed_queues text[],
    max_due_interval interval,
    max_escalation_level integer
) RETURNS TABLE (
    binding_id uuid,
    binding_version bigint
)
```

Uses the existing binding version to retain and replace a binding with a new identifier and approved policy scope. The new binding starts paused; retain the returned ID/version and explicitly activate it after checking the environment. The old binding and receipt history remain available. Binding version is distinct from runtime version and case action revision.

### mdm_admin.set_policy_binding_state()

```text
mdm_admin.set_policy_binding_state(
    binding_id uuid,
    expected_runtime_version bigint,
    state text,
    reason text
) RETURNS bigint
```

Sets a binding's runtime state to `active` or `paused` and returns the new runtime version. Supply the actual expected runtime version, or 0 when recreating a missing runtime row. Activation checks the current database, roles, capabilities, and postmaster activation epoch; it requires pg_mdm preload. Revalidate and activate after restart, restore, or clone. Pausing remains available without preload.

### mdm_steward.set_case_controls()

```text
mdm_steward.set_case_controls(
    case_key bigint,
    assigned_queue name,
    due_at timestamptz,
    escalation_level integer,
    manual_assignment_protected boolean,
    expected_action_revision bigint,
    reason text
) RETURNS TABLE (
    operation_id uuid,
    action_revision bigint
)
```

Applies a human control edit to the case using its current action revision and a reason. The arguments describe the complete queue, due date, escalation level, and manual-assignment protection state to write. Returns the operation and new action revision. This changes case controls, not pair identity or golden-value policy.

### mdm_admin.backfill_policy_case_opened_at()

```text
mdm_admin.backfill_policy_case_opened_at(
    case_key bigint,
    opened_at timestamptz,
    reason text
) RETURNS TABLE (
    operation_id uuid,
    action_revision bigint
)
```

Administratively supplies a supported opening time for an eligible historical case whose opening time is unknown, with a reason. This is an audited correction with its own action revision, not an automated inference from the current time. The implementation rejects inappropriate timestamps, repeated backfills, and missing cases.

### mdm_steward.submit_policy_intent()

```text
mdm_steward.submit_policy_intent(
    binding_id uuid,
    request_key bytea,
    case_key bigint,
    action text,
    arguments jsonb,
    expected_review_version bigint,
    expected_definition_version bigint,
    expected_publication_revision bigint,
    expected_stewardship_epoch bigint,
    expected_evidence_basis_digest bytea,
    expected_action_revision bigint,
    expected_policy_digest bytea,
    policy_revision text,
    evaluation_ref text,
    work_ref text
) RETURNS TABLE (
    receipt_id uuid,
    outcome text,
    reason_code text,
    case_key bigint,
    action_revision bigint,
    control jsonb,
    resulting_publication_revision bigint
)
```

Submits an automation action using a binding, stable 32-byte request key, the case snapshot's expected versions and 32-byte evidence/policy digests, and policy/evaluation/work references. It returns a receipt result. Identical request replay returns the stored receipt. Same-key changed-body replay returns `IDEMPOTENCY_CONFLICT` / `REQUEST_KEY_BODY_MISMATCH` with a null receipt ID; it does not replace the original receipt. Malformed or unauthorized requests can raise SQL errors without a receipt. `APPLIED_CONTROL` is distinct from a later publication. See [receipt semantics](outputs.md#policy-relations).

## Error handling

Failures use stable `MDM_*` codes alongside explanatory messages. Handle a version conflict by rereading and reconciling the relevant state, a limit error by examining the workload and definition, and a capability or boundary failure by correcting the installation or recovery state. Do not reinterpret failed evaluation as a negative match. The [troubleshooting guide](../how-to/troubleshoot.md) groups common codes by task; the complete code mapping is maintained in [src/error.rs](../../src/error.rs).

The signatures here are taken from [sql/archive/pg_mdm--0.15.0.sql](../../sql/archive/pg_mdm--0.15.0.sql), with behavior checked against [src/api](../../src/api/) and the current tests. For another installed release, use that release's archive and documentation rather than assuming its defaults and result shape are identical.
