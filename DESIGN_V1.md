# `pg_mdm` OSS v1 design

## Deterministic entity resolution and golden records on PostgreSQL

**Status:** Proposed first open-source release  
**Foundation:** [`pg_trickle`](https://github.com/trickle-labs/pg-trickle) provides change capture and incremental relational maintenance  
**Goal:** Ship a useful, conservative MDM system while keeping advanced source contracts, stewardship workflows, and enterprise operations outside the first compatibility promise  
**Product contract:** Five nouns, five actions, and three public outputs

---

## 1. Release boundary

`pg_mdm` v1 resolves records that already exist in PostgreSQL. A user defines a real-world entity, maps source columns to logical fields, describes which agreements count as identity evidence, and chooses how each golden value should be selected. The extension then publishes a durable membership map, one golden record per resolved entity, and a review queue for decisions that should not be made automatically.

The user-facing model deliberately remains small. It has five nouns—`source`, `field`, `match`, `entity`, and `golden_value`—and normal operation has five actions—`create`, `describe`, `preview`, `refresh`, and `explain`. Every entity has three stable public output tables: `mdm_out.<entity>`, `mdm_out.<entity>_members`, and `mdm_out.<entity>_review`. Definition versions, source-record identities, publication revisions, operation records, aliases, and private evidence relations are real system objects, but they are supporting lifecycle objects rather than additional concepts a normal user must configure.

The central architectural rule is simple:

> **`pg_trickle` maintains changing relational facts; `pg_mdm` decides identity.**

`pg_trickle` is responsible for capturing source changes, maintaining normalized and candidate relations incrementally, tracking complete source frontiers, ordering private dependency graphs, and falling back to a full relational recomputation when incremental maintenance is not safe. `pg_mdm` is responsible for pair decisions, manual constraints, conservative clustering, stable `mdm_id` continuity, golden overrides, reviews, and atomic publication. This division keeps generic incremental SQL machinery out of the MDM engine while ensuring that identity semantics remain explicit, versioned, and explainable.

```text
Source relations
      │
      ▼
Private pg_trickle graph
  projected records
  normalized values
  candidate blocks
  candidate pairs
  pair evidence
  golden candidates
      │
      ▼
pg_mdm resolver
  decisions
  component checks
  clustering
  stable IDs
  golden values
  reviews
      │
      ▼
Public base tables
  mdm_out.<entity>
  mdm_out.<entity>_members
  mdm_out.<entity>_review
```

V1 includes deterministic built-in normalization, exact and bounded fuzzy comparisons, bounded candidate generation, incrementally maintained evidence, full-entity resolution, one conservative clustering policy, stable entity identity, anchored golden overrides, full-rebuild fallback, atomic publication, and current-state explanation with limited publication history. It does not include affected-set MDM resolution, custom matching code, approximate nearest-neighbor discovery, complete-snapshot or generic foreign-source contracts, valid-time reconstruction, a semantic downstream change feed, direct entity merge or partitioned split commands, alternate clustering policies, managed downstream MDM dependencies, multi-tenant resource isolation, resumable multi-transaction runs, configurable history compaction, formal service objectives, or extension-managed workload fairness. None of those capabilities is a hidden V1 requirement; any later addition must enter through the V2 roadmap and its compatibility rules.

---

## 2. Product model

### 2.1 Source

A source is a PostgreSQL relation whose rows represent records that may participate in one entity definition. V1 supports local tables and partitioned tables in tracked or soft-delete mode. Each source has a stable name within the entity, a declared source mode, one or more source-key columns, mappings from physical columns to logical fields, and optional field authority. Ordinary views, materialized views, and foreign tables are not part of the V1 promise. The initial integration admits only trigger capture on supported local regular and partitioned tables. WAL capture and complete-snapshot or remote source contracts require separate qualification before adoption.

The source key must be backed by a primary key or by a non-partial, non-expression, immediate unique index whose key values cannot be ambiguously null. V1 allows scalar and supported composite keys. The key is encoded with the versioned typed row-identity contract supplied by `pg_trickle`, combined with the entity and an immutable internal source identity, and stored as a durable internal `source_record_key`. The first accepted definition allocates that source identity and freezes its source name, relation binding, key columns, key types, collations, and encoding contract. A later definition may reuse it only with the same identity contract. Changing the relation lineage or key contract requires a new source name and identity. Logical restore uses a separate administrator-approved rebind that verifies the portable relation name and frozen contract before accepting new database-local object IDs. The first time a key is observed, `pg_mdm` also allocates an opaque `source_record_id` for stewardship and explanation. Reusing the same source key for a different real-world source record is unsupported.

### 2.2 Field

A field is a logical attribute such as `name`, `email`, `phone`, `date_of_birth`, or `tax_id`. Different sources may map differently named physical columns to the same field. A field declares a logical PostgreSQL type, one versioned built-in cleaner, cleaner options where needed, and a display policy for review and explanation. V1 requires a UTF-8 database and uses deterministic binary ordering for semantic tie breaks so that locale-sensitive query plans do not change identity results.

Cleaning never overwrites source data. The private evidence graph stores the original value only when current golden output or provenance requires it, and stores normalized values separately. A cleaner returns one of the states `value`, `absent`, `empty`, `invalid`, `unknown`, `redacted`, or `unsupported`. SQL `NULL` means `absent`. A source mapping may provide `unknown` or `redacted` only through a separate validated state column with the tags `present`, `unknown`, and `redacted`; cleaners never infer either state from a magic value. When the tag is `unknown` or `redacted`, the graph ignores and does not retain the value column. Values that are absent, empty, invalid, unknown, redacted, or unsupported provide no positive or negative automatic matching evidence. An invalid value from a source declared authoritative may create a review because it weakens a promised identity check.

### 2.3 Match

A match describes how one or more fields contribute identity evidence. It names a built-in exact or fuzzy comparison, assigns the result the strength `identity`, `strong`, or `supporting`, and assigns an `evidence_group`. Evidence groups make correlation explicit: two rules derived from the same underlying fact, such as an email-address rule and an email-domain rule, cannot be counted as independent evidence merely because they are separate rules. A declared group may only reduce independence. The compiler records each rule's normalized input lineage and rejects a definition that places rules with overlapping lineage, or alternate derivations of one normalized fact, in different effective groups.

Identity evidence is intended for identifiers that can decide a pair by themselves, such as a trustworthy tax identifier. Strong evidence may create an automatic pair edge, such as an exact normalized email match. Supporting evidence can reinforce a strong edge, help admit a singleton into an established component, or create a review, but it never discovers a pair or creates an automatic pair edge by itself. Source authority is evaluated separately: two valid, different authoritative identity values on active source records create an automatic conflict unless a steward has recorded an explicit `MATCH` decision.

### 2.4 Entity

An entity is the real-world thing being mastered, such as a person, company, supplier, account, or product. Its immutable definition owns its sources, fields, matches, golden policies, masking rules, and built-in preset versions. The entity definition expresses what identity means. It does not expose worker counts, SQL join order, indexes, temporary-table choices, or physical query plans.

The compiler does expose the logical candidate plan because candidate discovery is part of matching semantics. It records which deterministic predicates can discover a pair, which evidence rules are evaluated for those pairs, and which hard completeness limits apply. The implementation may change indexes or execution strategies without changing the definition, but it may not silently change the logical set of pairs a pinned definition is supposed to examine.

### 2.5 Golden value

A golden value selects one output field independently from the current members of an entity. V1 includes only versioned built-in policies: `prefer_source`, `latest`, `first_non_null`, and `most_common`. Source authority for matching does not automatically make a source preferred for every output field, so golden precedence is always explicit.

Every selected value retains provenance. At minimum, provenance identifies the `mdm_id`, field, selected value, normalized value, winning source record when one exists, policy and policy version, deterministic tie-break reason, definition version, and publication revision. A computed policy such as `most_common` also records the contributing normalized values and the source record chosen to supply the published spelling.

---

## 3. Source modes and completeness

V1 replaces the earlier `changed_at` polling idea with source modes that describe how completeness is actually established. A timestamp can be useful metadata, but it is not a commit-order or completeness proof. A source may still map a trustworthy row-change timestamp for the `latest` golden policy, but `pg_mdm` does not use that timestamp to decide whether all source changes have been consumed.

| Mode | Eligible relations | Change detection | Deletion behavior | Completeness at publication |
|---|---|---|---|---|
| `tracked` | Local table or partitioned table | `pg_trickle` trigger capture | Hard deletes are captured explicitly | Inserts, updates, and deletes are proven through the recorded frontier |
| `soft_delete` | Same relations as `tracked`, plus a deletion predicate | `pg_trickle` capture plus predicate evaluation | A matching predicate makes the row inactive; a hard delete is also captured | Lifecycle changes and hard deletes are proven through the recorded frontier |
A tracked source is the preferred V1 mode. `pg_trickle` captures committed changes and gives the private graph a contiguous source frontier, so `pg_mdm` does not need a second CDC mechanism or a user-maintained watermark. A soft-delete source uses the same capture path but treats a declared predicate such as `deleted_at IS NOT NULL` as a business lifecycle signal. The source record identity and tombstone remain durable, and a later reactivation of the same key is processed as a normal lifecycle change.

Every publication records the exact source boundaries returned by `pg_trickle`. The word `current` means current through those recorded boundaries, not identical to the database at the instant a later reader runs a query. `mdm.describe()` shows both `change_completeness` and `deletion_completeness` for each source, together with the opaque frontier and whether later work is known, unknown, or pending.

V1 rejects ordinary and materialized views. A user may copy a derived result into a local table that `pg_trickle` tracks, but the producer must preserve stable source keys and emit complete inserts, updates, and deletes through that table. This keeps source completeness and execution-role behavior explicit.

---

## 4. Entity definitions and versions

An entity definition is an immutable, versioned value. `mdm.create()` accepts a complete definition and stores a new desired version only when its definition digest differs from the current desired version. Updating an existing entity requires `expected_version`, which prevents accidental overwrites. Reusing the current desired definition is a no-op. Returning to an older definition is a new version: A→B→A creates versions 1, 2, and 3 even though versions 1 and 3 have the same definition digest.

V1 supports one optional versioned built-in preset such as `person`, `company`, or `product`, plus explicit entity-local overrides. Presets may not silently change an active entity. The stored definition contains the fully expanded value and exact preset version. Preset composition, user-defined reusable fragments, and nested configuration graphs are deferred.

V1 keeps three domain-separated SHA-256 identities:

| Identity | Canonical input |
|---|---|
| Definition digest | Portable logical semantics: expanded sources, fields, state mappings, rules, candidate predicates, limits, source-key encoding, exact cleaner and comparator versions, pair-decision policy, clustering policy, stable-ID policy, golden policies, semantic engine version, and required capability version |
| Compiled-artifact digest | Definition digest, compiler and artifact format versions, exact graph-spec bytes, generated SQL, and versioned terminal-relation schemas |
| Graph-binding digest | Compiled-artifact digest plus the database-local graph generation, execution-role binding, source object bindings, member contract generations and digests, and canonical graph digest |

Physical indexes, planner choices, batch sizes, and worker counts are not inputs to any semantic decision. The digest encodings and domain tags are versioned and covered by canonical fixture vectors.

Definitions never change after insertion. A compiler stores each compiled artifact as an append-only attachment to one definition version. An upgrade may attach a newer compiler artifact without rewriting the definition or the older artifact. Execution selects one exact artifact and dispatches every cleaner, comparator, candidate rule, clustering policy, stable-ID policy, and golden policy by the version recorded in that artifact. It never resolves a stored name to the current implementation.

Every executable stage requires all its semantic inputs to be present in the stored definition and manifest, including defaults and hard limits. A compiler cannot supply a missing semantic value from its current defaults. Development artifacts may record unimplemented stages, but those stages remain non-executable. Adopting semantics introduced by a later development release requires an explicit new definition version when the existing version lacks them.

Each installed artifact receives an append-only graph binding and a new private `pg_trickle` graph generation. The active graph is never altered in place to adopt new matching semantics or compiler output. A proposed binding receives its own private relations, is validated independently, and becomes active only when `mdm.refresh()` successfully publishes an MDM revision under that binding. After activation, the previous graph may be retired once the minimum explanation horizon has been preserved. An extension upgrade cannot silently change the evidence behind an active publication.

`mdm.describe(format => 'definition')` returns the canonical expanded definition without private relation names or physical storage choices. It can be passed to `mdm.create()` in another database, where relation names and source-key contracts are resolved again. Portability does not mean that independently initialized databases receive identical newly allocated UUIDs; it means they compile the same matching semantics and reject incompatible dependencies explicitly.

The constructors below illustrate the intended model rather than freeze the final PostgreSQL type signatures:

```sql
WITH proposed AS (
    SELECT mdm.entity(
        name   => 'customer',
        preset => 'company',

        sources => ARRAY[
            mdm.source(
                name           => 'crm',
                relation       => 'crm.customer'::regclass,
                source_id      => ARRAY['id'],
                mode           => 'tracked',
                row_changed_at => 'updated_at',
                fields         => jsonb_build_object(
                    'name',   'display_name',
                    'email',  'email_address',
                    'tax_id', 'vat_number'
                )
            ),
            mdm.source(
                name      => 'registry',
                relation  => 'registry.company'::regclass,
                source_id => ARRAY['registry_id'],
                mode      => 'tracked',
                fields    => jsonb_build_object(
                    'name',   'registered_name',
                    'email',  null,
                    'tax_id', 'registered_tax_id'
                ),
                authority => jsonb_build_object(
                    'tax_id', 'authoritative'
                )
            )
        ],

        fields => ARRAY[
            mdm.field(name => 'name',   type => 'text', cleaner => 'company_name'),
            mdm.field(name => 'email',  type => 'text', cleaner => 'email'),
            mdm.field(name => 'tax_id', type => 'text', cleaner => 'tax_id')
        ],

        matches => ARRAY[
            mdm.match(
                name           => 'same_tax_id',
                fields         => ARRAY['tax_id'],
                comparison     => 'exact',
                strength       => 'identity',
                evidence_group => 'tax_identity'
            ),
            mdm.match(
                name           => 'same_email',
                fields         => ARRAY['email'],
                comparison     => 'exact',
                strength       => 'strong',
                evidence_group => 'email'
            ),
            mdm.match(
                name           => 'similar_name',
                fields         => ARRAY['name'],
                comparison     => 'fuzzy',
                strength       => 'supporting',
                evidence_group => 'name'
            )
        ],

        golden_values => ARRAY[
            mdm.golden_value(
                field   => 'name',
                policy  => 'prefer_source',
                sources => ARRAY['registry', 'crm']
            ),
            mdm.golden_value(
                field   => 'email',
                policy  => 'prefer_source',
                sources => ARRAY['crm']
            ),
            mdm.golden_value(
                field   => 'tax_id',
                policy  => 'prefer_source',
                sources => ARRAY['registry', 'crm']
            )
        ]
    ) AS definition
)
SELECT mdm.create(
    definition       => definition,
    expected_version => null,
    comment          => 'Initial customer identity model'
)
FROM proposed;
```

V1 allows additive evolution of the public golden table. A new golden field may be appended when a new definition becomes active. Removing, renaming, reordering, or changing the PostgreSQL type of an existing public golden column is rejected in V1; users who need a breaking output contract create a new entity name. Internal matching fields may be added or removed because they do not change the public table schema, but every such change creates a new semantic definition and private graph generation.

---

## 5. The `pg_trickle` integration contract

`pg_mdm` depends on `pg_trickle` as a supported extension, not as a collection of private tables to query. It must never read or write `pg_trickle` private catalogs, frontiers, change buffers, scheduler tables, or generated implementation columns. The two projects interact through a small, versioned SQL contract whose behavior is stable even if `pg_trickle` changes its internal storage or execution plan.

This contract is a V1 release gate jointly owned by the two projects. Before `pg_mdm` freezes its SQL API, a supported `pg_trickle` release must advertise `external_graph_refresh` major 1 as enabled through `pgtrickle.integration_capabilities()` and pass a shared conformance suite. An absent or disabled capability is unsupported regardless of the package version. Existing internal functions, private catalogs, or a sequence of individually supported calls do not satisfy the gate.

For each definition version, `pg_mdm` creates every private stream table with durable `EXTERNAL` orchestration and initialization disabled. It uses `pgtrickle.stream_table_contract()` to pin each member's contract generation and digest, then uses `pgtrickle.graph_contract()` to pin one canonical digest for the complete root closure. `pgtrickle.set_orchestration_mode()` is the explicit transition API when creation cannot set the mode directly. `EXTERNAL` members are never dispatched by the ordinary scheduler or refreshed through the human-oriented single-table API.

The required execution primitive is `pgtrickle.refresh_graph_strict(roots, expected_graph_digest, full_policy)`. It validates and locks the complete graph before mutation, refreshes members in deterministic dependency order against one coherent source-boundary decision, and returns `pgtrickle.graph_refresh_result`. The result includes `contract_version`, `graph_refresh_id`, `graph_digest`, `source_boundary`, `source_boundary_digest`, and per-node results with the contract digest, stable result class, detailed action, row-identity version, and row-probe version. A concurrent refresh or lifecycle mutation waits under PostgreSQL lock policy or raises a stable error; it never becomes a notice-and-skip success.

The refresh and frontier advancement occur inside the caller's outer PostgreSQL transaction, so a later `pg_mdm` failure rolls the private graph and its consumed frontiers back with the public MDM publication. Graph V1 admits only supported local CDC sources and fails before graph mutation when it cannot prove a complete boundary.

V1 does not require the separately versioned `output_delta_consumer` capability. After refreshing the graph, `pg_mdm` reads the complete terminal evidence relations and runs the full resolver. This keeps differential maintenance inside `pg_trickle` and avoids making `pg_mdm` correctness depend on reconstructing a complete affected set from change events. If the installed release advertises Delta V1, V1 ignores it for publication semantics.

All private MDM stream tables remain in `EXTERNAL` orchestration mode so that pair evidence cannot advance independently of membership, golden values, and reviews. `mdm.refresh()` is the only normal coordinator that may advance the private graph. Administrators may schedule `mdm.refresh()` through ordinary PostgreSQL scheduling tools, but the `pg_trickle` scheduler does not publish an MDM entity by itself.

V1 uses differential or automatic-full-fallback maintenance for private graphs. It does not use `pg_trickle` immediate mode because MDM clustering and stable-ID reconciliation are intentionally not performed inside every source DML statement. A `pg_trickle` choice between differential and full relational evaluation is a performance decision only: both paths must produce the same terminal evidence relations for the same source boundary and pinned graph specification.

The `pg_mdm` package declares a supported `pg_trickle` release line, but runtime capability negotiation is authoritative. An unsupported `external_graph_refresh` major or minor, unknown row-identity encoding, changed member contract, changed graph digest, or missing strict integration primitive blocks creation and refresh before any public output changes. V1 is not complete until the shared conformance suite covers commit, rollback, concurrent source writes, concurrent refresh and lifecycle attempts, full fallback, crash recovery, scheduler exclusion, clone isolation, and extension upgrade.

---

## 6. The private evidence graph

Each compiled artifact contains a private graph of ordinary relational stages. A typical graph has a source-record stage that projects stable keys and mapped values, a normalized-value stage that applies built-in cleaners, one or more candidate-block stages, a canonical candidate-pair stage, a pair-evidence stage, and a golden-candidate stage. The exact number of stream tables is private, but `mdm.describe()` exposes the logical stages, predicates, hard limits, and three digest classes to authorized operators without requiring generated table names.

```text
source relations
      │
      ▼
records ──► normalized values ──► candidate blocks ──► candidate pairs
                    │                                      │
                    └──────────────────────────────────────► pair evidence
                    │
                    └──────────────────────────────────────► golden candidates
```

Candidate generation is deterministic and exact within its declared logical predicates. Identity and strong rules compile to built-in channels such as normalized exact equality, composite equality, deterministic prefixes, or bounded token blocks. Fuzzy comparison is evaluated only after a pair has been discovered by a complete built-in channel. Supporting rules are evaluated only for discovered pairs; an undiscovered pair cannot become an automatic edge or review because of supporting evidence alone. V1 does not use approximate nearest-neighbor indexes, planner-dependent top-k discovery, random sampling, or custom candidate-key functions in publication semantics.

The logical candidate plan is pinned with the definition. If an engine upgrade wants to replace one blocking predicate with another, change a tokenization rule, alter a block limit, or change the number of neighbors examined, that is a new semantic plan and requires a new entity definition version. PostgreSQL indexes and join algorithms may change freely as long as they enumerate the same logical pair set.

Every built-in channel has a hard completeness limit. A separate graph stage computes block cardinalities and fails before the pair-join stage runs when a block exceeds the limit. The public error reports the channel, cardinality, configured limit, and next action. Any raw block key or plain digest is sensitive and remains available only in protected internal diagnostics. The canonical candidate-pair relation also has an aggregate hard resource ceiling after duplicates across channels are removed. Crossing either limit fails the refresh; it never truncates the relation. V1 does not publish singleton assignments for records whose required pair enumeration was truncated, because that would silently turn missing work into evidence of non-match. A lower warning threshold may create a review while the channel is still complete, but crossing a hard limit is always a fail-closed condition.

V1 ships only extension-owned cleaners, comparators, and golden selectors. Generated SQL calls an exact versioned entry point for each function. Versions, options, fixed-point arithmetic, and tie ordering are pinned in the definition digest and the compiled artifact. Unknown or unavailable versions fail closed. User-defined PostgreSQL functions, relation lookups from semantic functions, network access, clock time, random state, custom component checks, and custom candidate generation are deferred because they would require a much larger dependency, invalidation, security, and reproducibility contract.

The compiler emits one allowlisted `SELECT` shape per stage. It quotes identifiers, binds values, and schema-qualifies relations, functions, operators, and casts. It accepts no raw user SQL. Before an artifact can become executable, PostgreSQL must parse and run every stage against rollback-only fixtures under the bound execution role, row-level security, and a hostile session `search_path`.

---

## 7. Pair decisions

A built-in comparator evaluates one candidate pair and returns a versioned evidence result. Exact rules return agreement, disagreement, or no usable evidence. Fuzzy rules additionally return a fixed-point similarity value and a built-in classification threshold, but floating-point arithmetic is not used in identity decisions. Every evidence item records its rule, evidence group, normalized inputs or protected digests, classification, and comparator version.

The pair-decision table is deliberately small and ordered. An active `NOT_MATCH` decision is a cannot-link constraint and always prohibits the pair. An active `MATCH` decision is an explicit human must-link and takes precedence over automatic evidence, including an authoritative identity conflict; this override is visible in every explanation. Without a manual decision, different valid authoritative identity values on active source records produce `AUTHORITATIVE_CONFLICT`. Otherwise an agreeing identity rule produces an automatic identity edge, and an agreeing strong rule produces an automatic strong edge. Supporting evidence alone produces no automatic edge, although it may create or prioritize a review for a discovered pair.

| Highest applicable condition | Pair result |
|---|---|
| Active `NOT_MATCH` | Prohibited pair |
| Active `MATCH` | Manual match edge |
| Conflicting valid authoritative identity values | Authoritative conflict and review |
| At least one agreeing identity rule | Automatic identity edge |
| At least one agreeing strong rule | Automatic strong edge |
| Supporting evidence only | Review candidate or no edge |
| No usable agreement | No edge |

Strong disagreement is not a universal cannot-link because many strong fields can legitimately change or be shared. It can lower the edge ordering, contribute to a review, or prevent a component-admission condition from being satisfied, but only an active `NOT_MATCH` or an automatic authoritative identity conflict blocks a union. Presets must therefore reserve `identity` and authoritative status for values whose disagreement really is meaningful.

Automatic edges are processed in a total deterministic order. Identity edges precede strong edges; edges with more independent agreeing evidence groups precede those with fewer; remaining built-in evidence vectors are compared lexicographically using fixed-point values; and exact ties are broken by the canonical encoded source-record pair. The order is a versioned part of the pair-decision policy and cannot change for an active definition.

---

## 8. One conservative clustering policy

V1 has one clustering policy. It is designed to prevent a common entity-resolution failure in which a chain of reused emails, phone numbers, or addresses gradually accretes unrelated records into one large component. The algorithm therefore evaluates both pair evidence and the current shape of the components it would join.

The resolver starts with one component per active source record and loads every active `NOT_MATCH` as a component-level cannot-link constraint. It then applies active `MATCH` decisions in canonical source-record order. Stewardship writes cannot create a must-link closure that contains an active `NOT_MATCH`: a conflicting `MATCH` is rejected, and a `NOT_MATCH` whose endpoints already share an active `MATCH` closure is rejected. A valid manual match may join components even when automatic authoritative evidence disagrees, because the steward has explicitly chosen to override that evidence.

Automatic edges are processed in the deterministic order defined above. Before any automatic union, the resolver rejects the union if it would place a `NOT_MATCH` pair in one component or if the two components contain conflicting valid authoritative identity values. It then applies the following admission rules:

| Components being joined | Automatic admission rule |
|---|---|
| Two singletons | One identity edge or one strong edge |
| One singleton and one established component | One identity edge, or one strong edge plus an additional agreeing evidence group connecting the singleton to the component |
| Two established components | Shared authoritative identity, or at least two independent strong cross-component connections on distinct member pairs |

Two items of evidence are independent only when their compiled evidence groups differ and their normalized input lineages are disjoint. For an established-to-established union, the strong connections must also involve distinct cross-component member pairs; two rules on one pair do not prove that two existing clusters are broadly compatible. Supporting evidence may provide the additional independent group for a singleton joining an established component, but it cannot satisfy the established-to-established rule.

A plausible automatic edge that fails component admission becomes a review item with the rejected bridge, component summaries, conflicting values, and the exact rule that was not satisfied. It is not silently discarded. Accepted edges and rejection reasons are retained for `mdm.explain()`. Alternate clustering algorithms, configurable component policies, and custom component checks are deferred to V2 because changing any of them is a major semantic compatibility decision.

---

## 9. Refresh, full resolution, and atomic publication

`mdm.refresh()` is a synchronous transaction coordinator. V1 intentionally chooses one outer PostgreSQL transaction rather than a multi-transaction staging protocol. This can make a very large refresh a long transaction, but it gives the first release a clear consistency model: either the private evidence graph, MDM identity state, three public outputs, provenance, reviews, and source frontiers all commit together, or none of them do. Resumable work and checkpointed multi-transaction execution are V2 capabilities.

A refresh proceeds as follows:

1. The coordinator acquires the entity row lock and an entity-scoped advisory transaction lock. It pins the desired definition version and definition digest, selected compiled-artifact digest, graph-binding digest, active publication revision, stewardship decision epoch, private graph generation, member contract generations and digests, and semantic engine versions.
2. It invokes `pgtrickle.refresh_graph_strict()` with the private roots, expected graph digest, and `ALLOW` full policy inside the same transaction. The returned `graph_refresh_result` identifies the coherent source boundary and the action taken by each private stage. Whether `pg_trickle` used differential maintenance or a full relational recomputation does not change the resolver path.
3. It reads the complete terminal evidence relations, active stewardship decisions, and durable identity ledger for the pinned definition and source boundaries.
4. The full resolver evaluates pair decisions, applies manual constraints, processes automatic edges in canonical order, and clusters every active source record under the policy in Section 8.
5. It reconciles stable IDs against the durable identity ledger, recomputes all golden values and reviews, validates every invariant, and computes the logical row-level changes to the three public output tables. Candidate overflow, missing dependency state, or resource pressure fails the refresh rather than reducing this work.
6. Before publication, it verifies the complete definition-to-artifact-to-binding chain and checks that the pinned definition, decision epoch, graph generation, and base publication revision still match. Because the entity row is locked, a mismatch should be exceptional, but the compare-and-swap check remains part of the contract.
7. It applies only the logical inserts, updates, and deletes needed to transform the public base tables, stores provenance and identity history, and advances the publication revision. It verifies the expected final pinned state and public payload after output triggers run. The caller commits or rolls back the outer transaction. Reentrant publication for the same entity fails closed.

V1 intentionally resolves the complete entity on every refresh. This is not an all-pairs comparison: `pg_trickle` maintains the bounded candidate and evidence relations, and the resolver processes only that complete logical evidence set. Full resolution keeps one executable meaning for clustering, stable-ID reconciliation, golden values, and reviews. A later release may add affected-set resolution only after differential tests prove that it produces exactly the full-resolver result for inserts, updates, deletes, edge additions and removals, merges, splits, and stewardship changes.

A canonical publication projection determines whether a refresh changes semantics. It includes the selected definition, memberships, active and retired identity state, aliases, splits, golden values and provenance, review state, and retained explanation facts. It excludes the compiled artifact and graph binding, source-boundary observations, operation IDs, timestamps, concurrency counters, the prospective revision number, and row last-change markers. A changed winning source or retained explanation fact creates a semantic publication even when the visible golden value and membership stay equal. An artifact or binding change that proves the same projection creates only an observation.

A no-change refresh records a new observation boundary and associates it with the active semantic publication, but it does not create a new semantic publication revision or update public rows. Determine semantic change before applying cleanup that depends on a new revision, including split-notice resolution and retention pruning. If a change exists, apply that cleanup and compute the final publication digest. Cleanup cannot create the revision that permits it. The original publication manifest remains immutable; `mdm.describe()` distinguishes the frontier at which the revision was first published from the later frontier through which the same result has been proven equivalent. Different entities may refresh concurrently. Only one refresh, rebuild, or activation for a particular entity may execute at a time.

Each observation also records the decision epoch it consumed. Pending stewardship status compares the current epoch with the latest successful observation, even when the immutable publication was first produced under an older epoch.

`mdm_admin.rebuild()` runs the complete private graph and full reference resolver under the desired definition and active decisions. It uses the existing identity ledger, aliases, split history, and first-membership records, so it preserves IDs according to the V1 policy. A rebuild without the original identity ledger can reproduce memberships and golden values but cannot promise the same newly allocated UUIDs.

---

## 10. Stable source and entity identity

The durable identity of a source record is the tuple `(entity_id, immutable source identity, encoded source key)`. `pg_mdm` retains this identity after the row becomes inactive so that a later reappearance of the same key refers to the same source record. A reused source name cannot rebind old records to a new relation or key contract. The public members table includes an opaque `source_record_id` and a canonical tagged JSON representation of the source key; stewardship and internal constraints use the durable internal identity rather than a temporary review row or a display string.

Each active resolved entity has one durable `mdm_id`. New entities receive a time-ordered UUID from the supported PostgreSQL generator. An unchanged component keeps its existing ID. When components merge, the ID with the earliest creation publication survives, with UUID byte order as the final tie break, and every retired ID becomes an alias of the survivor. Alias chains resolve transitively to the current canonical result, while each individual historical link remains retained for explanation.

When one component splits, the child containing the active source record with the smallest first-membership publication revision keeps the old `mdm_id`. Ties within that revision are broken by canonical encoded source-record identity. Every other child receives a new ID and records `split_from` history. This policy is intentionally simple; weighted continuity, canonical ID locks, and planned split workflows belong in V2.

`mdm.explain()` provides a machine-readable identity-resolution result for any retained `mdm_id`. It reports `active`, `merged_into`, `split_into`, `retired`, or `unknown`, together with current successor IDs and the relevant publication revisions. A merge has one canonical successor. A split may have several successors and therefore is never represented as a simple alias.

Identity history is ordered by publication. A split may retain its parent ID as one child, and a later merge may join its children again. These are valid transitions, not alias cycles. Resolve successors through increasing publication revisions and deduplicate current IDs; never recursively traverse the union of alias and split links as an unversioned graph. Alias-only cycles remain invalid. An active ID retains status `active`, with its split history and current successor set reported separately.

The same source state, definition, decisions, and identity ledger produce the same reconciliation result. This guarantee does not claim that two independently initialized databases allocate identical first-time UUIDs. It claims that rebuild and subsequent refresh preserve and transform an existing durable identity ledger according to the pinned rules.

---

## 11. Golden records and overrides

The engine selects each golden field independently after clustering. This field-level survivorship model may produce a golden record whose values did not all coexist in one source row, and the documentation must state that plainly. Users who require coherent record-level survivorship need a later policy rather than assuming that independently selected fields describe one historical source version.

The V1 built-in policies have complete tie semantics. `prefer_source` evaluates sources in the declared order, then prefers a valid candidate with the greatest declared row-change time when one is available, and finally uses canonical source-record order. `latest` requires every participating source to map a trustworthy, mutually comparable `row_changed_at` value; it orders by that value, declared source priority, and canonical source-record identity. `first_non_null` uses declared source order and canonical source-record order. `most_common` counts equal normalized values across active members, breaks count ties by the number of authoritative contributors, then by source priority and canonical normalized bytes, and chooses the published spelling from the highest-priority contributing record. A deterministic choice may still create a review when the winning margin is tied or when authoritative values are invalid.

A golden override is a steward-supplied value for one field with a required anchor `source_record_id`. The override follows the component containing its anchor through ordinary merges and splits, which avoids attaching a human value to whichever child happens to inherit an old `mdm_id`. If the anchor becomes inactive, the override becomes inactive and the normal policy is used with a review notice. If two different active overrides for the same field arrive in one component after a merge, the entity membership may still publish, but that golden field is published as `NULL` with an override-conflict review until a steward clears or supersedes one directive.

Every golden output, including an override and a deliberate `NULL` caused by conflict, stores provenance and a status. Provenance is protected by PostgreSQL permissions and the field display policy. V1 stores the winning raw value and the minimal contributing facts needed to explain the current and immediately previous publication; it does not promise indefinite retention of every non-winning raw source value.

---

## 12. Stewardship and review

V1 has three stewardship operations: `MATCH`, `NOT_MATCH`, and anchored golden override. Pair decisions attach to durable source-record identities and survive refresh. A current decision is effective when both records are active, dormant when either record is inactive, and historical after supersession. Dormant decisions participate in every write-time global contradiction check but not in active clustering. They become effective again when both records are active. Every write records the authenticated actor, selected role, reason, operation ID, base publication revision, resulting decision epoch, predecessor directive when applicable, and an optimistic concurrency version. The base revision is zero before first publication; it is not a promise that the directive has already been published. Pair decisions and override writes lock the entity before their directive, so publication cannot consume a partially changed decision set.

`mdm_steward.decide()` rejects a `MATCH` whose resulting must-link closure contains an active `NOT_MATCH`. It also rejects a `NOT_MATCH` whose endpoints already share an active must-link closure. The coherence check must complete before the write commits; a configured resource limit causes the write to fail rather than accept an unchecked constraint. The error returns a bounded contradiction chain that identifies the decisions that must be changed. `MATCH` deliberately overrides automatic evidence, including an authoritative identity conflict, but never overrides an active `NOT_MATCH`. `NOT_MATCH` is enforced at component level, so no sequence of automatic or manual unions can place the prohibited pair in one published entity.

The review queue contains unresolved ambiguous pairs, authoritative identity conflicts, conservative bridge rejections, golden ties, invalid authoritative values, anchored overrides whose source record became inactive, override conflicts, and split notices. A split notice is keyed by its parent ID, complete successor set, and split publication. It opens only for the publication that first records that split, resolves on the next semantic publication, and does not recur because the split history remains. A resolved notice leaves the public retention window only during a later semantic publication; retention age alone does not create a revision. The internal occurrence remains retained. A later distinct split receives a new issue key. Candidate overflow and incomplete required evidence are operation failures rather than reviews because V1 does not publish a result derived from truncated search.

A review has a stable issue key built from the entity definition version, reason code, and canonical subject identities. Each occurrence receives a new immutable `review_id`. If the issue disappears, that occurrence becomes resolved; if it recurs under the same semantic definition and subjects, a new occurrence uses the same issue key. A changed definition or materially different subject produces a new issue key. Review writes use optimistic concurrency so a steward cannot unknowingly act on a stale occurrence.

V1 does not expose direct merge, partitioned split, move-member, identity lock, field lock, bulk action, or invariant-override commands. Stewards express identity corrections through durable pair decisions. The absence of direct split tooling is intentional: a steward can add the necessary `NOT_MATCH` decisions, but workflows that describe an entire target partition belong in V2.

---

## 13. The five actions

### 13.1 `mdm.create()`

`mdm.create()` validates and stores a complete immutable definition. Validation checks the source relation types, frozen source identities, entity execution role, grants, row-security behavior, source-key constraints, mapped value and state columns, built-in cleaner and comparator options, authoritative field usage, candidate-plan completeness, reserved output names and columns, golden-policy inputs, and the negotiated `external_graph_refresh` capability. It stores an append-only compiled artifact. When Graph V1 is available, it also creates every member transactionally with `EXTERNAL` orchestration and initialization disabled and stores an append-only graph binding. It does not change the active public entity until `mdm.refresh()` successfully activates the desired version.

The definition row stores the user definition, expanded definition, definition digest, creator, portable execution-role name, comment, parent version, compiled logical candidate plan, and semantic version manifest. Separate append-only rows store compiled artifacts and database-local graph bindings. An idempotent retry may reuse an exact attachment; it never overwrites one. If compilation, graph creation, or validation fails, the new version and its attachments are not left partially installed.

### 13.2 `mdm.describe()`

`mdm.describe()` explains both the user model and the operational state. It shows the concise and fully expanded definitions, active and desired versions, definition digest, compiled-artifact digest, graph-binding digest, public schema compatibility, source modes and boundaries, change and deletion completeness, execution role, fields and masking policies, match roles and evidence groups, the logical candidate plan and hard limits, private graph health, member contract generations and digests, active publication revision, earliest retained explanation revision, pending definition or stewardship work, entity and review counts, last successful refresh, and any blocking error.

Entity status is one of `unpublished`, `current`, `pending`, `stale`, or `unknown`. `current` means that the active definition and decisions are published through the recorded source boundaries and no later change is currently known. `pending` means a later source, definition, or decision change is known. `stale` means such work is known and the latest refresh failed. `unknown` means that current capture health cannot prove whether later work exists; refresh still requires a proven Graph V1 boundary. These labels never replace the detailed per-source boundaries.

### 13.3 `mdm.preview()`

`mdm.preview()` has three explicitly labeled modes. `validation` expands and compiles the definition without resolving records. `sampled` runs a deterministic diagnostic sample and reports examples of pair decisions, candidate-block sizes, expected work, and local membership changes; it does not claim statistically representative global merge, split, or review counts. `scoped` resolves explicitly supplied source records or `mdm_id` values through the same pair-decision, clustering, stable-ID, and golden logic used by refresh, but in isolated temporary state. The result is exact only for the materialized induced subproblem. It makes no claim about the subjects' result or impact in full-entity resolution because omitted records and constraints may change components, identity claims, goldens, and reviews.

Preview never advances a `pg_trickle` frontier, changes the desired or active definition, writes stewardship decisions, or publishes outputs. A complete exact preview of the entire entity, user-facing labeled regression suites, precision and recall reports, schema inference, and broad stewardship impact simulation are V2 work.

### 13.4 `mdm.refresh()`

`mdm.refresh()` refreshes the private evidence graph and publishes one complete MDM revision under the transaction model in Section 9. The caller may request the desired definition or require that a particular version still be desired. The result reports the source boundaries, graph actions, terminal evidence counts, resolved component count, membership changes, merges, splits, stable-ID outcomes, golden changes, review changes, publication revision, stage timings, and whether `pg_trickle` used differential maintenance or full relational recomputation. The MDM resolver path is always full in V1.

A refresh that cannot prove source completeness, candidate completeness, graph-version compatibility, decision coherence, or final invariants fails before public output changes. The prior complete publication remains readable.

### 13.5 `mdm.explain()`

`mdm.explain()` accepts a source record, pair, active or retired `mdm_id`, golden field, review item, or retained publication revision. It returns structured facts and a readable explanation: normalized values, candidate-discovery channels, pair evidence, manual decisions, authoritative conflicts, accepted and rejected component unions, stable-ID survival, aliases and split successors, golden provenance, and review lifecycle as relevant.

When a pair was not generated, explain reports which pinned logical candidate predicates it failed. It may run one bounded on-demand comparison for diagnostics, but the result is labeled diagnostic and does not change the publication or imply that all omitted pairs were examined. Historical explanation is by immutable publication revision, not business-valid time. V1 guarantees enough retained facts to explain the current and immediately previous successful publication and reports any longer implementation-specific retention window through `describe()`.

---

## 14. Public output contract

The three public outputs are logged PostgreSQL base tables with stable names. Definition creation derives `<entity>`, `<entity>_members`, and `<entity>_review`, verifies each complete UTF-8 name is at most 63 bytes, rejects collisions after PostgreSQL folding, and reserves all three names transactionally. It rejects a pre-existing conflicting object. Golden field names cannot collide with fixed output metadata or PostgreSQL system columns. Identifier-bearing constructor inputs remain `text` until these checks complete so PostgreSQL cannot truncate them first.

The outputs are not views over a private revision pointer, because V1 explicitly supports ordinary row triggers, logical decoding, and standard PostgreSQL CDC against the named outputs. Publication applies a logical row-level diff inside one transaction and never implements a normal rebuild as `TRUNCATE` followed by complete reinsertion.

### 14.1 `mdm_out.<entity>`

This table contains one row per active `mdm_id`. Its primary key is `mdm_id`, followed by the typed golden columns in their stable creation order. Standard metadata includes `member_count`, `has_review`, `last_change_revision`, and `golden_status` metadata available through explanation. `mdm.describe()` reports the entity's current publication revision. A publication updates a row's `last_change_revision` only when that row's public payload changes. New golden columns may be appended by a later active definition, but existing public column names, positions, and types remain stable throughout V1.

### 14.2 `mdm_out.<entity>_members`

This table is the durable source-to-entity map. It contains `source_record_id`, source name, canonical tagged `source_id` JSON, current or last `mdm_id`, active status, first and last published membership revisions, concise membership reason, and `last_change_revision`. Every active source record maps to exactly one active `mdm_id`. Inactive source records may remain visible as tombstones so old source references can be explained; their stored `mdm_id` is the last published ID and may resolve through merge or split history.

### 14.3 `mdm_out.<entity>_review`

This table contains the current review queue and the limited retained status needed for optimistic stewardship. Each row has `review_id`, issue key, occurrence number, status, severity, reason code, subject references, masked summary, opened and resolved revisions, `last_change_revision`, and concurrency version. Raw sensitive values are absent unless the reader has the relevant source and field permissions and requests them through `mdm.explain()`.

One SQL statement sees one complete publication through PostgreSQL MVCC. A consumer that reads several output tables and needs one cross-table version must use one `REPEATABLE READ` or `SERIALIZABLE` transaction and verify the entity publication revision reported by `mdm.describe()`. Separate statements under `READ COMMITTED` may observe different committed publications.

### 14.4 Ordinary CDC, not a semantic change feed

Consumers may use triggers or logical decoding on the three public base tables and will observe one transactionally complete MDM publication. This is ordinary row-level database change capture. It does not explain that an ID retired because of a merge, that one parent split into several children, or that a golden change came from a particular stewardship action. A semantic `<entity>_changes` feed with ordered merge, split, alias, replay, retention, and resynchronization behavior remains a V2 feature.

---

## 15. PostgreSQL architecture, security, and operations

V1 uses the following schemas:

```text
mdm
  source, field, match, entity, golden_value
  create, describe, preview, refresh, explain

mdm_out
  <entity>
  <entity>_members
  <entity>_review

mdm_steward
  decide
  override_golden
  clear_golden_override

mdm_admin
  rebuild
  drop_entity

mdm_internal
  definitions, operations, identity ledger, provenance,
  review history, private graph registry, and resolver work tables
```

Each entity captures a portable execution-role name and a database-local binding at creation. Public entry points that validate or read sources are `SECURITY INVOKER` and require `current_user` to equal the bound execution role. For delegated use, the authenticated `session_user` must have PostgreSQL's `SET` option for that role and must run `SET ROLE` before calling `pg_mdm`. A caller cannot nominate a role that it has not selected. The extension rejects a superuser, a role with `BYPASSRLS`, and the extension owner as an execution role.

Bind the role OID at creation and compare it on each authorized action. A role recreated under the same name is a different binding and requires explicit administrator rebinding. Store role OIDs only in derived state. Administrators provision the non-login helper owner and assign ownership as part of installation and every upgrade before granting action access. The extension does not create global roles.

Narrow internal `SECURITY DEFINER` helpers perform privileged catalog writes and own private objects, but they never query source data. Every such function uses `SET search_path = pg_catalog, mdm_internal, pg_temp`, schema-qualifies every object, revokes default `PUBLIC` execution, and has a non-login least-privilege owner. V1 does not attempt `SET ROLE` inside a security-definer function. Before each source execution, the invoker path verifies the role binding, relation and column `SELECT`, row-security behavior, and the definition-to-artifact-to-binding chain. Revocation or object replacement blocks work before graph mutation or publication. Success and exception tests prove that `session_user`, `current_user`, `search_path`, and relevant configuration values are unchanged after the call. Graph V1 admission must separately prove that private defining queries preserve this identity and row-level security contract.

Every callable privileged helper independently authenticates its caller and checks the requested entity, write invariants, and grants. Prior validation in a public wrapper is not an authorization boundary. Derive actor identity from PostgreSQL's execution context, never from caller-supplied identity or a claim that validation already passed. Direct helper calls with forged arguments must fail without writes.

RLS visibility can change without source-row DML through policy changes, role membership, session settings, or relations consulted by a policy. The source binding and Graph V1 admission tests must prove invalidation or complete recomputation for each supported case. Reject a policy dependency or session-dependent scope whose visibility changes cannot be tracked or pinned. Rechecking `SELECT` privileges alone cannot validate cached evidence. This requirement follows from PostgreSQL's [row-security policy behavior](https://www.postgresql.org/docs/18/ddl-rowsecurity.html).

Administrators grant distinct capabilities to configurators, stewards, output readers, and explanation readers. Field display policy controls whether sensitive values appear as `full`, `masked`, or `hashed` in review and explanation. Plain digests of source values and candidate keys are sensitive because readers can test guesses against them. Public errors and ordinary logs contain entity IDs, source-record IDs, reason codes, and counts, but not raw values, normalized values, block keys, generated SQL, or value digests. Authorized internal diagnostics may retain digests under the same access and backup controls as normalized values.

The initial implementation inherits the deployment requirements of its supported `pg_trickle` release, including its PostgreSQL major-version target, preload requirements, and privileged extension installation. `pg_mdm` pins and tests a compatible release line rather than accepting an arbitrary newer version. It uses only the stable extension-to-extension SQL contract and treats a changed or missing contract as an explicit compatibility failure.

Every create, preview, refresh, rebuild, stewardship write, and destructive lifecycle operation receives an operation ID. A successful operation record commits with the operation and includes a stable MDM result code, concise outcome, source boundaries, graph actions, row and pair counts, component counts, identity changes, golden and review changes, stage timings, and semantic versions as relevant. After a rollback, the invoking scheduler or coordinator records failure context in a new transaction when possible; no semantic guarantee depends on failed-attempt logging surviving. V1 does not promise a formal SLO, but documentation provides a tested operating envelope for source rows, candidate-block sizes, component sizes, expected storage amplification, and publication transaction size.

The durable state that must survive backup and restore includes definitions, compiled artifacts, source identities, portable source and role names, source-record identities and tombstones, current and superseded stewardship directives, the `mdm_id` ledger, aliases and split history, active memberships, publication and observation metadata, golden provenance, and retained review facts. Every extension-member table declares its durable or derived class when introduced. Install and upgrade SQL registers durable contents with `pg_extension_config_dump`. Durable table foreign keys must have a restore-safe acyclic load order; supersession uses one-way predecessor links instead of bidirectional cycles.

Database-local role, relation, type, graph, and contract OIDs are derived bindings and are not portable. A logical restore into a clean database installs the extensions, restores durable contents, resolves qualified names to new OIDs, validates fingerprints, privileges, and row-level security, rebuilds graph bindings and private relations, validates restored public outputs or creates missing ones from the durable ledger, and compares the semantic result with the source database. Missing, ambiguous, or replaced objects fail closed until an administrator completes explicit rebinding. Physical recovery and logical dump and restore are separate tests. V1 relies on PostgreSQL for backup transport, point-in-time recovery, and streaming replication; checkpoint resume and a managed continuous restore service remain V2 work.

Runtime-created public tables are ordinary dumpable objects. Logical recovery validates and reuses restored tables, including their grants, indexes, triggers, dependent views, and replica identity. It recreates outputs only when absent. Rebuilding data must not replace an existing relation or discard its consumer objects. Public-table dump behavior and extension configuration-data restore are tested together.

`mdm_admin.drop_entity()` provides explicit entity lifecycle cleanup. It removes the private graph and public outputs and requires a privileged caller plus an explicit destructive confirmation. Entity rename is not supported in V1 because the entity name is part of the stable output contract.

---

## 16. V1 invariants

These invariants define V1 more strongly than any private table layout or Rust module boundary.

1. **Proven source boundary.** A publication names the complete `pg_trickle` source boundary from which its evidence was derived. Unknown or incompatible source state cannot be published as complete.
2. **Atomic publication.** Private graph advancement, memberships, entities, golden values, reviews, required provenance, identity history, and publication metadata commit together or not at all.
3. **Relational-maintenance equivalence.** For the same source boundary and pinned graph specification, differential and full `pg_trickle` maintenance produce the same complete terminal evidence relations. The V1 MDM resolver always processes those complete relations.
4. **Complete logical candidate enumeration.** Every pair inside the pinned logical candidate predicates is enumerated completely, or the refresh fails. A truncated search is never evidence of non-match.
5. **Deterministic, versioned semantics.** Cleaner behavior, candidate predicates, pair decisions, edge order, component admission, stable-ID reconciliation, and golden tie breaks are versioned and use complete total ordering.
6. **Coherent manual constraints.** An active must-link closure cannot contain an active cannot-link. `MATCH` overrides automatic evidence but never an active `NOT_MATCH`.
7. **Conservative component admission.** An automatic union must satisfy both pair evidence and the documented component-level rule; a one-edge chain cannot grow an established component indefinitely.
8. **Unique active membership.** Each active source record maps to exactly one active `mdm_id` within an entity.
9. **Durable identity continuity.** Merges retain one canonical successor, splits retain all successors, and retained IDs have a machine-readable resolution through `mdm.explain()`.
10. **Golden provenance.** Every published golden value or conflict `NULL` identifies its policy, inputs, decision status, and publication revision.
11. **Resource pressure fails closed.** Memory, disk, time, candidate, or component limits may reject the refresh. They may not lower evidence requirements, skip required pairs, or publish partial output.
12. **`pg_trickle` encapsulation.** No V1 guarantee depends on reading or mutating a private `pg_trickle` catalog, buffer, generated column, or storage layout.

---

## 17. Acceptance criteria

V1 is ready when a user can define one entity over several supported local or partitioned PostgreSQL tables, use tracked and soft-delete sources, map scalar or supported composite source keys, normalize common identity fields with built-in cleaners, express exact and fuzzy identity, strong, and supporting evidence, and understand the compiled candidate plan before publication. The system must resolve records without an all-pairs comparison, prevent incomplete candidate work from masquerading as non-match, publish conservative components with stable IDs, and expose golden records, membership, and review through ordinary SQL tables.

A steward must be able to record durable `MATCH`, `NOT_MATCH`, and anchored golden overrides with optimistic concurrency and explanations. The resolver must enforce cannot-link closure, make the precedence of manual and automatic evidence visible, prevent singleton chain accretion under the documented component policy, preserve IDs across no-op refreshes and documented merge and split cases, and resolve retired IDs through machine-readable explanation.

The supported `pg_trickle` release and shared conformance suite are release prerequisites. They must prove capability negotiation, canonical graph contracts, execution-role and row-security preservation, durable `EXTERNAL` orchestration, strict graph refresh, source frontier advancement, and MDM publication across the required commit boundary. Tests must cover concurrent source writes, concurrent refresh and lifecycle attempts, rollback after private graph maintenance, differential-to-full fallback, source deletes, graph-contract change, unsupported capability versions, clone isolation, and restore followed by rebuild. No code path may advance a consumed frontier while leaving the corresponding public MDM revision uncommitted.

The full resolver is the executable V1 reference. Before Graph V1 is available, production-generated SQL must run against ordinary PostgreSQL fixture sources and produce the versioned terminal relations consumed by production readers. Independent expectations cover normalization, candidates, pair evidence, and a set-based small-graph clustering oracle. After Graph V1 is available, generated sequences of inserts, updates, deletes, decision changes, edge appearance and disappearance, merges, splits, golden-only changes, and graph full fallback must be replayed through both differential and clean full graph maintenance. At every source boundary, the terminal evidence and published memberships, IDs, goldens, and reviews must agree. A no-change refresh must not create a semantic publication, and a rebuild with the original identity ledger must preserve every ID required by the V1 policy.

V1 qualification uses one internal labeled reference domain to measure candidate recall, false merges, missed matches, cluster errors, and review volume. The corpus includes shared contact details, weak chains, authoritative conflicts, deletions, reactivations, and identifiers that require business context. These measurements qualify defaults and the tested operating envelope; they do not add a user-facing evaluation product to V1.

V1 is not blocked on affected-set MDM resolution, ordinary or materialized views, complete-snapshot contracts, preset composition, review-ID reopening, generic foreign tables, `changed_at` polling, custom SQL functions, approximate nearest-neighbor search, exact global impact preview, user-facing labeled precision and recall suites, direct entity merge and partitioned split workflows, locks and bulk stewardship, downstream MDM dependency graphs, semantic change feeds, valid-time history, namespaces, quotas, resumable work, configurable retention, continuous restore verification, or formal service objectives. Adding any of those capabilities, including as a private implementation, requires explicit project-owner approval before it enters a V1 implementation plan.
