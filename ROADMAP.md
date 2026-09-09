# `pg_mdm` V1 implementation roadmap

## Purpose

This roadmap divides the V1 design into testable development releases implemented sequentially by a coding agent with human review. Only one release is in progress at a time, and the next starts after the current release meets its exit evidence. The estimates are rough-order-of-magnitude ranges, not commitments, and include implementation, tests, documentation, review, and defect fixing. Their confidence is low until working code establishes delivery velocity.

The roadmap follows [`DESIGN_V1.md`](DESIGN_V1.md). Each release should leave its completed behavior runnable and tested, while features assigned to [`DESIGN_V2.md`](DESIGN_V2.md) remain out of scope. V1 uses full-entity resolution over complete terminal evidence relations. It does not depend on the `output_delta_consumer` capability or affected-set resolution.

This repository currently contains designs only. Work on v0.1 through v0.7 can start now. The foundation baseline is `pg_trickle` v0.98.0 at commit `737927d336aabfff3b69bc2b0c29917b556c3626` (tag object `168faa71074a81ba4315bf3a162dc5c32cc914dd`). CI uses the published `pg_trickle-0.98.0-pg18-linux-amd64.tar.gz` artifact with SHA-256 `6b8a9cd3bb4761ede6150c29ba01f27eecbc2e3cffd5a85096c5683809c17fa7`; v0.1 records the image digest and full build provenance derived from it. `pg_trickle` v0.98.1 was released on 9 September 2026, but it still advertises Graph V1 and Delta V1 as disabled. It does not change the gate and is not the baseline unless a reviewed dependency change qualifies it. Live graph integration in v0.8 and v0.9 remains blocked until a released `pg_trickle` capability advertises `external_graph_refresh` major 1 as enabled and passes the shared conformance gate.

Before attaching calendar dates, v0.1 and v0.2 must establish the extension toolchain and test harness. Revise the remaining estimates from measured delivery. A milestone is complete only when its behavior is installed, runnable, and covered by its stated tests. Design or partially wired code does not count. If a milestone exceeds its upper range, re-estimate the remaining work and move optional behavior to V2 rather than silently extending V1.

Until Graph V1 passes its gate, tests use an SQL-backed production-shaped path:

```text
fixture source tables
         |
         v
production-generated SQL
         |
         v
versioned terminal relations
         |
         v
production terminal readers
         |
         v
reference resolver and publication
```

Tests may still load terminal relations directly for focused resolver cases, but those fixtures do not satisfy the integration-shaped exit evidence. From v0.3 onward, tests execute the generated normalization, candidate, pair, and evidence SQL against ordinary PostgreSQL fixture tables. They compare the terminal rows with independently constructed expectations before the production readers invoke the resolver. This remains a test arrangement, not a production bypass around Graph V1.

The initial V1 integration supports only `pg_trickle` trigger capture. WAL capture remains out of scope until `pg_trickle` advertises it as qualified. Delta V1 also remains out of scope. A later release may use `output_delta_consumer` to optimize affected-set resolution, but V1 always reads the complete terminal evidence relations after a strict graph refresh.

## Development releases

### v0.1 — Extension foundation (3–5 person-weeks)

Detailed plan: [`plans/v0.1.md`](plans/v0.1.md).

Establish the extension package, installation and upgrade scripts, internal and public schemas, roles, privileges, fixed security-definer search paths, durable operation records, and the durable-versus-derived dump policy. Declare `pg_trickle` as an extension dependency and lock the selected v0.98.0 artifact. Add one internal SQL adapter around `pgtrickle.integration_capabilities()`. On the baseline, the adapter must report `external_graph_refresh 1.0 enabled=false` and `output_delta_consumer 1.0 enabled=false` without treating either result as an installation failure. This release does not yet create or resolve entities.

Build the Graph V1 conformance harness in this release. Against 0.98.0, positive Graph V1 tests must skip or block according to the advertised capability state. Negative tests must prove that `pg_mdm` fails closed and never reads private catalogs or calls provisional internal APIs.

Exit evidence: clean install, upgrade, privilege, hostile-`search_path`, rollback, capability-discovery, dump-policy, and negative conformance tests run on the single PostgreSQL major inherited from the selected `pg_trickle` release. The test report labels skipped capability cases separately from passes. Additional PostgreSQL majors are post-V1 scope.

### v0.2 — Definitions and validation (5–7 person-weeks)

Detailed plan: [`plans/v0.2.md`](plans/v0.2.md).

Implement the entity, source, field, match, and golden-value definition model together with immutable definition versions, separate definition and compiled-artifact digests, frozen source identities, transactional output-name reservation, optimistic concurrency, and the initial `create` and `describe` surfaces. Compile each valid definition into an append-only `pg_trickle` artifact stored as data, but do not create a live graph. Prove execution-role authorization and source-query behavior under row-level security. Invalid or unsupported definitions must fail without leaving partially installed state.

Exit evidence: definitions round-trip through `describe`; A→B→A creates three history rows; only a submission equal to the current desired definition is a no-op; equivalent definitions produce the same artifact; unauthorized role nomination, privilege revocation, and row-security failures fail closed; and a clean-database logical dump and restore preserves definition meaning after explicit role and source rebinding. Re-estimate v0.3–v0.7 from the effort measured through this release.

### v0.3 — Normalization (4–6 person-weeks)

Detailed plan: [`plans/v0.3.md`](plans/v0.3.md).

Implement the V1 built-in cleaners, typed normalized-value states, deterministic ordering rules, and source-record identity handling. Execute production-generated record and normalization SQL against fixture source tables and compare it with the versioned cleaner vectors. Tests must cover supported scalar and composite keys, exact cleaner dispatch, explicit unknown and redacted inputs, invalid and absent values, authoritative fields, hostile execution context, and equivalent results across clean rebuilds.

### v0.4 — Candidate generation (6–9 person-weeks)

Detailed plan: [`plans/v0.4.md`](plans/v0.4.md).

Compile exact, composite, prefix, token, and other bounded V1 candidate channels into complete candidate blocks and canonical pairs. Enforce per-block and aggregate completeness limits so resource pressure fails the operation rather than truncating required work or treating an unexamined pair as a non-match.

Exit evidence: both the Rust generator and production-generated PostgreSQL SQL match an independent all-pairs oracle. A separate graph stage rejects an oversized block before pair-join execution. Aggregate limits fail without publishing partial results. The release records block skew, discovery overlap, unique-pair growth, peak memory and temporary storage, and a provisional pilot envelope. Re-estimate v0.5–v0.7 from the measured candidate path.

### v0.5 — Evidence and pair decisions (7–10 person-weeks)

Detailed plan: [`plans/v0.5.md`](plans/v0.5.md).

Evaluate built-in exact and fuzzy comparisons for discovered pairs, group correlated evidence, apply authority conflicts, and produce deterministic automatic pair decisions. Run generated evidence SQL through the same versioned terminal schemas and production readers used by the resolver. Add durable steward `MATCH` and `NOT_MATCH` decisions with precedence, optimistic concurrency, contradiction checks, and logical dump and restore coverage for current and superseded decisions.

### v0.6 — Conservative clustering (7–10 person-weeks)

Detailed plan: [`plans/v0.6.md`](plans/v0.6.md).

Implement the full-reference resolver, deterministic edge order, must-link closure, cannot-link enforcement, and the V1 component-admission rule that prevents weak chain accretion. Compare the production union-find result with a deliberately simple set-based oracle on small graphs. Generated tests must exercise edge appearance and disappearance, conflicting constraints, large components, and stable results under input and plan reordering. Re-estimate v0.7 and later work from measured clustering cost.

### v0.7 — Identity and publication model (8–12 person-weeks)

Detailed plan: [`plans/v0.7.md`](plans/v0.7.md).

Implement stable `mdm_id` allocation and reconciliation, merge and split continuity, aliases, golden-value selection, anchored overrides, provenance, reviews, and bounded machine-readable explanation. Publish the three V1 output-table shapes in resolver tests, without yet advancing a live `pg_trickle` graph. Deliver the release as four review gates: identity and history; goldens and overrides; reviews and explanation; then output DDL, logical diffs, and atomic publication.

Exit evidence: generated histories preserve the specified identities and produce identical memberships, goldens, reviews, and explanations under replay and input reordering. No-op and provenance-only cases follow the canonical publication projection. A clean-database restore preserves IDs, aliases, splits, overrides, review history, and retained provenance. Re-estimate integration and qualification after this release.

### v0.8 — `pg_trickle` graph integration (6–10 person-weeks after upstream availability)

Select one immutable compiled artifact for a definition version and create its private stream-table graph without rewriting either row. Create every member with `EXTERNAL` orchestration and initialization disabled. Obtain `stream_table_contract()` for each member and `graph_contract()` for the complete closure, then store an append-only graph binding with the artifact digest, database-local execution-role and source bindings, canonical graph digest, member contracts, and graph-binding digest. Add the supported lifecycle operations without reading private catalogs or calling provisional internal APIs.

This release starts only after `pg_trickle` advertises `external_graph_refresh` major 1 as enabled and passes the v0.1 conformance harness. Run the admission suite against each qualifying upstream release when it appears; do not wait for v0.8 to discover its behavior. Re-run the previously skipped positive tests as the admission gate. Private catalogs and provisional internal APIs are not substitutes for the public contract.

### v0.9 — Transactional refresh (6–9 person-weeks)

Implement `preview`, `refresh`, and administrative rebuild around strict graph refresh, complete source boundaries, full terminal-relation scans, full-entity resolution, and atomic publication. Preview modes are `validation`, `sampled`, and `scoped`; a scoped preview is exact only for its materialized induced subproblem and does not claim full-entity impact. `mdm.refresh()` locks the entity and calls `refresh_graph_strict()` inside the caller's transaction. It records `graph_refresh_id`, `source_boundary`, and `source_boundary_digest`, reads the complete terminal evidence relations, runs the reference resolver from v0.6, and publishes memberships, golden values, and reviews in the same transaction. A failure after graph maintenance must roll back graph contents, consumed frontiers, MDM state, and public outputs.

Exit evidence: success, no-op, injected failure, concurrent source-write, concurrent lifecycle, and fallback tests prove that graph and MDM state commit or roll back together.

### v0.10 — Operational hardening (8–12 person-weeks)

Complete the cross-cutting security, authorization, concurrency, lifecycle locking, crash recovery, clone isolation, backup and restore, extension upgrade, full-fallback, and resource-limit evidence started in earlier releases. Failures must use stable, actionable results and must never leave a partial publication or silently weaken candidate completeness.

### v0.11 — Release qualification (5–8 person-weeks)

Run the shared `pg_trickle` conformance suite and the generated differential-versus-full reference suite across inserts, updates, deletes, stewardship changes, merges, splits, golden-only changes, rollback, concurrency, fallback, rebuild, and upgrade. Qualify the operating envelope measured since v0.4 and complete the installation, administration, recovery, and user documentation required for a supported release.

## Release evidence traceability

Each release updates this table with the exact test name, artifact digest, and result. A blank or skipped blocking test does not satisfy its invariant.

| V1 invariant | Owning release | Required executable evidence |
|---|---|---|
| Proven source boundary | v0.8, v0.9 | Strict boundary conformance and complete terminal-relation scans |
| Atomic publication | v0.7, v0.9 | Injected rollback at every persistence and graph-refresh stage |
| Relational-maintenance equivalence | v0.3–v0.5, v0.11 | Generated SQL versus independent fixtures; differential versus full replay |
| Complete candidate enumeration | v0.4 | Rust and SQL results versus the all-pairs oracle; pre-join block-limit failure |
| Deterministic versioned semantics | v0.2–v0.7 | Canonical fixture vectors, exact version dispatch, replay, and reorder tests |
| Coherent manual constraints | v0.5 | Active and dormant contradiction tests; restored supersession history |
| Conservative component admission | v0.6 | Production union-find versus the set-based small-graph oracle |
| Unique active membership | v0.7 | Membership constraint, merge, split, and mixed-transition tests |
| Durable identity continuity | v0.7 | Rebuild, alias, split, tombstone, and clean-restore histories |
| Golden provenance | v0.7 | Selector, override, provenance-only revision, retention, and restore fixtures |
| Resource pressure fails closed | v0.4–v0.7 | Measured SQL and resolver limits with no partial publication |
| `pg_trickle` encapsulation | v0.1, v0.8, v0.11 | Capability adapter and forbidden-private-API tests |

## Upstream integration risk

| Risk | Owner | Baseline | Admission cases | Current state |
|---|---|---|---|---|
| `RISK-PGT-GRAPH-V1` | `pg_mdm` release owner | v0.98.0 commit `737927d3`; Linux AMD64 package SHA-256 `6b8a9cd3…c17fa7` | Capability absence, disabled state, major mismatch, contract canonicalization, durable `EXTERNAL` mode, source-boundary completeness, strict rollback and frontier rollback, concurrency and lifecycle locking, clone, restore, upgrade, and forbidden private access | API code is present, but Graph V1 is disabled and unqualified in v0.98.0 and v0.98.1 |

Review this risk on every upstream release and at each `pg_mdm` milestone. Record the tested artifact and unresolved conformance failures. Do not replace the gate with an upstream version-number check.

## Quality and operating evidence

Use one committed, de-identified organization-resolution corpus from v0.4 onward. It must contain labeled pairs and clusters for shared contact details, weak chains, authoritative conflicts, deletions, reactivations, and identifiers that need business context. Record candidate recall, false merges, missed matches, cluster errors, and review volume against internal acceptance thresholds before v0.6 exits. This is release evidence, not the deferred user-facing V2 diagnostics product.

The provisional pilot envelope starts in v0.4 and grows with each release. Measure source rows, block memberships and skew, repeated discoveries, unique pairs, comparisons, component checks, memory, temporary storage, publication rows and bytes, retained history, and transaction duration. Record PostgreSQL, CPU, memory, storage, data distribution, and fixture digests with every result. V0.11 qualifies the combined envelope; it does not collect these measurements for the first time.

## v1.0 release gate

V1.0 freezes the public compatibility contract only after every V1 acceptance criterion passes. The supported `pg_trickle` release must advertise `external_graph_refresh` major 1 as enabled, and the shared suite must prove canonical graph contracts, durable external orchestration, strict transactional refresh, complete source boundaries, rollback, concurrency, clone isolation, recovery, and supported upgrades. V1 does not use `output_delta_consumer` and supports only trigger capture.

The milestone ranges total 65–98 engineering person-weeks before cross-cutting rework. The published ranges total 40–59 person-weeks through v0.7 and 52–78 through v0.9. These sums are not calendar forecasts; they show why integration-shaped evidence must start before live graph integration. Because there is no implementation evidence and Graph V1 remains disabled and unqualified, use an initial funding envelope of 90–150 person-weeks rather than a single target. For one engineer sustaining 40–45 project weeks per year, that is roughly 24–45 calendar months after implementation starts. Upstream waiting time and unrelated maintenance extend that range.

Start v0.1 now and deliver v0.1 through v0.7 in order. Use separate reviewed changes for each release and the four v0.7 review gates. If Graph V1 remains unavailable after v0.7, stop before live graph execution and keep the compiler artifacts and conformance harness current. Start v0.8 only after Graph V1 passes its admission gate, then continue through v0.11. Publish a calendar forecast only after v0.2 establishes measured coding-agent delivery velocity, and revise it after v0.4 and v0.6 expose candidate and clustering cost.
