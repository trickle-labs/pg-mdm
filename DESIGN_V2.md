# `pg_mdm` post-V1 capability catalogue

## Advanced source contracts, stewardship, history, integration, and operations on PostgreSQL

**Status:** Post-v0.11 design with a recommended delivery scope; optional capabilities remain proposals
**Reviewed:** 14 September 2026
**Baseline:** Released `pg_mdm` v0.12.0 and the contract in [`DESIGN_V1.md`](DESIGN_V1.md). The v0.12 tag admitted pg-trickle v0.105.2; main is qualifying v0.105.3.
**Foundation:** [`pg_trickle`](https://github.com/trickle-labs/pg-trickle) remains the incremental relational engine
**Goal:** Add deeper source, matching, stewardship, history, integration, and operational capabilities without changing the small product model established by V1  
**Product contract:** The same five nouns, five actions, and three primary public outputs

---

## 1. Relationship to V1

V2 is cumulative. It starts with the complete V1 contract and adds capabilities only when a real deployment needs them. A V1 entity remains a valid and independently useful entity: it keeps its pinned cleaners, candidate predicates, pair-decision policy, clustering policy, stable-ID policy, golden policies, `pg_trickle` integration contract, and output schema until an administrator explicitly creates and activates a new semantic definition. Installing a V2-capable release must not reinterpret an existing V1 publication or silently broaden the work that V1 users are expected to operate.

The public model remains intentionally small. The five nouns are still `source`, `field`, `match`, `entity`, and `golden_value`; the five normal actions are still `create`, `describe`, `preview`, `refresh`, and `explain`; and the three primary public outputs are still `mdm_out.<entity>`, `mdm_out.<entity>_members`, and `mdm_out.<entity>_review`. V2 may add optional history, lineage, metrics, and semantic-change projections, and it may add privileged stewardship and administration functions, but it does not create another normal configuration language or require users to understand `pg_trickle` internals.

The architectural rule from V1 does not change:

> **`pg_trickle` maintains changing relational facts; `pg_mdm` decides identity.**

V2 deepens both sides of that boundary. The proposed `prepared_graph_generation` capability would let `pg_trickle` freeze one externally orchestrated graph in place with durable member leases, while the proposed optional `prepared_output_delta_binding` capability would pin terminal delta ranges. `pg_mdm` may provide richer source contracts, alternate conservative clustering policies, entity-level stewardship, stable-ID continuity policies, valid-time projections, semantic events, checkpointed domain work, and multi-entity orchestration. `pg_trickle` does not make its graph refresh resumable and does not manage MDM workers or checkpoints. Neither extension is allowed to take over the other extension's governance. `pg_mdm` never reaches into private `pg_trickle` catalogs or buffers, and custom matching code never owns clustering, identity reconciliation, invalidation, or publication.

This catalogue does not promise that every section ships. A capability enters a delivery backlog only after a deployment demonstrates the need and the project approves its prerequisites, deterministic semantics, bounded explanation, migration path, and tests. A focused release that solves that need is preferable to implementing the catalogue speculatively.

### 1.1 Released baseline and remaining work

V2 names a design generation, not a promise to release pg-mdm 2.0 or to implement every catalogue item. The recommended first scope begins with policy-assisted review routing for pg-react, followed by exact preview and regression fixtures, explicit merge and split stewardship, and a semantic change feed. These can use the existing synchronous Graph V1 path. The [roadmap](ROADMAP.md#recommended-post-v011-releases) sequences their implementation and keeps V1 acceptance work visible.

The released code and upstream contracts establish the following boundary:

| Item | State after the releases | Consequence for V2 |
|---|---|---|
| pg-mdm v0.11.0 | Synchronous strict graph refresh, full MDM resolution, identity history, and atomic publication | Reuse the compiler, resolver, and publication code |
| pg-mdm dependency | v0.12.0 pins pg-trickle v0.105.2; main updates the build lock, E2E package, and runtime version to v0.105.3 | Admit v0.105.3 only after its exact-package cumulative and populated-upgrade checks pass |
| Graph V1 and Delta V1 | Upstream v0.105.2 advertises `external_graph_refresh` 1.0 and `output_delta_consumer` 1.0 as stable and enabled | Graph V1 is sufficient for the first scope. Delta consumption still needs MDM implementation and equivalence proof |
| Prepared generations and prepared delta binding | Neither capability appears in the v0.105.2 manifest. The upstream design remains a post-1.0 proposal | Sections 3 and 4 specify a future contract, not callable released APIs |
| Capture | Upstream advertises trigger and WAL capture as stable | Keep MDM on trigger capture until its own WAL admission suite passes |
| Preview | `preview_entity()` currently returns metadata and counts. It labels `scoped` as exact without resolving the requested subproblem | Repair the existing claim, then implement shared resolution and exact impact comparison |
| Graph strategy | Candidate blocks and candidate-pair joins explicitly use `FULL` in `src/graph_spec.rs` to avoid dropped inserts previously seen on v0.105.1 | Keep these fallbacks until the exact graph regressions pass on an admitted artifact |
| Release qualification | The [v0.11 plan](plans/v0.11.md) covers paired AUTO/FULL probes, compiled MDM histories, rollback, and cumulative operational checks | Audit the broader V1 criteria and record missing quality and workload evidence before declaring v1.0 |

Upstream facts come from the [v0.105.2 capability manifest](https://github.com/trickle-labs/pg-trickle/blob/v0.105.2/docs/capability-manifest.json), [release notes](https://github.com/trickle-labs/pg-trickle/releases/tag/v0.105.2), and [prepared-generation proposal](https://github.com/trickle-labs/pg-trickle/blob/v0.105.2/plans/PROPOSAL_V2_PREPARED_GRAPH_GENERATIONS.md). The release qualifies packages, upgrades, runtime behavior, and benchmark smoke tests. Its 72-hour soak and seven-day longevity runs remain deferred. Package qualification does not prove MDM-specific semantics or make prepared execution available.

The remaining sections describe target behavior unless explicitly identified as released. A release tag is evidence of delivery, not evidence that every earlier design assertion has been implemented.

---

## 2. Capability boundary

V2 is where the system may become suitable for larger organizations, more varied source systems, more demanding stewardship teams, and downstream consumers that need more than the current three tables. The additions fall into five broad groups: richer source and time contracts; reusable and testable configuration; expert matching and clustering extensions; deeper stewardship and identity continuity; and operational features for shared, long-running deployments.

| Area | V1 baseline | V2 direction |
|---|---|---|
| Source ingestion | Local tracked and soft-delete tables | Complete snapshots, ordered change feeds, external providers, richer lifecycle states, schema contracts, replay horizons, and distributed-source proofs |
| Time | Current state at an immutable publication revision | Effective time, late corrections, source-version history, and explicitly qualified point-in-time projections |
| Configuration | One optional built-in preset with entity-local overrides | Versioned organization fragments, nested canonical composition, per-path provenance, inference, and regression fixtures |
| Matching | Built-in cleaners, comparators, and complete candidate channels | Registered custom functions, declared lookup relations, expert candidate keys, additional evidence models, and richer diagnostics |
| Clustering | One conservative policy | Several versioned conservative policies and bounded component checks |
| Stewardship | Pair `MATCH`, pair `NOT_MATCH`, and anchored golden override | Merge, split, move-member, locks, scoped invariant overrides, approvals, assignment, and bounded bulk action |
| Stable identity | One merge and split continuity policy | Weighted continuity, canonical-ID locks, durable anchors, and planned identity migrations |
| Downstream use | Ordinary SQL and ordinary PostgreSQL CDC | Semantic change feed, revision-bound entity dependencies, retained lineage, and optional external delivery |
| Execution | One synchronous outer transaction with full-entity MDM resolution | Proven affected-set resolution and optional one-transaction evidence preparation followed by checkpointed MDM resolution and atomic promotion |
| Operations | One administrative scope using PostgreSQL controls | Namespaces, quotas, fairness, service objectives, restore verification, retention policies, and deeper health diagnostics |

Optional capabilities compose through an explicit dependency graph. Each entity manifest pins every enabled capability and version, and the compiler rejects missing, disabled, or incompatible prerequisites before source work begins. `prepared_graph_generation` major 1 requires `external_graph_refresh` major 1. `prepared_output_delta_binding` major 1 separately requires `prepared_graph_generation` major 1 and `output_delta_consumer` major 1. Valid-time projections require retained source versions and a historical identity mode, and resolved entities as sources require the semantic-feed replay, retention, and resynchronization contract. `mdm.describe()` reports the enabled capabilities, their prerequisites, and any capability for which the entity is ineligible. An optional capability may be unavailable; it may not silently degrade into a weaker result.

No V2 feature may weaken the V1 fail-closed rules. A richer source adapter cannot turn an unproven watermark into completeness. A custom candidate generator cannot use truncation as proof of non-match. A bulk stewardship action cannot accept an incompletely evaluated population. A workload budget cannot lower a match threshold. More capability means more explicit contracts, not more opportunities to guess.

---

## 3. Architecture and the richer `pg_trickle` contract

The V1 implementation compiles each immutable entity definition into a private `pg_trickle` graph and refreshes that graph inside the same transaction that resolves and publishes the MDM entity. V2 keeps that path as the default for small and medium entities because it is simple and has a strong atomicity story. V2 proposes a prepared execution path when MDM resolution cannot reasonably fit in one transaction. The evidence graph must still refresh within one transaction. This path is blocked on upstream capability delivery and must preserve the synchronous result.

Later V2 releases may optimize the transactional path with affected-set MDM resolution. The full resolver remains the reference semantics. An affected-set implementation requires `output_delta_consumer`, must consume every typed batch or respond to `FULL_INVALIDATION` with full resolution, define and prove its closure rule, and pass generated equivalence tests for evidence changes, merges, splits, and stewardship decisions before it can publish results. Durable terminal deltas are not a prerequisite for prepared full resolution.

The extension-to-extension boundary remains a versioned SQL contract. `pg_mdm` discovers `prepared_graph_generation` and, when used, `prepared_output_delta_binding` through `pgtrickle.integration_capabilities()`. It pins supported capability majors and minimum minors in the entity manifest and rejects an absent, disabled, or incompatible combination before source work begins. It uses only the public preparation, opening, inspection, verification, promotion, abandonment, and optional consumer-binding operations.

A prepared graph generation is a complete private evidence state associated with one immutable definition, one execution role, one source-boundary manifest, one graph digest, pinned member contracts and content epochs, and the applicable `pg_trickle` row-identity and probe versions. `pgtrickle.prepare_graph()` performs one ordinary strict graph refresh in the caller's transaction, then freezes the current stream-table storage in place with one durable lease per member. Capability major 1 does not clone storage or permit overlapping prepared generations. Later source changes remain pending until the lease is released. The prepared rows are already in the private stream tables, but `PREPARED` means that `pg_mdm` has not accepted them as a public identity result.

Promotion is the critical boundary. The final `pg_mdm` publication transaction verifies its compare-and-swap conditions, publishes memberships, goldens, reviews, provenance, semantic events, and identity history, and calls `pgtrickle.promote_prepared_graph()` with the expected generation digest. Promotion records acceptance and releases member leases in the same transaction. With prepared delta binding it also applies the exact required acknowledgements. A rollback leaves the generation `PREPARED`, leases active, consumer cursors unchanged, and the old MDM publication complete. A superseded or failed run calls `pgtrickle.abandon_prepared_graph()` with a reason; abandonment releases leases without acknowledging deltas or reversing the private graph.

The richer integration contract also permits `pg_trickle` to maintain relational facts that are not direct source projections. Examples include source-version tables, deterministic lookup joins, candidate-frequency statistics, valid-time interval projections, and downstream MDM dependency inputs. These are still relational facts. The final decisions about pair classification, component admission, manual precedence, stable identity, review state, and publication remain in `pg_mdm`.

---

## 4. Prepared and resumable refresh

The proposed prepared profile would make MDM resolution resumable for entities that exceed the practical transaction duration, memory, temporary-storage, or lock envelope of V1. It does not make the `pg_trickle` graph refresh resumable. `pgtrickle.prepare_graph()` must still complete one strict graph refresh in one PostgreSQL transaction. If relational evidence construction cannot fit that envelope, the entity is ineligible for this capability major and needs a later graph-execution design.

The preparation transaction uses a durable MDM run ID as `request_id`, calls `prepare_graph()` with the private roots and expected graph digest, and stores the returned `prepared_generation_id`, `generation_digest`, graph refresh ID, source boundary, member contracts, and optional consumer bindings in the run manifest. The generation and MDM run record commit together. Request identity makes a retry after an uncertain connection result idempotent.

The MDM run state remains:

```text
planned
    → preparing_evidence
    → evidence_ready
    → resolving
    → validating
    → ready_to_publish
    → published

Any pre-publication state may instead become superseded, failed, or abandoned.
```

`evidence_ready` corresponds to a `pg_trickle` generation in `PREPARED`. The `pg_trickle` state machine is separate and smaller: `PREPARED` transitions to `PROMOTED` or `ABANDONED`; integrity loss changes it to `INVALID`, which can only be abandoned. An old generation never expires, promotes, or abandons itself.

The manifest also pins the entity definition, active publication revision, stewardship decision epoch, source contracts, semantic engine versions, identity-ledger digest, and every mutable dependency needed by custom code. Source changes committed after the prepared boundary do not invalidate the run; they remain pending for a later graph refresh. A changed desired definition, new stewardship directive, changed lock, incompatible function dependency, or replaced base publication normally supersedes the MDM run even when the prepared evidence remains valid.

Every processing transaction calls `pgtrickle.open_prepared_graph(prepared_generation_id, expected_generation_digest)` before reading member or bound delta relations. The call validates the generation and takes a transaction-scoped shared transition lock. Durable member leases prevent graph mutation between transactions; the shared lock prevents promotion or abandonment during a read. `pg_mdm` never reads a prepared generation without opening it.

Resolution work is divided into deterministic ranges such as canonical source-record intervals, candidate-block intervals, component roots, or stable run-local component identifiers. A checkpoint records only completed work whose inputs are named by the run manifest and whose outputs are stored in run-scoped logged tables. Replaying a completed range is a no-op, and worker order cannot affect edge ordering, component decisions, or stable-ID reconciliation. If a work range cannot be made idempotent and independently verifiable, it is not eligible for checkpointing and must be recomputed.

Multiple MDM workers may claim ranges with coordinator-owned leases, but those leases only allocate work. `pg_trickle` separately owns one durable lease for every prepared graph member, and those leases provide evidence isolation. A crashed worker's range lease expires, and another worker repeats or verifies the range. The number of workers, batching, query plans, and physical indexes may change performance but not the logical result.

Before publication, `pg_mdm` may call `pgtrickle.verify_prepared_graph()` to distinguish prepared validity from future refresh continuity. The final publication transaction performs the MDM compare-and-swap, applies logical row changes and required history, and calls `pgtrickle.promote_prepared_graph()` with the exact generation digest and, when enabled, one required acknowledgement per prepared consumer binding. If any statement or the transaction fails, the old publication remains complete and the generation remains `PREPARED` for retry or abandonment.

Abandonment is explicit. A superseded or failed run records its reason and calls `pgtrickle.abandon_prepared_graph()` in the same transaction. The call releases member leases but does not acknowledge output deltas and does not roll private stream tables back to their prior contents. A later run consumes the cumulative delta since the last promoted publication or performs a full resynchronization.

A deployment does not have to enable prepared execution. The V1 single-transaction path remains the simplest and strongest default. `mdm.describe()` reports capability versions, generation state, prepared age, future continuity, pending source growth, and why a particular entity is eligible or ineligible for the prepared profile.

---

## 5. Full source contracts

V1 source modes are intentionally small presets. V2 may generalize them into a complete source contract when a deployment needs complete snapshots, ordered events, replay, changing row scope, external snapshot tokens, late correction, or lifecycle states beyond active and inactive. The contract is compiled into ordinary private graph inputs and `pg_trickle` source capabilities; it is not a second ingestion engine inside `pg_mdm`.

A full source contract describes the identity and scope of source records, the mechanism that proves change completeness, and the interpretation of source versions. It may declare the stable-key columns and key-reuse policy, source row scope and filter semantics, schema and collation requirements, lifecycle field, event or snapshot identity, source sequence, event ID, effective interval, observed time, completeness watermark, duplicate handling, correction behavior, replay horizon, and the `pg_trickle` adapter or locally materialized landing relation that supplies the data.

The V1 modes remain named presets in this larger model, while snapshot becomes a V2 contract that requires a `pg_trickle` capability able to prove its boundary:

| Mode | Source-contract interpretation | Release boundary |
|---|---|---|
| `tracked` | A current local relation with exact `pg_trickle` change capture and explicit hard deletes | V1 |
| `soft_delete` | A tracked current relation with a retained lifecycle predicate in addition to hard-delete capture | V1 |
| `snapshot` | A complete replacement relation with a snapshot identifier that proves the scanned row scope is complete | V2 |

V2 may additionally support an `ordered_changes` contract. This contract consumes an append-only or update-safe local landing relation whose rows contain a stable event ID, source-record key and incarnation, a source sequence that totally orders events for that record, lifecycle action, payload or changed fields, and a completeness watermark. A global order across independent records is not required. Events for one source-record incarnation that cannot be totally ordered are rejected; arrival order and timestamps are not tie breaks. The watermark means that every event in the declared source scope through that boundary has been delivered; it is not merely the greatest timestamp seen. Duplicate delivery is a no-op, and a correction explicitly names the source event or source version it supersedes.

External systems are normally integrated by landing their snapshots or change events in local PostgreSQL relations and letting `pg_trickle` maintain the downstream facts. A direct foreign or distributed relation may be accepted only through a provider capability that proves how data, snapshot position, and completeness belong together. If one complete snapshot cannot cover every relation involved in a source contract, the source is reported as unaligned and cannot support a publication that claims complete current state.

V2 may distinguish lifecycle states such as `active`, `deleted`, `deactivated`, `superseded`, `temporarily_absent`, and `outside_scope`. Each state has an explicit effect on matching eligibility, current membership, golden-value eligibility, history, and reappearance. A record that temporarily leaves an extract must not become deleted merely because it is missing, and a record outside a newly narrowed scope must not be confused with a source-level delete. Changing row scope is therefore a semantic definition change and usually requires a complete reconciliation.

Key reuse remains forbidden by default. A source that legitimately reuses keys must provide a source generation, incarnation, or version boundary that makes the new record distinguishable from the prior source-record identity. The generated identity becomes part of the source contract and must be available to stewardship and historical explanation. Adopting key reuse for an existing source is an incompatible source-identity migration unless every retained source record can be assigned an incarnation without ambiguity. Each retained `source_record_id`, directive anchor, and historical reference remains bound to one explicit incarnation; later incarnations receive new source-record IDs. If that binding cannot be proved, activation is rejected. Guessing that a reappearing key represents the same real-world record is never acceptable.

---

## 6. Publication time, observed time, and valid time

V1 answers a clear current-state question: what did `pg_mdm` publish from the source state represented by a particular publication revision and source frontier? V2 may add valid-time behavior, but it keeps publication history and business-effective history separate because they answer different questions.

The system may expose three related times. `publication_revision` identifies an immutable MDM result. `observed_at` or the source frontier identifies when the relevant source version became known to the database. `effective_at` and `effective_until` describe when the source says that version was true in the business domain. These values can differ, and every historical query must say which axis it uses.

A late correction creates a new source version and eventually a new MDM publication. It may change what the current system believes was effective at an earlier business time, but it never rewrites an old publication. A query for “what did we publish at revision 120?” returns the immutable result from revision 120. A query for “what does revision 180 now say was valid on 2025-06-01?” may return a corrected historical projection, clearly labeled as a current belief evaluated from revision 180.

Point-in-time membership and golden projections are optional V2 surfaces rather than changes to the three current-state outputs. An implementation may expose functions or tables under an `mdm_history` schema, but every answer includes the publication revision used as knowledge, the requested valid time, and a completeness status. If required source versions, evidence, or identity history have expired, the query returns `incomplete_history` with the missing horizon instead of inventing an answer.

Historical identity is explicit. A projection that reproduces an actual publication returns the durable `mdm_id` values from that publication. A corrected valid-time partition that never existed as a publication uses a projection-scoped `historical_component_id` derived under the pinned historical policy. It may report related durable IDs for explanation, but it does not create aliases, claim stable-ID continuity, or mutate the current identity ledger. The identifier is meaningful only with its knowledge revision, requested valid time, and definition version.

Valid-time resolution can be expensive because a correction may change candidate evidence, clusters, and golden values across an interval. `pg_trickle` may maintain source-version and interval relations incrementally, while `pg_mdm` computes affected historical components under the same deterministic policies used for current state. A deployment may choose to retain source versions without enabling full historical clustering, or may enable history only for named entities or fields. The advertised history capability must match what the retained data can actually answer.

---

## 7. Configuration composition and provenance

V1 supports one optional versioned built-in preset with entity-local overrides. V2 may add organization fragments after repeated entity definitions show that users need to share source conventions, field definitions, evidence groups, masking rules, golden policies, or expert candidate settings. A fragment is a versioned partial definition, not a sixth product noun and not a runtime entity.

Every reference to a preset or fragment pins an exact version. The compiler rejects cycles, expands references before semantic validation, and requires an entity-local resolution for every conflicting scalar or incompatible collection item. There is no implicit last-writer-wins rule, and one entity never inherits the mutable state of another entity. Built-in presets, organization fragments, inferred proposals, and direct declarations all pass through the same canonical expansion path.

The fully expanded definition remains the semantic input to graph compilation. `mdm.describe()` may additionally show per-path provenance, allowing an operator to see that a phone cleaner came from a company-wide contact fragment, a candidate bound came from a regulated-customer preset, and a golden source preference was overridden locally. Updating a fragment does not mutate any active entity. Adoption creates a new entity definition version, a new private graph generation, and an ordinary preview and refresh workflow.

The first V2 implementation should prefer flat reusable fragments or one nested level. Deep fragment graphs should be added only when actual organizations cannot manage their definitions without them. More nesting increases the burden of conflict explanation, migration review, and reproducibility, so the compiler must continue to emit one readable expanded definition regardless of composition depth.

---

## 8. Preview, inference, and regression testing

V2 expands preview from local diagnostic sampling into a complete configuration-verification surface. It still uses the normal compiler, private graph stages, pair-decision engine, clustering engine, stable-ID reconciler, and golden selector. Preview is never allowed to grow a second approximate implementation whose behavior drifts from publication.

Every preview result is labeled with one of a small number of evidence levels:

```text
validation
sampled
bounded_estimate
exact_subjects
exact_entity
unknown
```

A full exact preview creates or reuses a separate private graph for the proposed definition, resolves the complete requested source boundary, and compares the result with the active publication without publishing it. For large entities it may prepare that graph and use checkpointed MDM resolution, but `prepared_graph_generation` major 1 still permits only one active prepared lease per stream-table member and does not clone member storage. The preview reports exact pair, merge, split, stable-ID, golden, review, output-schema, semantic-event, storage, and execution effects, and its MDM-owned results expire according to preview retention policy.

V2 may add labeled pair, entity, and golden fixtures. A pair fixture records expected evidence or pair classification for named source records. An entity fixture records an expected partition, forbidden co-memberships, required anchors, or expected stable-ID outcome. A golden fixture records the expected selected value, provenance class, or conflict. Fixtures run against proposed definitions and engine upgrades, but they do not become live stewardship decisions unless a user explicitly records a corresponding directive.

When labels are sufficiently representative, preview may report precision, recall, false-merge, false-split, conflict, and golden-selection metrics. The report states the label population, coverage, sampling method, and confidence limitations. It never calls a number globally representative merely because it was computed from a convenient sample. Rare dense blocks and high-risk authoritative conflicts receive separate coverage reporting because they are often the cases that matter most.

Schema inference may propose source keys, field mappings, cleaners, evidence groups, authority, candidate channels, and golden policies. Inference produces an ordinary proposed definition with confidence and rationale. It never creates or activates an entity automatically. A user can edit the proposal, run fixtures and preview, and then submit it through `mdm.create()` like any other definition.

Stewardship preview also becomes richer. Before a merge, split, move, lock, or bulk directive is accepted, preview can compute the exact affected closure, resulting memberships, ID continuity, golden changes, review changes, semantic events, and downstream dependency impact. Consequential actions may require that the write name the exact preview digest it is accepting.

---

## 9. Registered semantic extensions

V1 deliberately ships only extension-owned cleaners, comparators, and golden selectors. V2 may add registered semantic extensions for deployments whose identity rules cannot be represented safely with the built-ins. Custom behavior is accepted only through narrow function contracts whose inputs, outputs, dependencies, resource bounds, and versioning are fully declared and statically verifiable.

Illustrative contracts include:

```sql
(raw_value anyelement, options jsonb)
    -> mdm.cleaned_value

(left mdm.cleaned_value, right mdm.cleaned_value, options jsonb)
    -> mdm.match_evidence

(value mdm.cleaned_value, options jsonb)
    -> text[]

(left mdm.component_summary, right mdm.component_summary, options jsonb)
    -> mdm.component_check

(candidates mdm.golden_candidate[], options jsonb)
    -> mdm.golden_choice
```

These functions correspond to cleaners, pair comparators, bounded candidate-key generators, component checks, and golden selectors. Registration records the exact signature, owner, language, volatility, parallel-safety declaration, option schema, output cardinality, cost class, maximum input and output size, deterministic test vectors, canonical executable fingerprint, statically resolved transitive function dependencies, supported PostgreSQL versions, collation dependencies, and any required extension versions. Unknown or changed dependencies block refresh until a new definition version is created.

Administrator-provided code is eligible only when the registrar can inspect its executable form and resolve its complete transitive call graph. Registration is restricted to privileged roles and allowlisted languages, schemas, operators, casts, and callees. PostgreSQL volatility and parallel-safety declarations and deterministic test vectors are evidence for review, not proof of purity. Dynamic SQL, unresolved dispatch, opaque user-supplied native code, security-definer functions, network access, undeclared relation lookups, clock access, random state, sequence use, session-state dependence, and side effects are rejected. A declared mutable lookup relation should normally be represented as an explicit source to the private `pg_trickle` graph and joined into the function's arguments, rather than queried invisibly from inside the function. This makes invalidation and source completeness visible.

A custom candidate-key function returns a bounded list of deterministic keys for one normalized record. It does not enumerate arbitrary pairs or control join order. A custom component check receives a bounded, versioned summary chosen by the engine and returns an accept, reject, or review result with reason codes; it does not mutate components or choose stable IDs. A custom golden selector chooses among supplied candidates and returns complete provenance; it does not query the network or write an external system.

Embeddings, reference data, and model outputs may participate when they have been materialized into versioned PostgreSQL relations with explicit producer and model versions. Live model calls during refresh are unsupported because a network response cannot be replayed or bounded like a relational input. `pg_trickle` may maintain deterministic relational projections over the landed model data, while the model version and source boundary become part of the MDM manifest.

---

## 10. Advanced candidate discovery and matching

V2 may add richer built-in candidate channels such as phonetic keys, character n-grams, locale-specific token families, address components, geospatial cells, deterministic vector buckets, and composite source-specific blocks. Every channel remains a logical semantic plan whose predicate, normalization, bounds, tie order, overflow behavior, and compiler version are pinned in the entity definition.

Candidate channels have one of two roles. A `complete` channel promises to enumerate every pair inside its declared logical predicate and may support automatic identity decisions. A `supplemental` channel proposes additional pairs for review or diagnosis but does not let absence count as negative evidence. This distinction permits carefully bounded approximate retrieval to help stewards without pretending that an approximate nearest-neighbor result is a complete identity search.

An approximate index or planner-dependent top-k search is therefore supplemental by default. It may add review candidates, rank a preview sample, or help a steward find a likely counterpart, but it cannot make a publication claim that all required matching pairs were considered. A deployment that needs vector evidence for automatic matching must use a deterministic complete predicate, such as an exhaustive bounded block or an exact radius relation whose completeness is proven by the selected execution contract.

V2 may offer more flexible oversized-block behavior than V1, but every option remains explicit. A channel may fail, route the complete block to a specialized exact strategy, or apply a deterministic partition that is mathematically proven to preserve the declared candidate predicate. It may not drop the least convenient rows, keep only the first `N`, or turn a runtime memory limit into a published set of non-matches. Operational resource failure still aborts publication.

Evidence aggregation may become more expressive through field-specific negative evidence, source-pair calibration, source authority, and versioned decision tables. The final pair result remains decomposable into named evidence groups and fixed-point values. A user must be able to see which evidence decided the pair, which evidence merely ranked it, which disagreements were ignored, and which rule version was active. Statistical models may contribute evidence only when their model artifact, feature definition, fixed decision rule, and complete input versions are pinned and explainable.

---

## 11. Versioned conservative clustering policies

V1 has one component-admission policy. V2 may add several conservative policies when observed workloads show that one rule cannot safely serve every entity type. A policy might require stronger multi-edge support for people, permit a durable registry anchor for companies, enforce source-specific cardinality constraints for products, or apply a registered bounded component check for a regulated identifier.

Every policy preserves the core governance of the resolver. Edge order is deterministic; active cannot-links are enforced at component level; authoritative identity conflicts are checked unless explicitly overridden by stewardship; component summaries are bounded; every accepted and rejected bridge receives a reason; stable-ID reconciliation runs after the final partition; and incremental resolution must equal the full reference result for the same manifest.

Policy choice is a semantic definition field and is pinned by version. Adding a new policy does not change existing entities. Migrating from the V1 policy to another policy requires exact preview of the expected merges, splits, reviews, and stable-ID results, followed by an explicit definition activation. A policy upgrade that changes only an implementation optimization may preserve the semantic version only when differential tests prove that the logical result is identical over the supported input domain.

Custom component checks can enrich a built-in policy, but there is no unrestricted custom clustering entry point. The engine owns must-link and cannot-link closure, edge ordering, component construction, contradiction detection, ID continuity, invalidation, and publication. This keeps a custom check from silently bypassing stewardship or producing a partition that the engine cannot explain or rebuild.

---

## 12. Expanded stewardship

V2 adds entity-level stewardship when pair decisions become too awkward for real operational work. The pair-level `MATCH`, pair-level `NOT_MATCH`, and anchored golden override remain valid primitives, but a steward may also request a merge of named entities, a split into an explicit member partition, movement of one source record to a target entity, a lock on identity or membership, a lock on one or more golden fields, or a narrowly scoped override of an automatic invariant.

All identity-changing stewardship actions append immutable directives. They never edit the three public output tables directly. A merge directive compiles to explicit membership requirements and planned ID continuity. A split directive names the complete intended partition of the current members and compiles to must-link and cannot-link constraints sufficient to preserve that partition. A move-member directive is represented as a bounded removal from one component and admission to another. Unlocking, clearing, or correcting an action supersedes an earlier directive rather than deleting history.

Every merge, split, and move directive names an expected base publication revision and durable source-record identities. Entity IDs in the request are lookup handles resolved at that revision, not the stored subject of the directive. Acceptance expands the request against the complete base membership and stores the canonical compiled constraints and continuity plan. If membership changed since the named revision, the write fails and requires a new preview. A split constrains the named partition only; later source records follow the automatic policy unless a separate lock explicitly declares a future-membership scope. A lock states whether it protects a canonical ID, a fixed source-record set, or future membership. No directive acquires future scope merely because it named an entity that later merged, split, or gained members.

V2 formalizes decision precedence:

1. Non-overridable structural integrity and source-record uniqueness.
2. Active entity, membership, and field locks.
3. Active stewardship directives, pair decisions, and explicitly scoped invariant overrides.
4. Automatic component invariants and authoritative identity conflicts.
5. Automatic identity, strong, and supporting evidence.

Timestamps record history but do not resolve contradictions. A newer directive does not silently defeat an older one unless it explicitly supersedes it. Before accepting a directive, the engine reduces the affected instructions to must-link, cannot-link, membership, lock, and golden constraints and checks the complete affected closure. A rejected action returns a bounded contradiction chain and names the directives or locks that must be superseded.

Some automatic invariants may be manually overridden, but the override must name its scope and reason. For example, a steward may merge two entities despite conflicting registry identifiers when they have verified a source error. The override does not disable the same invariant for other entities, and it does not override an active `NOT_MATCH` unless that `NOT_MATCH` is explicitly superseded. Structural conditions such as one active source record belonging to two active entities are never overridable.

Review becomes a workflow rather than only a queue. V2 may support assignment, ownership, due dates, approval requirements, comments, attachments by reference, and organization-specific reason codes. Priority can consider authoritative-source involvement, component size, downstream impact, stable-ID risk, age, and business criticality, but the score is decomposable in `mdm.explain()` and cannot change identity by itself.

Bulk action is allowed only over a reproducible predicate evaluated at a named publication revision. The action must be homogeneous, bounded, previewed, and accompanied by aggregate impact plus representative explanations. If the complete population cannot be evaluated or the preview expires because its source or decision boundary changed, the bulk write is rejected. Large actions may use the prepared execution path, but the directives become active atomically.

### 12.1 Policy-assisted review routing

The [MDM integration plan](../pg-react/plans/PLAN_PG_MDM_STEWARDSHIP_INTEGRATION.md) and [React integration plan](../pg-react/plans/PLAN_PG_REACT_MDM_STEWARDSHIP.md) establish a current use case for assignment, due dates, escalation, and receipts. Their proposed `MDM-STEWARDSHIP/1` contract is owned by MDM. The [roadmap handoff](ROADMAP.md#pg-react-stewardship-integration) schedules contract freeze, implementation, and joint qualification. All interface names below remain proposed until those gates pass.

MDM maintains a logged, permission-controlled `mdm_steward.policy_cases_v1` table and allocates a persistent unique `case_key bigint` for each UUID review occurrence. Keys are never hashes or reused identifiers. Preserve the occurrence's opening time and closed mapping through restore and the replay horizon. Existing occurrences need an auditable timestamp backfill; an unknown opening time blocks automatic deadlines. Publish evidence fields with the MDM publication transaction. Queue, deadline, and escalation controls change in their own audited control transaction. Tokens distinguish review version, definition, evidence basis, action-driving revision, publication revision, and stewardship epoch. Pending stewardship follows successful publication observations, including no-change observations.

React submits only `ASSIGN_QUEUE`, `SET_DUE_AT`, and `ESCALATE` through the proposed `mdm_steward.submit_policy_intent(...)`. MDM authenticates the caller, deduplicates `(binding_id, request_key)`, and validates new requests against a locked case and automation binding. The binding pins permitted actions and the active policy digest. MDM enforces manual assignment protection and deadline/escalation limits, then commits the control and `mdm_steward.policy_receipts_v1` receipt together. Identical authorized retries return the receipt; changed arguments conflict. Stale, paused, replaced, unauthorized, and out-of-horizon requests cannot apply new controls.

React owns routing evaluation, managed time, and immutable durable work. It completes local intent delivery, receipt correlation, and its database work in one transaction. It never invokes MDM refresh from a consequence. `APPLIED_CONTROL` does not close an identity case. A case closes only when a successful MDM publication resolves it. Later decision support must distinguish acceptance pending publication from applied publication. Receipts, audit timestamps, and unrelated publication changes cannot create repeat actions. An idle overdue case must escalate once per allowed level without source edits.

The initial deployment uses one administrative scope, a shared PostgreSQL 18 database, trigger capture, scheduler off, `READ COMMITTED`, and independently owned graphs. React reads an authorized non-RLS metadata projection because its evaluated sources currently reject RLS. Keep sensitive evidence under MDM permissions and reject unsupported scopes. Never use a view or `BYPASSRLS` to bypass this restriction. Neither extension becomes a mandatory dependency of the other, and human stewardship works without React.

This routing contract needs no exact entity preview, semantic feed, Delta V1, prepared execution, or general workflow engine. Optional M3 approval support adds exact-action proposals and authenticated distinct human approvals before React R4. The automation role cannot supply human votes, call human decision/override APIs, or lower administrator requirements. Automatic identity decisions, bulk actions, and direct merge/split by the worker require a separate future contract. Restore, clone, and policy replacement disable new automation until bindings, deduplication, and case bases are reconciled.

---

## 13. Stable-ID continuity policies

V1 uses one simple rule for deciding which `mdm_id` survives a merge and which child keeps an old ID after a split. V2 may add continuity policies for deployments where downstream references make the V1 oldest-ID and oldest-member rules unnecessarily disruptive. These policies are still deterministic and do not attempt to infer an unrecorded human preference.

A continuity policy may consider shared active membership, authoritative source members, durable steward anchors, entity creation revision, manually locked canonical ID, historical membership duration, and a stable source priority. Every input has a fixed weight or ordered precedence, and complete ties use a canonical byte order. `mdm.explain()` shows the candidate IDs, the continuity inputs, the selected survivor, and the exact tie break.

A steward may lock a canonical ID before a planned merge or identify the member anchor that should preserve identity through a split. Locks are part of the directive ledger and participate in contradiction checks. Two locked canonical IDs cannot be merged until one lock is superseded or an authorized plan states which ID will retire. A split with multiple locked anchors must assign those anchors to distinct children or be rejected.

Aliases, split relationships, and machine-readable ID resolution remain mandatory. A retired ID can resolve to one merge successor, several split successors, a later chain of successors, or a final retired state. The resolver returns the path and the publication revisions at which each transition occurred. Retention may compact the physical representation, but it cannot break an advertised ability to resolve old IDs.

Changing continuity policy is an explicit semantic migration. Existing publications and historical explanations keep their original policy. A rebuild under the old policy preserves the old ledger; adoption of a new policy previews the expected continuity changes and records a new policy version in the first publication that uses it.

---

## 14. Advanced golden records

V1 selects each golden field independently using built-in policies. V2 may add field groups and coherent-record policies for cases where independently selected values can form a combination that never existed in any source. A field group may require that name, legal form, and registration address come from one source record, while contact fields remain independently selected. The policy explains both the group-level winner and the field-level provenance.

V2 may also add weighted source trust, field-specific freshness, consensus among independent sources, quality scoring, validity intervals, and registered selectors. A `latest` policy is accepted across sources only when the participating source contracts declare comparable field timestamps or a common ordered event boundary. A row-level update timestamp is not automatically a trustworthy timestamp for every field.

Golden output may carry a status such as `selected`, `conflicted`, `overridden`, `locked`, `stale`, or `historically_incomplete`. The current public value remains typed, while detailed status and provenance are available through explanation and optional lineage projections. An unresolved conflict can publish `NULL` or a policy-defined safe value only when the definition explicitly chooses that behavior; the system never silently breaks a tie by physical row order.

Golden overrides can be attached to a durable source-record anchor, a source value version, an entity-and-field lock, or an explicitly synthetic steward value. Merge and split behavior follows the directive and anchor semantics rather than only the mechanical ID survivor. A merge with incompatible locked values is blocked or reviewed. A split routes an anchored override to the child containing its anchor; an unanchored entity-level override requires explicit steward confirmation or becomes inactive with a review.

Relation-backed selectors use declared source or lookup relations maintained through the private `pg_trickle` graph. Every mutable input, model version, and rule table is named in the manifest. The selector still returns bounded provenance and cannot query an undeclared table or external service during publication.

---

## 15. Semantic downstream change feed

V1 consumers can observe ordinary row changes on the three primary output tables. V2 may add `mdm_out.<entity>_changes` for consumers that need entity-aware events such as merges, splits, member moves, and canonical-ID transitions. The feed is generated by the MDM resolver from semantic before-and-after state; it is not reconstructed later by guessing from physical table diffs.

Events are ordered by `(publication_revision, event_no)` and commit in the same transaction as the corresponding current-state publication. Every event has a stable event ID, entity name, publication revision, event number, event type, affected IDs, relevant source-record or field references, reason code, causal operation or directive, semantic policy versions, and access-controlled before-and-after digests. Raw sensitive values are absent by default and appear only in a separately protected payload when an administrator explicitly enables them.

The feed describes published semantic state, not the earlier control-plane transaction that accepted a directive or lock. Such a write makes the entity pending; its event appears only if and when a refresh publishes the resulting control state. When an enabled feed produces a semantic event, the refresh advances the semantic publication revision even if the three primary tables have no row changes. A refresh that only proves a later observation boundary emits no event and remains a V1-style no-change refresh.

Candidate event types include:

```text
ENTITY_CREATED
ENTITY_RETIRED
ENTITY_MERGED
ENTITY_SPLIT
MEMBER_ADDED
MEMBER_REMOVED
MEMBER_MOVED
GOLDEN_VALUE_CHANGED
GOLDEN_OVERRIDE_CHANGED
REVIEW_OPENED
REVIEW_REOPENED
REVIEW_RESOLVED
IDENTITY_ALIAS_CHANGED
LOCK_CHANGED
```

A merge event identifies one canonical successor and every retired ID. A split event identifies every successor and the members assigned to each child, because a split cannot be represented by a single alias. Events that summarize large member sets may refer to a retained detail relation rather than exceed a bounded row size, but the reference and retention contract are explicit.

Consumers advance a durable cursor by publication revision and event number. The feed defines retention, gap detection, idempotent replay, and full resynchronization. A consumer that falls behind the retained horizon receives a gap status and must reload a current snapshot at a named publication revision before resuming. Retention cannot silently delete events still covered by an active consumer promise.

The feed is an ordinary logged PostgreSQL table and can be consumed with SQL, logical replication, or a transactional outbox integration. An optional integration with `pg_tide` or another delivery tool may transport events externally, but `pg_mdm` owns event creation and ordering. External delivery success is not part of the MDM publication transaction unless a separately documented outbox contract says so.

---

## 16. Resolved entities as sources

V2 may allow the current output of one MDM entity to become a source for another. A downstream definition treats the upstream `mdm_id` as its stable source key, maps selected golden fields to logical fields, and names the upstream publication revision as its completeness boundary. This is useful for mastering organizations from mastered legal entities, households from mastered people, or products from mastered variants.

The compiler records a dependency edge between entities. Dependencies form a directed acyclic graph inside one administrative namespace. Creation rejects cycles and reports the shortest dependency path. An entity may depend on several upstream entities, but every downstream publication names the exact upstream revisions it consumed so that explanation and rebuild do not rely on “latest” as an unstated input.

The upstream semantic change feed localizes invalidation. Golden changes may affect only downstream evidence that uses the changed field. Membership changes may affect the upstream current row without changing its ID. A merge retires source IDs and introduces a canonical successor, while a split introduces several new source records; these cases require explicit downstream handling and cannot be inferred safely from a generic timestamp.

Refresh proceeds in topological order. An upstream entity publishes independently, after which dependent entities become pending. A downstream failure does not roll back a valid upstream publication. For small dependency groups, a future optional atomic group may publish several entities together, but the default contract is revision-bound independent publication because it is easier to operate and recover.

`pg_trickle` may maintain the relational projections between upstream output tables and downstream private evidence graphs, but `pg_mdm` owns the entity dependency DAG, revision selection, merge and split interpretation, and downstream status. Missing semantic-feed history or an unlocalizable upstream schema change forces a broader downstream reconciliation rather than an incomplete incremental update.

---

## 17. Reproducibility and resolution manifests

V1 records enough semantic information to rebuild current state under its documented horizon. V2 may add a complete resolution manifest for audit-grade explanation, prepared execution, and controlled replay. The manifest describes one proposed or published result, not merely the user definition.

A complete manifest may include:

```text
canonical expanded entity definition and digest
preset and fragment versions with per-path provenance
source-contract versions and source schema fingerprints
source snapshot, event, or frontier boundaries
upstream MDM publication revisions
source-record version references and retained payload digests
private pg_trickle graph specifications, prepared_generation_id, and generation_digest
pg_trickle integration, row-identity, planner, and semantic-plan versions
cleaner, comparator, candidate, component, clustering, stable-ID, and golden policy versions
registered function signatures, fingerprints, tests, and declared dependencies
PostgreSQL major version, database encoding, collation providers and versions
stewardship decision epoch, locks, and directive-ledger digest
base publication revision and identity-ledger digest
compiler, resolver, and manifest format versions
```

The manifest states a reproducibility horizon. Exact replay is possible only while every required source version, custom dependency, identity ledger state, and engine artifact remains available. `mdm.describe()` reports which inputs are retained, which have been compacted to digests, and which historical results can no longer be recomputed. The system never implies indefinite replay merely because publication metadata was retained.

Engine and dependency upgrades are classified as `storage_only`, `physical_plan_only`, `semantic_opt_in`, or `mandatory_correctness_repair`. A storage-only or physical-plan-only change may preserve publication hashes only after differential and full-reference tests prove the logical evidence and MDM result are unchanged. A semantic change receives a new version and requires explicit adoption. A mandatory repair records the defect, identifies affected entities, previews impact where possible, and publishes an explicit repair revision rather than pretending nothing changed.

A `pg_trickle` upgrade is evaluated through the same rules. An unchanged public integration contract is not by itself sufficient if row identity, logical candidate enumeration, source-frontier semantics, or graph-specification meaning changed. `pg_mdm` validates the stored manifests and either accepts the upgrade as physical, requires a shadow rebuild, or blocks activation until the entity adopts a new semantic contract.

Configuration changes receive an invalidation class such as `metadata_only`, `output_only`, `golden_only`, `normalization_local`, `candidate_local`, `match_local`, `cluster_global`, `source_resnapshot`, `history_rebuild`, or `incompatible`. The compiler records the classification and safe fallback so that `create()`, `preview()`, resumable execution, and `refresh()` agree about the required work.

---

## 18. Diagnostics, explanation, and service objectives

V2 may add diagnostics that help teams improve definitions and operate sustained workloads. Matching diagnostics can show value coverage, candidate count, block-size distribution, candidate yield, duplicate-channel overlap, comparison cost, automatic-match rate, review yield, authoritative conflict rate, labeled precision and recall, and records for which one rule was the only connection. These metrics describe behavior; they do not silently change candidate plans or thresholds.

Cluster diagnostics can show size distribution, source diversity, weakest accepted bridge, independent cross-component evidence, anchor compatibility, cannot-link pressure, split sensitivity, lock count, continuity risk, and the component checks that most often rejected unions. Golden diagnostics can show source win rates, ties, stale selections, override rates, invalid authoritative values, coherent-group failures, and provenance completeness. Historical diagnostics can show late-event volume, correction depth, and the portion of the valid-time horizon that remains exactly replayable.

`mdm.explain()` may provide a bounded cause graph between a source or control change and an output consequence. The explanation begins with the requested subject and expands through named source versions, candidate channels, evidence, directives, component decisions, ID continuity, golden selection, reviews, downstream events, and dependent entities. Expansion has deterministic limits and continuation tokens so that one large component cannot exhaust the backend. Sensitive values remain governed by field permissions and masking policy.

With full source contracts, `mdm.describe()` can distinguish `current`, `pending`, `stale`, `unknown`, and `historically_incomplete` using proven boundaries rather than recent timestamps. A source is current only through a declared complete boundary. An entity can be operationally healthy while having unknown future snapshot changes, and it can be operationally unhealthy while its last publication remains transactionally correct.

Optional service objectives may cover source-to-MDM lag, prepared-run duration, publication latency, failed-run age, high-risk review age, semantic-feed retention, explain latency, preview turnaround, and recovery after promotion or failover. Objectives affect scheduling, alerting, and capacity planning. Missing an objective cannot turn an incomplete source into a complete one or justify publishing a weaker identity result.

`pg_trickle` metrics and health are integrated rather than copied. `mdm.describe()` and MDM monitoring views may link an entity to the relevant private graph health, pending delta, source frontier, prepared generations, and refresh history through supported public `pg_trickle` APIs. `pg_mdm` continues to avoid private catalog dependencies.

---

## 19. Namespaces, workload control, and scale

V2 may add administrative namespaces for shared installations. A namespace binds owning roles, permitted source schemas, output schemas, trusted function schemas, masking defaults, retention policy, quotas, service objectives, encryption or digest keys, and allowed `pg_trickle` capabilities. Namespace is an operational scope rather than a sixth MDM noun.

Every entity, definition, graph generation, source-record identity, `mdm_id`, operation, run, directive, review, semantic event, lock, resource counter, and generated relation is scoped by namespace. Cross-namespace matching is impossible by construction. An explicit export and import process may move definitions or selected records between namespaces, but no query path can accidentally discover a candidate pair across them.

Workload controls may cover concurrent entities, resolver workers, candidate comparisons, temporary bytes, prepared-state storage, write-ahead log volume, publication duration, history expansion, and maintenance windows. The scheduler may reduce concurrency, pause a run, choose the V1 transactional path, choose the prepared path, or defer low-priority work. It may not lower evidence thresholds, skip complete candidate channels, disable a cannot-link, or publish only the easiest components.

Fairness is enforced among runnable work, not by changing semantic cost. A large entity can be divided into checkpoint ranges and receive a bounded worker share while smaller entities continue to progress. High-priority stewardship repairs may preempt routine refresh work, but the preempted run remains pinned and either resumes safely or becomes superseded. Resource and scheduling decisions are recorded in operation diagnostics so an administrator can distinguish semantic failure from workload delay.

Distributed execution through Citus or future `pg_trickle` capabilities is optional. A distributed source or graph is eligible only when the underlying integration contract proves complete source frontiers, exact row identity, deterministic logical candidate enumeration, and atomic or explicitly revision-bound publication. Partial worker success is never presented as a complete MDM revision.

---

## 20. Backup, restore, replication, and verification

V2 distinguishes durable identity state from rebuildable relational and operational state. Durable state includes definitions and manifests, source-record identities and tombstones, source contracts and retained versions within the promised horizon, stewardship directives and locks, the `mdm_id` registry, aliases and split relationships, active memberships, publication metadata, golden provenance, semantic events within their retention promise, and required review history. Prepared work and private graph relations are rebuildable only when their manifests and source inputs remain available.

`mdm_admin.verify()` may check extension and integration versions, relation and function references, source-contract fingerprints, collation versions, private graph specifications, prepared-generation tokens, publication hashes, active membership uniqueness, directive coherence, stable-ID resolution, semantic-feed continuity, retention horizons, and whether incremental state is trustworthy. Verification is read-only unless a separate repair action is explicitly requested.

After physical restore or point-in-time recovery, the system determines whether private `pg_trickle` frontiers and prepared runs still agree with the durable MDM publication. A verified prepared run may resume; an unverified run is abandoned. Missing or suspect incremental state forces a protected full rebuild from retained sources and the durable identity ledger. The last valid public publication remains readable until a replacement is complete.

Streaming replicas expose only transactionally complete publications. Background refresh does not run while PostgreSQL is in recovery. After promotion, the coordinator verifies extension state, source capture, private graphs, and prepared leases before dispatching new work. A staging table cannot become active merely because failover occurred.

PostgreSQL remains responsible for backup transport, WAL retention, physical replication, promotion, and infrastructure recovery. `pg_mdm` is responsible for naming the durable tables, verifying semantic consistency after recovery, and refusing to resume work when its proof is incomplete. Formal recovery objectives are optional service policies, not replacements for correctness checks.

---

## 21. Security, privacy, retention, and compaction

V2 may classify fields and internal data by sensitivity and retention. A policy can control raw source storage, normalized storage, candidate keys, review display, explanation display, semantic-event payloads, exports, history, and logs. The same logical field may be fully visible to a steward, masked to an analyst, hashed in a review summary, and absent from an external event.

Exact candidate indexes may use namespace-keyed digests when equality semantics permit, but the design must acknowledge that equality-frequency and co-occurrence patterns can still reveal information. Digest keys are operational secrets, are never embedded in portable definitions, and have an explicit rotation and rebuild procedure. Fuzzy matching often requires more revealing normalized data, so its storage and access contract must be documented rather than hidden behind the word “hash.”

Retention is defined by data class rather than one global age. Classes may include permanent identity IDs and aliases, active and superseded directives, configuration and resolution manifests, winning golden provenance, semantic events, source-record versions, normalized values, non-winning raw values, rejected pair evidence, preview state, failed-run staging, and operational metrics. Each capability states which classes it needs and for how long.

Compaction may use validity intervals, append-only events, periodic checkpoints, summarized evidence, and partitioned storage. It must preserve current output and every advertised guarantee, including old-ID resolution, active decision audit, retained point-in-time queries, semantic-feed replay, causal explanation, and the stated reproducibility horizon. A retention change that removes a capability requires preview and explicit administrative acceptance; storage pressure cannot silently delete evidence required by an active promise.

Legal holds or investigation holds may freeze selected source records, entities, directives, publications, or event ranges. Holds prevent retention cleanup but do not by themselves change current identity. Access to held data remains subject to ordinary PostgreSQL authorization and namespace policy.

Custom semantic functions and lookup relations are treated as sensitive execution dependencies. Registration, invocation, and explanation follow the namespace's verified-code policy, and raw function errors are sanitized so they cannot leak values into logs. The execution role and search path remain pinned and explicit across normal, preview, prepared, and recovery paths.

---

## 22. Actions, privileged functions, and outputs

The five normal actions remain the entry points for ordinary users. `mdm.create()` may now accept source contracts, fragments, richer matching policies, and optional capabilities, but it still stores one complete immutable definition. `mdm.describe()` may show composition provenance, source watermarks, history horizons, dependency revisions, prepared runs, namespace limits, service objectives, function dependencies, and semantic-feed state. `mdm.preview()` may run exact complete impact analysis and regression fixtures. `mdm.refresh()` may choose the transactional or prepared path according to policy and eligibility. `mdm.explain()` may answer publication-time, valid-time, directive, event, and downstream-cause questions with bounded expansion.

Privileged functions live under `mdm_steward` and `mdm_admin`. Stewardship functions may create, preview, approve, supersede, or inspect merge, split, move, lock, override, assignment, and bulk directives. Administration functions may verify, rebuild, manage namespaces, inspect retention, prune eligible data, resume or abandon runs, migrate semantic policies, and manage dependency scheduling. These functions are specialized control surfaces, not alternatives to the five normal actions.

The three primary current-state outputs remain stable. V2 capabilities may add optional projections such as:

```text
mdm_out.<entity>_changes
mdm_history.<entity>
mdm_history.<entity>_members
mdm_lineage.<entity>_golden
mdm_metrics.<entity>
```

Optional projections have their own versioned schemas and retention promises. Enabling one does not change the meaning or column stability of the V1 tables. A V1 entity can continue to expose only the three primary outputs even on a V2 installation.

Public-schema changes remain conservative. Additive metadata and new optional tables are preferred. Breaking changes to an existing primary output require a new entity contract or an explicit migration surface with dual publication and deprecation. A fragment, clustering policy, or source-contract upgrade cannot rename or retype a current golden column as a side effect.

---

## 23. Recommended delivery scope and dependencies

### 23.1 Admit the new baseline and close V1 gaps

Qualify the main branch's pg-trickle v0.105.3 pin against pg-mdm's compiled graphs, authorization rules, and publication transaction. The populated upgrade and cumulative E2E must pass before calling the artifact admitted. Reproduce the bytea candidate-block and multi-row candidate-pair insert histories before considering removal of `FULL`. A passing simple scan probe does not qualify those joins.

Audit the V1 acceptance criteria against executable tests and retained release results. Correct the current scoped-preview exactness claim and complete its promised subproblem behavior. Connect the organization corpus to runnable quality checks, add held-out cases, and measure the synchronous operating envelope. These are baseline obligations; a V2 feature cannot substitute for them.

#### Policy routing before exact preview

M0 is complete: MDM-STEWARDSHIP/1 and its shared fixture are frozen with both owner approvals in [`contracts/MDM-STEWARDSHIP-1-review.md`](contracts/MDM-STEWARDSHIP-1-review.md). Deliver M1's review projection, M2's typed routing controls and receipts, and M4 joint qualification with React R0–R3 and R5. The roadmap assigns these to MDM v0.13–v0.15. Preserve MDM's human authority and test exact policy rows, request bodies, receipts, worker outcomes, time-only escalation, publication rollback, and recovery on the real shared stack.

M3 approval requirements remain independently gated before React R4 at 0.51.0. A definition preview does not implement approval of an exact action. Identity-changing commands retain their stronger impact-preview and constraint requirements below.

### 23.2 Exact preview, fixtures, and bounded manifests

Deliver this after the initial routing integration, with no runtime dependency on pg-react. Reuse the production compiler, terminal readers, full resolver, identity reconciler, and golden selector for a non-publishing result. Extract only the shared resolve-and-compare path needed by preview. Keep output writes and durable ID allocation behind publication.

Start with `exact_entity` for local tracked and soft-delete sources that fit one transaction. A proposed definition uses a separate private graph and a proven source boundary. Pin the definition and artifact digests, base publication, decision epoch, execution role, source boundary, and semantic versions using the existing catalog metadata. New identities receive preview-local handles. Acceptance must recheck the pinned inputs; changed inputs require a fresh preview or a complete recomputation. Preview must not consume durable IDs, change directives, activate a definition, or mutate public outputs.

Use labeled pair, partition, forbidden-co-membership, and golden fixtures through the same path. Report exact merges, splits, survivor choices, goldens, reviews, and output-schema effects with bounded details. Report semantic-event effects only when the feed is implemented. Establish fixture metrics and workload limits before adding inference or an interactive diagnostics product. Large exact preview can wait for prepared execution without blocking bounded synchronous preview.

### 23.3 Merge and split stewardship

Implement explicit merge and complete split partitions using the existing immutable decision ledger and constraint checks. Require the base revision, durable source-record subjects, decision epoch, and accepted preview digest. Persist all compiled constraints in one transaction, detect contradictions over the complete affected closure, and publish through the ordinary refresh path.

Keep the V1 continuity policy for this release. Routing and manual assignment protection belong to M1/M2; approval requirements belong to the separate M3 gate. Add move-member, identity and membership locks, alternate continuity, and bulk action only with a concrete workflow and their additional precedence tests. The first merge or split release must already cover retry, stale preview, supersession, dormant records, reactivation, and clean restore.

### 23.4 Semantic events before entity dependencies

Create the optional change table from the existing semantic publication comparison. Define event order, stable IDs, payload permissions, no-change behavior, retained detail, consumer cursors, gaps, and snapshot resynchronization together. Events and current outputs commit or roll back together. Start with events for implemented operations; lock events become eligible only when locks exist.

Use ordinary SQL consumers first. Resolved entities as sources follow only after replay and resynchronization work across merges, splits, restore, and retention gaps. They then add the acyclic dependency graph and exact upstream revision bindings described in section 16. External delivery is a separate adapter.

### 23.5 Optional capability tracks

These tracks cover the rest of the catalogue. Each needs a deployment use case, an owner, a bounded initial scope, and the acceptance evidence in section 26 before entering a release plan.

| Track and design sections | Smallest useful implementation | Prerequisites and completion evidence |
|---|---|---|
| Source contracts and time, 5–6 | One local complete-snapshot or ordered-event contract | Prove scope and completeness, duplicates, per-record order, lifecycle, key incarnation, and replay. Retain source versions before late corrections and valid-time projections; test incomplete history and projection-scoped identity |
| Composition and inference, 7–8 | Flat pinned fragments with per-path provenance | One canonical expansion path, conflict rejection, and unchanged old definitions. Add inference only as a proposal verified by fixtures and exact preview |
| Matching and clustering, 9–11 | One demanded built-in channel or policy | Complete candidate oracle, bounded explanations, versioned migration, and quality evidence. Registered code waits for inspectable dependency closure and invalidation tests; approximate retrieval remains supplemental |
| Extended stewardship, 12 | One required move, identity lock, or bulk operation beyond M1/M2 routing | Exact affected-closure preview, explicit scope and precedence, contradiction checks, authorization, and immutable audit. M3 human approvals remain separately gated before React R4 |
| Continuity and goldens, 13–14 | One demonstrated continuity policy or coherent field group | Exact impact preview, merge/split and lock contradiction tests, source timestamp comparability where needed, and complete provenance |
| Revision-bound dependencies, 16 | One upstream-to-downstream entity relationship | Semantic feed replay and resynchronization, cycle rejection, merge/split handling, and independently atomic publications |
| Prepared full resolution, 3–4 and 17 | One prepared run per entity, one worker, logged checkpoints | Upstream enabled `prepared_graph_generation` with admitted public APIs; open on every read transaction, idempotent replay, compare-and-swap promotion, explicit abandonment, and recovery/equivalence tests |
| Affected-set resolution, 3 | Synchronous Delta V1 consumer with full fallback | Measured full-resolver bottleneck; proven closure for inserts, removals, merges, splits, and control changes; atomic acknowledgement; full resynchronization on invalidation or gaps. Prepared delta binding additionally requires both upstream prepared capabilities |
| Diagnostics and recovery, 18 and 20 | Public graph-health links and read-only semantic verification | Actionable failures, bounded disclosure, restore and failover checks; verify each newly promised replay horizon |
| Shared administration, 19 and 21 | Namespace isolation only when multiple administrative scopes are required | Scope every identity, graph, directive, event, and query before enabling shared use; test cross-namespace denial, quotas, and workload delay without semantic changes |
| Retention and privacy, 20–21 | Data-class retention for each enabled feed, preview, or history feature | Gap handling, active promises, held records, protected payloads, and old-ID resolution. General compaction and key rotation follow preservation tests |

Prepared execution is a separate dependency branch. Request upstream implementation and shared conformance for preparation, opening, leases, verification, promotion, and abandonment before scheduling MDM integration. Do not emulate these guarantees through private pg-trickle catalogs. Add multiple workers only after single-worker checkpoint replay is correct and measurements show useful parallelism. If graph refresh itself exceeds the transaction envelope, prepared major 1 cannot solve the problem.

Retention and authorization ship with each feature that needs them. Deferring a general retention or namespace product never permits an unbounded feed, leaked preview, or broken consumer promise.

### 23.6 Definition of complete

The initial routing integration is complete after M0/M1/M2/M4 and React R0–R3/R5 pass joint qualification. Close the broader first V2 scope after exact preview and fixtures, merge/split stewardship, and the semantic feed also pass their release gates. Optional M3 approvals remain a separate gate. Record the enabled capability versions and explicit exclusions in the release manifest. This is a scoped V2 delivery, not completion of every optional track.

To complete the entire catalogue, give every optional row above a reviewed implementation plan and demonstrate its section-26 acceptance cases in combination with the other enabled capabilities. Until then, label each row proposed, blocked, or deferred. Never mark a catalogue item complete because its upstream dependency shipped or because a private API exists.

---

## 24. V2 invariants

V2 retains all twelve V1 invariants. When the associated capabilities are enabled, it adds the following invariants:

13. **Canonical configuration expansion.** Presets, fragments, inferred proposals, and local declarations resolve to one canonical complete definition before semantic validation and graph compilation.
14. **Every mutable semantic dependency is declared.** A function, model, lookup relation, collation, extension, source adapter, or setting cannot affect identity unless its version and invalidation contract appear in the manifest. Custom executable dependencies must also be statically inspectable and allowlisted.
15. **Prepared evidence is immutable.** A resumable run resolves only the prepared graph generation and source boundaries named by its manifest; later changes remain pending and cannot leak into the run.
16. **Prepared publication is atomic.** Promotion of the prepared `pg_trickle` generation, MDM current-state tables, identity history, reviews, provenance, and semantic events commits together or not at all.
17. **Time axes and historical identity are explicit.** Publication history, observation time, and business-valid time are never silently substituted for one another. Incomplete historical answers are labeled, and a recomputed historical partition cannot mutate or impersonate the durable current-state identity ledger.
18. **Stewardship directives are coherent and durably scoped.** Merge, split, move, pair decisions, locks, overrides, and golden instructions name their base state and durable subjects and reduce to a contradiction-checked set of constraints; timestamps alone never decide precedence.
19. **Identity continuity is explainable.** Every merge or split survivor is selected by a pinned policy or explicit lock and has a machine-readable successor history.
20. **Semantic events are ordered and recoverable.** When the change feed is enabled, every current-state publication has a complete ordered event segment, and consumers can detect gaps and resynchronize.
21. **Entity dependencies are acyclic and revision-bound.** Every downstream publication names the exact upstream publications it consumed, and missing dependency history causes broader work or failure rather than guesswork.
22. **Custom code does not replace engine governance.** Extensions may clean, compare, generate bounded keys, check bounded component summaries, or select goldens; the engine owns candidate completeness, directive precedence, clustering, identity, invalidation, and publication.
23. **Retention cannot break an active promise silently.** Compaction preserves every advertised current, history, replay, explanation, identity-resolution, and audit guarantee, or the capability is explicitly downgraded before data is removed.
24. **Namespace isolation is structural.** When namespaces are enabled, every identity, graph, run, decision, output, event, resource counter, and generated relation is scoped so that cross-namespace matching or disclosure cannot occur accidentally.
25. **Operational policy never changes truth.** Worker count, fairness, deadlines, quotas, and service objectives may delay, pause, broaden, or reject work; they never weaken semantic rules or publish incomplete state.
26. **Semantic upgrades are opt-in or explicit repairs.** Existing V1 and V2 definitions retain their pinned meaning until an administrator adopts a new semantic version or applies a documented mandatory correctness repair.
27. **Capability prerequisites fail closed.** Every enabled capability and version appears in the entity manifest. Missing or incompatible prerequisites make the entity ineligible; they never produce a silently reduced result.

---

## 25. Capability allocation

This table allocates the V1 contract and proposed V2 additions. It is not an implementation-status table; section 1.1 records the released baseline.

| Area | V1 contract | V2 addition |
|---|---|---|
| Product model | Five nouns, five actions, three primary outputs | No change |
| `pg_trickle` integration | `external_graph_refresh` major 1 with strict transactional private-graph refresh | `prepared_graph_generation` for freeze-in-place leases and atomic promotion; optional `output_delta_consumer` and `prepared_output_delta_binding` for affected-set work |
| Sources | Local tracked and soft-delete tables | Complete snapshots, full contracts, ordered events, external adapters, lifecycle states, row scope, schema and replay policy |
| Time | Immutable publication revisions | Effective time, observed time, late corrections, point-in-time projections |
| Configuration | One optional built-in preset with entity-local overrides | Organization fragments, nested canonical expansion, per-path provenance |
| Verification | Validation, diagnostic sampling, scoped named subjects | Full exact preview, labeled fixtures, metrics, inference, stewardship impact |
| Matching | Built-in exact and fuzzy evidence | Registered functions, declared lookups, richer evidence, model-backed inputs |
| Candidate discovery | Complete deterministic built-in channels | Expert candidate keys, supplemental approximate discovery, richer overflow strategies |
| Clustering | One conservative policy | Versioned conservative policies and bounded component checks |
| Stable IDs | V1 merge and split rule | Weighted continuity, canonical locks, durable anchors, migration policies |
| Golden values | Independent built-in field selection | Field groups, coherent records, advanced trust and freshness, registered selectors |
| Stewardship | `MATCH`, `NOT_MATCH`, anchored golden override | Merge, split, move, locks, scoped overrides, approvals, assignment, bulk action |
| Outputs | Golden, members, review | Optional changes, history, lineage, and metrics projections |
| Downstream use | SQL and ordinary PostgreSQL CDC | Semantic events and resolved entities as revision-bound sources |
| Reproducibility | Current-state semantic manifest and reference rebuild | Complete resolution manifest, replay horizon, upgrade classes, audit-grade prepared runs |
| Execution | One outer transaction with full-entity MDM resolution | One-transaction evidence preparation, then checkpointed multi-worker MDM work with one atomic final promotion; graph refresh itself is not resumable |
| Operations | PostgreSQL roles, locks, backup, replication, and resource controls | Namespaces, quotas, fairness, SLOs, verify, failover validation, and recovery workflows |
| Storage | PostgreSQL-managed current and minimum explanation state | Data-class retention, legal holds, checkpoints, history compaction, and explicit capability downgrade |
| Security | Execution roles, protected internals, masking | Namespace isolation, sensitivity-driven storage, verified custom-code policy, digest-key management |

---

## 26. V2 acceptance

For a declared V2 release scope, acceptance requires every included capability to pass the gates in this section. Deferred catalogue items must remain explicitly excluded.

V2 is acceptable when every implemented capability remains reachable through the five normal actions or a clearly privileged stewardship or administration function, leaves the three V1 primary outputs stable, preserves existing V1 definitions under their pinned semantic versions, publishes atomically, and provides a bounded explanation of its effect. The compiler must validate each capability's pinned prerequisites before source work begins. A capability is not complete until its failure boundary, migration path, rollback or abandonment behavior, retention requirements, and interaction with incremental and full reference resolution are tested.

A source-contract feature must prove its boundary, per-record event order, lifecycle semantics, and any source-identity migration. A valid-time feature must keep publication and business time distinct and define the identity of every historical projection. A custom function must have a statically resolved and allowlisted dependency closure. A new clustering or continuity policy must be versioned and previewable. An entity-level stewardship operation must name its base publication and durable constraint scope. A semantic feed must define order, replay, retention, gaps, resynchronization, and no-change revision behavior. A downstream entity must pin upstream revisions. A checkpointed MDM run must publish exactly the result of its immutable prepared-generation manifest or publish nothing.

The test program must continue to compare incremental, prepared, resumed, and full-reference results under generated source changes, late events, corrections, definition changes, directives, locks, merges, splits, candidate overflow, worker failure, failover, retention boundaries, and `pg_trickle` upgrades. Physical execution choices may change, but memberships, ID continuity, goldens, reviews, semantic events, and explanations must remain equal for the same manifest.

V2 is not acceptable merely because every catalogue item exists. A focused set of advanced capabilities that solves demonstrated deployment needs, preserves the product boundary, and keeps V1 entities boring and stable is a better V2 than a broad platform whose semantics cannot be explained or rebuilt.
