# pg-trickle changes required by pg-mdm incremental refresh

## Scope

**Status:** Fulfilled by pg-trickle v0.108.0; retained as the contract and evidence checklist.

pg-mdm 0.13.0 admits pg-trickle 0.108.0 at commit
`8bd0a4b5eb3e586ebdeea56bd774611aa7907e25`. pg-mdm resolves and publishes
affected identity components. Remaining `AUTO` and `FULL` execution is an
intentional bootstrap or recovery fallback.

This file records the pg-trickle work and Delta V1 release evidence used for
pg-mdm's steady-state mutation and recovery qualification.

## Required changes

| Area | pg-trickle 0.106.1 behavior | Required result |
|---|---|---|
| Row identity | `pgtrickle.encode_row_id_v2()` is `STABLE` | Differential admission recognizes only this extension-owned function and signature as a safe stable primitive |
| LATERAL validation | An immutable one-row composite function is rejected because validation parses `SELECT function(...)` as a defining query without a `FROM` clause, and custom `RETURNS TABLE` columns are not inferred | Validate the expression and resolve its declared output columns |
| Dependent nodes | Evidence inherits the unsupported record and normalization paths | Record, normalization, and evidence nodes pass `DIFFERENTIAL` admission |
| Graph capability | Graph V1 1.1 does not promise these query shapes | Graph V1 advertises a new minor version only after the shapes pass conformance |
| Delta recovery | `INVALIDATED` is declared but never entered; resnapshot tokens fence only the log head; clone and restore instance changes are not checked | Add public validation, resnapshot requests, complete fencing, and a constrained qualification hook |

## Admit the exact V2 row identity encoder

pg-mdm record nodes call:

```sql
pgtrickle.encode_row_id_v2(
    'MDM_SOURCE_KEY_V1',
    ROW(entity_id, source_identity_id, source_key_columns...)
)
```

The encoder is declared `STABLE` in pg-trickle 0.106.1. A forced
`DIFFERENTIAL` registration fails with this reason:

```text
STABLE: stable expressions have refresh-dependent volatility and can change
between refreshes. Use FULL or AUTO.
```

The encoder reads PostgreSQL type and collation metadata, so changing its
volatility declaration is not the safe default. Resolve function calls after
PostgreSQL analysis and admit only the extension-owned function with this
identity:

```text
pgtrickle.encode_row_id_v2(text, anyelement)
```

Match by function OID and resolved signature. Continue rejecting every other
`STABLE` or `VOLATILE` function, including a same-named function in another
schema. Include the row-identity version and relevant input type metadata in
the stream contract so a schema or encoder change changes the contract
digest.

Add these pg-trickle checks:

- Assert that only the resolved extension-owned encoder bypasses the ordinary
  `STABLE` rejection and remains `PARALLEL SAFE`.
- Compare its complete byte output with the existing Row Identity V2 fixtures
  for every supported type, null, collation, and error case.
- Create a `DIFFERENTIAL` stream table whose projection contains the encoder.
- Verify exact insert, update, delete, rollback, and retry results against a
  separately rebuilt `FULL` table.
- Verify that an unsupported type or nondeterministic collation still fails at
  registration. It must not become a runtime fallback.

## Accept immutable one-row LATERAL functions

pg-mdm normalization nodes use this shape:

```sql
FROM records AS r
CROSS JOIN LATERAL mdm_graph.normalize_text(
    r.raw_value,
    'email',
    1,
    r.source_state,
    '{}'::jsonb
) AS n
```

`mdm_graph.normalize_text()` and `mdm_graph.normalize_date()` are
`IMMUTABLE PARALLEL SAFE` functions. Each returns one composite row. In
pg-trickle 0.106.1, `check_ivm_support_inner()` wraps the function body as
`SELECT <function call>` and sends it to `parse_defining_query_full()`. That
parser requires a `FROM` clause, so admission fails before the existing
row-scoped LATERAL operator can run.

Change LATERAL function handling so it inspects a function expression without
treating it as a complete defining query. For a `RangeFunction` without an
explicit column list, resolve `OUT` and `TABLE` columns from `pg_proc`,
including `proallargtypes`, `proargmodes`, and `proargnames`. The validator and
operator must:

1. Parse the function expression with PostgreSQL's raw parser or an equivalent
   expression parser.
2. Resolve every called function and reject `VOLATILE` or unresolved calls.
3. Accept an immutable function that returns one composite row and retain its
   declared output names and types in order.
4. Preserve an explicit alias and column list when supplied.
5. Keep the existing row-scoped recomputation path for the changed outer row,
   emitting exact delete and insert rows when the result changes or
   disappears.
6. Preserve current rejection of unsupported nested SQL, unsafe functions,
   `RIGHT JOIN LATERAL`, and `FULL JOIN LATERAL`.

Add a regression test for the exact pg-mdm normalization shape. For insert,
update, delete, soft delete, reactivation, rollback, and retry, compare every
row and column with a `FULL` reference table.

## Qualify the dependent pg-mdm graph shapes

After the two changes above, run the generated pg-mdm compiler-version-9 SQL
as pg-trickle fixtures. Cover these node families in dependency order:

1. `records/<source>` for scalar and composite source keys.
2. `normalized/<field>` for text, email, date, and null-state inputs.
3. Candidate block, block-stat, block-overflow, pair, pair-stat, and
   pair-overflow nodes for exact, prefix, token, and composite channels.
4. `evidence/<entity>` for exact and bounded Levenshtein comparisons.
5. `golden/<entity>` with its minimal direct dependency list.

The exact-email probe against pg-trickle 0.106.1 already admits the golden
query in `DIFFERENTIAL` mode. Keep that as a regression. Do not add a new
operator or a barrier node for golden.

For every fixture, the pg-trickle mutation matrix must prove:

- bootstrap uses `FULL`;
- steady-state insert, update, delete, soft delete, and reactivation use
  `DIFFERENTIAL`;
- pair additions and removals use `DIFFERENTIAL`;
- a no-change refresh writes no rows;
- overflow failures roll back all graph members;
- retry produces the same rows as a clean run;
- `refresh_graph_strict(..., full_policy => 'ERROR')` succeeds after bootstrap;
- every terminal relation is byte-for-byte equal to a separately rebuilt
  `FULL` graph after each mutation.

## Publish a Graph V1 capability boundary

Do not let pg-mdm infer the new support from a pg-trickle release number.
After the fixtures above pass:

1. Increment `external_graph_refresh` from 1.1 to 1.2.
2. Add these stable feature identifiers to its capability details:

   ```json
   {
     "differential_features": [
       "stable_row_identity_encoder_v2",
       "custom_table_srf_out_columns",
       "lateral_immutable_composite_function"
     ]
   }
   ```

3. Update `docs/capability-manifest.json` with the new minor version and the
   conformance command that proves both features.
4. Keep Graph V1 major version 1. The SQL signatures and transaction rules do
   not need a breaking change.

pg-mdm will require Graph V1 1.2 before compiling record, normalization, and
evidence nodes as `DIFFERENTIAL`. Older pg-trickle releases will keep the
current `AUTO` graph.

## Complete Delta V1 recovery

Add an owner-authorized, `SECURITY DEFINER`, fixed-search-path request API:

```sql
pgtrickle.request_output_delta_resnapshot(consumer_id uuid)
RETURNS TABLE (
    state text,
    state_reason text,
    acknowledged_batch_token bigint,
    log_head bigint,
    output_contract_digest bytea,
    row_identity_version smallint
)
```

`ACTIVE` and `PAUSED` consumers become `RESNAPSHOT_REQUIRED` with reason
`ADMIN_REQUESTED`. Calls for `RESNAPSHOT_REQUIRED` or `INVALIDATED` consumers
are idempotent. Preserve the cursor, log, batches, and payload, delete an
abandoned resnapshot token, enforce owner or superuser authorization, and use
normal transaction rollback.

Add a validating status API:

```sql
pgtrickle.validate_output_delta_consumer(consumer_id uuid)
RETURNS TABLE (
    consumer_id uuid,
    delta_relation text,
    state text,
    state_reason text,
    acknowledged_batch_token bigint,
    log_head bigint,
    batch_lag bigint,
    output_contract_digest bytea,
    row_identity_version smallint,
    database_instance_id text
)
```

Under a lock, validate the consumer, log, current stream contract, every
pending batch, and typed payload. Recoverable failures must persist
`INVALIDATED` with one of these stable reasons instead of raising an error
that rolls back the state transition:

```text
DELTA_GAP
PAYLOAD_INCONSISTENT
CONTRACT_MISMATCH
ROW_IDENTITY_VERSION_MISMATCH
DATABASE_INSTANCE_CHANGED
```

The validation must prove contiguous tokens, matching instance IDs, contract
digests and row-identity versions, consistent counts, zero payload for
`FULL_INVALIDATION`, exact typed payload cardinality for `EXACT`, contiguous
ordinals, and only `DELETE` or `INSERT` actions. Batch reads and both
acknowledgement functions must repeat their relevant checks under lock.

Extend resnapshot records to fence the log head, database instance ID,
contract digest, and row-identity version. Acknowledgement must reject a stale
token if any fenced value changed. On successful clone or restore adoption,
invalidate every non-dropped consumer with `DATABASE_INSTANCE_CHANGED`,
discard outstanding resnapshot tokens, and preserve cursor and log data until
a new baseline completes.

Finally, make exactness an executor guarantee. The refresh executor must pass
an explicit complete-capture result to the output finalizer. Emit `EXACT` only
when every output deletion and insertion was captured. Emit
`FULL_INVALIDATION` for FULL execution or any path without that proof.

## Add a constrained qualification hook

pg-mdm cannot create a real gap or mismatch without writing pg-trickle's
private catalogs. Add this disabled-by-default, superuser-only API:

```sql
pgtrickle.qualify_output_delta_recovery(consumer_id uuid, scenario text)
RETURNS text
```

Enable it only with a postmaster-level qualification GUC. Accept exactly
`FULL_INVALIDATION`, `DELTA_GAP`, `CONTRACT_MISMATCH`, and `INVALIDATED`.
Keep changes transactional. The invalidation case appends a valid zero-row
batch; the other cases make resnapshot mandatory with the matching stable
reason. Do not accept arbitrary SQL, identifiers, digests, or catalog writes.
Upstream tests must still corrupt private state directly to prove detection;
this hook exists only for downstream conformance.

## Publish Delta V1 recovery conformance

No unrestricted production fault-injection API is required. pg-trickle owns
the output log and must test its private invariants directly. The constrained
qualification hook above supplies only the downstream scenarios that public
APIs cannot otherwise produce. Add named release conformance tests that prove
the public behavior below:

| Case | Required public result |
|---|---|
| Non-pristine registration | The consumer starts in `RESNAPSHOT_REQUIRED` |
| Resnapshot | `begin_output_delta_resnapshot()` and `ack_output_delta_resnapshot()` activate the consumer at the captured log head |
| Exact batches | Tokens are contiguous and `APPLIED` advances the cursor |
| Full invalidation | The batch has no payload rows and only `RESYNCHRONIZED` can acknowledge it |
| Transaction rollback | Rolling back after acknowledgement leaves both the cursor and pending batches unchanged |
| Contract mismatch | Validation persists `INVALIDATED` with `CONTRACT_MISMATCH` and requires resnapshot |
| Row-identity version change | The consumer becomes `INVALIDATED` or `RESNAPSHOT_REQUIRED` before it can read incompatible rows |
| Internal token gap | Validation persists `INVALIDATED` with `DELTA_GAP`; batch reads never return a partial range |
| Payload mismatch | Batch counts, actions, typed columns, digest, and row-identity version remain internally consistent |

Add each test name and command to the release capability manifest. Increment
`output_delta_consumer` from 1.0 to 1.1 because the recovery APIs and semantics
are additive public contract changes. Advertise these details only after the
packaged conformance suite passes:

```json
{
  "consumer_recovery_version": 1,
  "public_resnapshot_request": true,
  "public_consumer_validation": true,
  "qualification_api": "qualify_output_delta_recovery",
  "typed_delta_encoding_version": 1
}
```

## Release and upgrade requirements

The pg-trickle release must include:

- an upgrade from 0.106.1 that preserves consumers, cursors, batches, and
  payload while adding the resnapshot fencing fields and public functions;
- PostgreSQL 18 Linux artifacts used by pg-mdm E2E;
- an updated capability manifest with checksums;
- Graph V1 1.2 and Delta V1 1.1 conformance results;
- a package, upgrade, and archive check from the same commit as the artifact.

## pg-mdm adoption after release

After pg-trickle ships the changes, pg-mdm must:

1. Pin the new pg-trickle tag, commit, artifact, and checksum.
2. Require Graph V1 minor version 2, Delta V1 minor version 1, and the named
   capability details.
3. Issue compiler version 10 and change qualified node families from `AUTO` to
   `DIFFERENTIAL`.
4. Run the full mutation matrix with `full_policy = 'ERROR'`.
5. Recompile a canary entity without changing its semantic definition version.
6. Compare its complete publication with a full rebuild.
7. Recompile the remaining entities only after the canary latency and fallback
   checks pass.

Until then, pg-mdm must keep `full_policy = 'ALLOW'` in production and report
the affected resolver separately from graph-stage `FULL` work.
