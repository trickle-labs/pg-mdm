# pg-mdm incremental improvements

**Status:** pg-mdm implementation complete; pg-trickle v0.108.0 admitted; Docker E2E and production rollout pending  
**Date:** 2026-09-21  
**Target:** pg-mdm 0.13.0, based on pg-trickle 0.108.0  
**Owners:** pg-mdm maintainers

All source, SQL, and test paths in this document are relative to the pg-mdm
repository root.

The remaining external work is specified in
[`PG_TRICKLE_INCREMENTAL_REQUIREMENTS.md`](PG_TRICKLE_INCREMENTAL_REQUIREMENTS.md).
pg-trickle 0.108.0 admits the compiler-version-10 graph nodes under
`full_policy = 'ERROR'`; this repository still cannot prove a production soak or
rollout without the target environment. That gate remains open.

## Goal

Make normal steady-state pg-mdm refreshes differential from source capture
through entity resolution, while preserving the current full path as the
correctness oracle and recovery path.

The target is not 100 percent differential execution. These cases must remain
full:

- the first graph population;
- graph replacement or contract change;
- a Delta V1 `FULL_INVALIDATION`, gap, or state that requires resnapshot;
- an unprovable affected set;
- a correctness or resource-limit fallback.

For a small insert, update, or delete after bootstrap, the target behavior is:

1. Synchronize only changed helper rows.
2. Refresh eligible pg-trickle nodes in `DIFFERENTIAL` mode.
3. Read exact terminal deltas through the public Delta V1 API.
4. Resolve only the affected identity components.
5. Publish the affected changes and acknowledge the consumed delta in the same
   transaction.

## Current state

pg-mdm already requests `DIFFERENTIAL` for candidate blocks, block statistics,
block overflow checks, pairs, pair statistics, and pair overflow checks in
`src/graph_spec.rs`. Source record nodes, normalized field nodes, evidence, and
golden-value nodes still request `AUTO`.

Two independent problems prevent end-to-end incremental behavior:

1. `src/api/refresh.rs::refresh_graph()` deletes and reinserts every row in
   `mdm_graph.source_identity_map`, `mdm_graph.source_records`, and
   `mdm_graph.definition_limits` before every graph refresh. This produces
   artificial CDC changes when the logical input is unchanged.
2. `src/api/refresh.rs::persist_refresh_inner()` loads all terminal rows and
   calls `evaluation::resolve_and_compare()` for the whole entity after every
   graph refresh. pg-mdm consumes the stable `output_delta_consumer` 1.1
   capability when the affected scope is exact.

Existing graph artifacts are immutable. Changing `src/graph_spec.rs` affects
newly compiled graph generations only. Existing entities need an explicit
recompile and graph-generation adoption path.

## Non-negotiable invariants

- The result of an affected refresh must equal a clean full refresh, including
  identities, memberships, aliases, splits, golden values, reviews, resolution
  facts, and the publication digest.
- The graph refresh, pg-mdm publication, Delta V1 acknowledgement, observation,
  and operation completion must commit or roll back together.
- Use only public `pgtrickle` SQL contracts. Do not read pg-trickle catalogs or
  change-buffer tables directly.
- Keep entity advisory locking in `persist_refresh_inner()`.
- Keep `full_policy = 'ALLOW'` as the production default. Use
  `full_policy = 'ERROR'` for qualification and regression tests.
- Do not acknowledge a batch that pg-mdm did not publish successfully.
- Do not weaken entity-wide limits when evaluating a subset.

## Delivery order

Implement the work in six delivery phases. Use a separate pull request for
phase 1, phase 3, phase 4, and phase 5. Split phase 2 by node family if its
qualification matrix produces a large diff. Do not begin the affected resolver
by rewriting the existing resolver. Reuse it on a proved scope and compare it
with the full path first.

## 1. Stop artificial helper-table churn

### Code changes

Change only `src/api/refresh.rs::refresh_graph()`.

1. Replace each unconditional entity-wide delete with a set-difference delete:

   - Delete an identity row only when its `(entity_id, source_name)` no longer
     exists in `mdm_internal.source_identities`.
   - Delete a source-record row only when its `source_record_id` no longer
     exists for the entity in `mdm_internal.source_records`.
   - Delete a definition-limit row only when the entity or desired definition
     no longer exists.

2. Change each insert to `INSERT ... ON CONFLICT ... DO UPDATE`.

3. Add an `IS DISTINCT FROM` predicate to every conflict update. PostgreSQL
   must not write a new row version when all values are unchanged.

4. Keep delete before upsert. This avoids a transient key conflict if a source
   replacement reuses an identity.

5. Do not extract a generic synchronization framework. The three statements
   have different keys and are readable in place.

The conflict keys are already indexed:

- `source_identity_map`: `(entity_id, source_name)`;
- `source_records`: `(source_record_id)`;
- `definition_limits`: `(entity_id)`.

Leave the delete-and-insert sequence in `src/api/create.rs::install_graph()`
unchanged. It runs during a new graph bootstrap, where a full population is
expected.

### Tests

Add one E2E case to `tests/e2e.sql`.

1. Create a temporary audit table and a `SECURITY DEFINER` trigger function as
   the PostgreSQL test administrator.
2. Attach `AFTER INSERT OR UPDATE OR DELETE` audit triggers to the three helper
   tables.
3. Run an unchanged second `mdm.refresh()` and assert that the audit table stays
   empty.
4. Insert one source row and refresh. Assert one helper `source_records` insert
   and no identity or definition-limit writes.
5. Delete the source row and refresh. Assert one `source_records` update from
   active to inactive and no unrelated helper writes.
6. Drop the test triggers, function, and audit table.

Use audit triggers rather than `ctid` comparisons. They detect unconditional
conflict updates as well as delete-and-reinsert churn.

### Exit criteria

- A no-change refresh writes zero helper rows.
- A one-record source change writes only the corresponding helper row.
- Existing refresh, rollback, and output assertions still pass.

## 2. Qualify more graph nodes for differential refresh

### Add a qualification harness

Before changing more refresh modes, add a reusable E2E assertion that records
the `node_results` returned by `mdm.refresh()` and verifies each logical node's
requested mode, effective mode, and fallback reason.

Cover these mutations for representative definitions:

- insert, update, and delete in each source;
- soft delete and reactivation;
- exact, prefix, token, and composite candidate channels;
- a change that adds a pair and a change that removes a pair;
- no-change refresh;
- candidate block and pair overflow;
- transaction rollback and retry.

For every node moved to `DIFFERENTIAL`, run the mutation matrix with
`full_policy = 'ERROR'` and compare all terminal rows with a separately rebuilt
full graph.

### Change modes in dependency order

Make one mode family differential at a time in `src/graph_spec.rs`:

1. Change `records/<source>` nodes in `source_node()` from `node()` to
   `node_with_refresh_mode(..., "DIFFERENTIAL")`.
2. Change `normalized/<field>` nodes in `normalized_node()` after every admitted
   source and field shape passes the matrix.
3. Change `evidence/<entity>` after candidate-pair additions and removals pass.
4. Change `golden/<entity>` last.

Do not combine these changes into one unreviewable mode flip. If a query shape
cannot qualify, leave only that node or shape on `AUTO` and record the fallback
reason in the test.

### Reduce false graph fan-in before changing golden

`golden/<entity>` currently depends directly on every preceding node and emits
guard expressions for all of them. Replace that list with the relations the
golden SQL actually reads. The evidence node already carries the candidate
subgraph in its transitive dependency closure.

Add a graph-spec unit test that proves:

- the golden root still reaches every required member;
- rendered SQL contains no unresolved logical references;
- the direct dependency list contains no ordering-only dependency.

Do not add barrier nodes or another graph layer unless qualification proves the
minimal dependency list is still unsupported or slower.

### Version and regeneration

After the first graph artifact changes:

1. Increment `src/graph_spec.rs::COMPILER_VERSION` from 8 to 9.
2. Keep `ARTIFACT_FORMAT_VERSION` unchanged unless the serialized artifact
   shape changes.
3. Add `mdm_admin.recompile(entity_name text)` using the existing definition
   parser, compiler, `definition_artifacts`, and `install_graph()` path.
4. Recompile the current desired definition without creating a new semantic
   definition version.
5. Install a new immutable `graph_generation`.
6. Bootstrap and validate the new generation in the same transaction.
7. Adopt it only after contract and full-result equivalence checks succeed.
8. Leave the old binding available until adoption succeeds. Existing lifecycle
   cleanup can remove obsolete generations later.

Do not silently reinterpret an installed artifact and do not overload
`mdm_admin.rebuild()`. Rebuild currently recomputes MDM state against the active
graph; recompilation changes the physical graph.

### Exit criteria

- All admitted record, normalization, candidate, and evidence shapes refresh in
  `DIFFERENTIAL` mode during the mutation matrix.
- Golden is differential where its actual query shape qualifies. It remains
  `AUTO` for any documented unsupported shape.
- Bootstrap remains full.
- Existing entities can adopt compiler version 9 without a semantic definition
  version change.
- Full and differential terminal relations are byte-for-byte equivalent after
  each mutation.

## 3. Bind pg-mdm to the public Delta V1 contract

This phase consumes deltas but still runs the current full resolver. Its purpose
is to prove registration, payload decoding, acknowledgement, and recovery
before they can affect results.

### Capability admission

Extend `src/integration.rs` with `require_output_delta_v1()`.

Accept the capability only when:

- the row exists;
- `major = 1`;
- `enabled = true`;
- the reported details identify the stable contract required by pg-mdm.

A missing or disabled capability is a full-resolution fallback, not an
installation failure. A malformed or incompatible capability is a typed
`MdmError` in `src/error.rs`.

### Catalog state

Add this table to `src/schema.rs` and the 0.12.0 to 0.13.0 upgrade SQL:

```sql
CREATE TABLE mdm_internal.graph_delta_consumers (
    graph_binding_id uuid NOT NULL,
    logical_id text NOT NULL,
    consumer_id uuid NOT NULL UNIQUE,
    delta_relation_name text NOT NULL,
    row_identity_version smallint NOT NULL CHECK (row_identity_version > 0),
    PRIMARY KEY (graph_binding_id, logical_id),
    FOREIGN KEY (graph_binding_id, logical_id)
        REFERENCES mdm_internal.graph_members(graph_binding_id, logical_id)
);
```

Do not store another cursor. `pgtrickle` owns the acknowledged token.

Update lifecycle deletion, extension dump configuration, archive SQL, restore
tests, and upgrade-completeness checks for the new table.

### Registration

In `src/api/create.rs::install_graph()`:

1. Register consumers only for `evidence/<entity>` and `golden/<entity>` after
   the graph binding and member contracts have been stored.
2. Use a deterministic consumer name containing the graph binding ID and
   logical ID.
3. Pass the member's exact output contract digest to
   `pgtrickle.register_output_delta_consumer()`.
4. Store the returned consumer ID, typed delta relation, and row-identity
   version. Read the contract digest from the referenced graph member.
5. Use `CURRENT` only for a new pristine graph before its first population.
6. Add `ensure_delta_consumers()` to the refresh path. It lazily registers a
   missing consumer for an upgraded or already populated active binding as
   `RESNAPSHOT_REQUIRED`, while the validated execution role is selected.
7. Let the upgrade SQL create catalog state only. It must not try to register
   consumers for every entity under one installation role.

### Read and acknowledge protocol

Add small, local helpers in `src/api/refresh.rs` for these public calls:

- `pgtrickle.output_delta_consumer_status()`;
- `pgtrickle.output_delta_batches()`;
- typed reads from the returned `delta_relation` filtered by `batch_token` and
  ordered by `ordinal`;
- `pgtrickle.ack_output_delta()`;
- initial `begin_output_delta_resnapshot()` and
  `ack_output_delta_resnapshot()`.

Validate contiguous tokens, batch mode, digest, row-identity version, action,
row count, and decoded column types before using a payload.

Protocol rules:

- A sequence containing only `EXACT` batches can be acknowledged as `APPLIED`.
- `FULL_INVALIDATION` leaves the v0.108.0 consumer active. Run a full MDM
  baseline and acknowledge through that token as `RESYNCHRONIZED`.
- Use begin/ack resnapshot only for consumer states `RESNAPSHOT_REQUIRED` or
  `INVALIDATED`.
- Read, publish, and acknowledge in the existing
  `persist_refresh_inner()` transaction. A raised error must roll all of them
  back.

### Observability

Add these fields to the refresh outcome and `mdm.describe()`:

- `resolver_strategy`: `full`, `shadow`, or `affected`;
- `resolver_fallback_reason`;
- `delta_batch_count` and `delta_row_count`;
- `delta_acknowledged_token` and `delta_lag`;
- counts of graph nodes by effective mode;
- logical IDs and reasons for unexpected full fallbacks;
- affected record and component counts.

Keep the existing raw `node_results`. Add summaries rather than another event
system.

### Exit criteria

- New and upgraded entities establish valid evidence and golden consumers.
- Exact payload rows decode deterministically.
- A successful full resolution acknowledges the exact batches it observed.
- A forced failure after acknowledgement but before transaction return leaves
  both publication state and cursor unchanged.
- A full invalidation produces a full result and a `RESYNCHRONIZED`
  acknowledgement.
- A no-change refresh leaves lag at zero.

## 4. Compute and verify the affected set in shadow mode

Add `src/affected.rs` for pure affected-set construction. Keep SQL loading in
`src/api/refresh.rs`; do not build a generic graph library.

### Seeds

Seed the set with every source record mentioned by an `INSERT` or `DELETE` row
from both evidence and golden terminal deltas.

Also read the existing append-only control histories between the last published
decision epoch and the current entity decision epoch:

- For each changed `steward_decisions` row, seed both endpoints. This includes a
  changed `NOT_MATCH`; it seeds endpoints but does not connect components.
- For each changed `golden_override_directives` row, seed the anchor record.

If the epoch interval cannot be reconstructed exactly, choose full resolution.
Do not add a second control journal unless the existing history proves
insufficient in a test.

### Fixed-point closure

Repeat these steps until the set stops growing:

1. For every selected record, add every record that shares its old published
   `mdm_id`. Include historical memberships needed to represent a deletion or
   split.
2. Add both endpoints of each current manual `MATCH` edge incident to the set.
3. Add both endpoints of each current pair decision that can supply identity or
   admission support: `AutomaticIdentity`, `AutomaticStrong`, and `Review`.

`Review` edges must participate because they can supply the independent
evidence group used by singleton admission. `NoEdge`, `AuthoritativeConflict`,
and `Prohibited` do not connect the closure. Their changed terminal rows still
seed their endpoints.

After reaching a fixed point, prove that no current connecting edge crosses
from the selected set to an unselected record. Fall back to full resolution if
the proof fails.

### Global guards

An affected run still enforces entity-wide limits.

- Read cheap global counts from the terminal relations for
  `max_active_records` and `max_automatic_edges`.
- Preserve the full path when exact `max_component_checks` equivalence cannot
  be proved from the affected and retained resolution facts.
- Fall back on a contract mismatch, row-identity mismatch, token gap, payload
  inconsistency, unsupported control change, or resource-limit uncertainty.

Do not add a percentage threshold yet. Record the affected-to-active ratio and
add a cutoff only if benchmarks show a repeatable crossover.

### Shadow comparison

For every exact batch:

1. Build the affected set.
2. Load the scoped sources, decisions, pair decisions, old identity, reviews,
   golden values, facts, and overrides.
3. Call the existing `evaluation::resolve_and_compare()` on that scope.
4. Splice the scoped result into the untouched old state in memory.
5. Continue to publish the independently computed full result.
6. Compare the complete semantic projection and error outcome. Record only
   counts and digests in operation metadata.

Move the existing preview helpers `scope_identity()`, `scope_reviews()`,
`scope_current_golden()`, and `scope_resolution_facts()` into shared pure code
only when both preview and shadow evaluation use them.

### Tests

Add pure affected-set tests for:

- evidence insert and delete;
- a rejected edge that becomes admissible;
- an old-component split;
- a merge cascade;
- an isolated golden-only change;
- `NOT_MATCH` add, replacement, and removal;
- order independence.

Extend `tests/resolver_property_tests.rs` with generated operation sequences.
For each step, compare affected-plus-splice with a clean full resolution. Check
complete identity, golden, review, resolution-fact, digest, and error
equivalence, not only row counts.

### Exit criteria

- Shadow and full results match for every generated and E2E case.
- Shadow and full failures return the same result class for global limits.
- No closure proof failure proceeds as affected.
- Shadow mode is stable for at least one representative soak run before it can
  publish.

## 5. Enable affected resolution and scoped publication

### Strategy selection

In `persist_refresh_inner()`, select the affected path only when all of these
conditions hold:

- Delta V1 is admitted and both terminal consumer bindings match the active
  graph generation.
- Both consumers are active and have a contiguous exact range through the graph
  refresh just completed.
- Contract digests and row-identity versions match the stored bindings.
- Control changes since the last publication are exactly reconstructable.
- The closure and global-limit proofs succeed.
- The request is not a rebuild or graph-generation transition.

Otherwise call the current full path and record one stable fallback reason.
If both exact ranges are empty and the decision epoch did not change, skip the
resolver and publication writes, store the observation, and return
`changed = false`.

### Reuse the evaluator

Do not create a second resolver. Extract only the input loading, scope, splice,
and publication boundaries needed to call
`evaluation::resolve_and_compare()` for either all records or the proved set.

The splice must preserve:

- untouched identity registry rows and memberships;
- alias and split history;
- untouched open and resolved reviews, including occurrence counters;
- untouched golden selections;
- untouched resolution facts.

Canonicalize the merged facts before assigning fact numbers. Never drop or
renumber facts based on an incomplete scope.

### Publication writes

Start with the smallest safe persistence change:

1. Upsert only changed identity, membership, and review rows. Add
   `IS DISTINCT FROM` predicates to avoid unchanged row versions.
2. Delete stale mutable output rows only within the affected source-record,
   MDM-ID, or review-ID set.
3. Append only new alias and split rows.
4. Preserve the complete per-revision golden and resolution-fact snapshots with
   set-based copy-forward of untouched rows plus affected replacements.
5. Compute the publication digest from the complete merged semantic projection,
   using the existing canonicalization.

The copy-forward tables can still write O(entity size) rows. Keep that format in
the first affected release because changing publication history is a separate
data-model migration. Instrument the row volume. Introduce validity intervals
or another storage model only if profiling shows copy-forward dominates the
remaining refresh cost.

### Transaction order

Within the existing entity advisory-lock transaction:

1. Synchronize source and helper rows.
2. Refresh the graph.
3. Read and validate both terminal delta ranges.
4. Select full or affected evaluation.
5. Persist the publication or no-change observation.
6. Acknowledge both consumers through the validated token.
7. Complete the operation.
8. Return and let PostgreSQL commit.

Any error after step 3 must roll back the graph refresh, publication, consumer
cursor, observation, and operation completion.

### Exit criteria

- A single-record mutation does not scan all evidence or golden terminal rows.
- Affected and full publications remain semantically identical under generated
  mutation sequences.
- Rollback never advances only one terminal consumer or exposes a partial
  publication.
- Merge, split, retire, reactivate, decision, and override cases pass E2E.
- No-change refreshes do no helper or publication writes and finish with zero
  delta lag.

## 6. Roll out safely

Use this activation sequence:

1. Ship helper synchronization and metrics.
2. Ship differential graph modes one family at a time.
3. Ship Delta V1 consumers with full resolution only.
4. Enable shadow affected-set comparison.
5. Enable affected publication for exact data deltas with no stewardship
   change.
6. Enable exact decision and override intervals after their generated tests
   pass.
7. Recompile one canary entity to compiler version 9.
8. Recompile the remaining entities after canary equivalence and latency checks
   pass.

Production continues with `full_policy = 'ALLOW'`. Alert on any steady-state
full node or resolver fallback that is not one of the documented recovery
cases. Use `ERROR` only in qualification until failed refresh metadata is
durably observable outside the rolled-back transaction.

## File-by-file change map

| File | Required change |
|---|---|
| `src/api/refresh.rs` | Delta-preserving helper sync, Delta V1 reads and acks, strategy selection, scoped loading, splice, publication ordering |
| `src/graph_spec.rs` | Per-family differential modes, minimal golden dependencies, compiler version 9 |
| `src/api/create.rs` | Register terminal consumers after graph installation |
| `src/integration.rs` | Strict Delta V1 admission |
| `src/affected.rs` | Pure seed and fixed-point closure logic |
| `src/evaluation.rs` | Reuse the existing evaluator for a proved scope; no second semantics path |
| `src/api/describe.rs` | Resolver, delta lag, and fallback summaries |
| `src/schema.rs` | `graph_delta_consumers` and dump configuration |
| `src/api/lifecycle.rs` | Delete local consumer bindings with graph/entity lifecycle |
| `src/error.rs` | Typed capability, delta, and closure failures |
| `sql/pg_mdm--0.12.0--0.13.0.sql` | Upgrade catalog and grants |
| `sql/archive/pg_mdm--0.13.0.sql` | Release archive after packaging |
| `tests/e2e.sql` | Helper churn, modes, Delta V1, rollback, fallback, upgrade, restore |
| `tests/resolver_property_tests.rs` | Full versus affected generated equivalence |

## Required test matrix

Run the focused checks after each pull request, then the full list before
enabling affected publication:

```bash
just fmt
just lint
just test-unit
just test-resolver
just test-resolver-properties
just test-publication-properties
just test-e2e
just check-upgrades
just check-archive
```

The final E2E matrix must include:

- initial and upgraded consumer baselines;
- exact insert, update, delete, soft delete, and reactivation;
- pair and evidence additions and removals;
- identity merge and split;
- decision and golden-override changes;
- candidate overflow and entity-wide limit failures;
- `FULL_INVALIDATION`, token gap, resnapshot, and contract mismatch;
- failure after publication and acknowledgement followed by retry;
- concurrent refresh serialization;
- backup and restore of local consumer bindings;
- graph compiler version 8 to 9 recompilation and adoption.

## Success measures

Capture these values before phase 1 and after each phase on the same data and
mutation workload:

- helper rows inserted, updated, and deleted;
- requested and effective graph modes by logical node;
- full-fallback count and reason;
- terminal delta rows and lag;
- active records versus affected records;
- total pair decisions versus affected pair decisions;
- graph, resolver, and publication durations;
- publication rows written;
- full-versus-affected semantic digest equality.

The release is successful when:

- no-change refreshes produce zero helper writes and zero resolver work;
- normal small mutations use differential graph refresh for every qualified
  node;
- normal small mutations use affected resolution;
- p95 refresh latency scales with the changed component rather than total entity
  size on representative workloads;
- the generated and E2E equivalence suites find no semantic or error mismatch;
- every fallback is visible and recoverable.

## Explicitly out of scope

- Removing the full resolver or full graph bootstrap.
- Reading private pg-trickle catalogs or change buffers.
- Waiting for unreleased prepared-generation or delta-binding APIs.
- Parallel affected-component execution before the serial path is measured.
- A configurable affected-set cutoff before benchmarks establish a crossover.
- Redesigning publication history unless copy-forward is a measured bottleneck.
