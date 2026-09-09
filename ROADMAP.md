# `pg_mdm` V1 implementation roadmap

## Purpose

This roadmap divides the V1 design into testable development releases implemented sequentially by a coding agent with human review. Only one release is in progress at a time, and the next starts after the current release meets its exit evidence. The estimates are rough-order-of-magnitude ranges, not commitments, and include implementation, tests, documentation, review, and defect fixing. Their confidence is low until working code establishes delivery velocity.

The roadmap follows [`DESIGN_V1.md`](DESIGN_V1.md). Each release should leave its completed behavior runnable and tested, while features assigned to [`DESIGN_V2.md`](DESIGN_V2.md) remain out of scope. V1 uses full-entity resolution over complete terminal evidence relations. It does not depend on the `output_delta_consumer` capability or affected-set resolution.

This repository currently contains designs only. Work on v0.1 through v0.7 can start now. Pin the exact `pg_trickle` 0.98.0 artifact after its release build completes. Live graph integration in v0.8 and v0.9 remains blocked until a released `pg_trickle` capability advertises `external_graph_refresh` major 1 as enabled and passes the shared conformance gate. Disabled Graph V1 and Delta V1 capabilities in 0.98.0 are expected states, not installation failures.

Before attaching calendar dates, v0.1 and v0.2 must establish the extension toolchain and test harness. Revise the remaining estimates from measured delivery. A milestone is complete only when its behavior is installed, runnable, and covered by its stated tests. Design or partially wired code does not count. If a milestone exceeds its upper range, re-estimate the remaining work and move optional behavior to V2 rather than silently extending V1.

Until Graph V1 passes its gate, tests populate synthetic terminal evidence relations directly:

```text
fixture evidence relations
         |
         v
    MDM resolver
         |
         +-- memberships
         +-- stable IDs
         +-- golden values
         +-- reviews
```

These fixtures represent normalized records, candidate pairs, pair evidence, and golden candidates. They let the work through v0.7 culminate in a proven full-reference resolver and publication model without executing a `pg_trickle` graph. The definition compiler still emits a deterministic `pg_trickle` graph specification as data.

The initial V1 integration supports only `pg_trickle` trigger capture. WAL capture remains out of scope until `pg_trickle` advertises it as qualified. Delta V1 also remains out of scope. A later release may use `output_delta_consumer` to optimize affected-set resolution, but V1 always reads the complete terminal evidence relations after a strict graph refresh.

## Development releases

### v0.1 — Extension foundation (3–5 person-weeks)

Establish the extension package, installation and upgrade scripts, internal and public schemas, roles, privileges, fixed security-definer search paths, and durable operation records. Declare `pg_trickle` as an extension dependency and pin the exact 0.98.0 release artifact after its release build completes. Add one internal SQL adapter around `pgtrickle.integration_capabilities()`. On 0.98.0, the adapter must report `external_graph_refresh 1.0 enabled=false` and `output_delta_consumer 1.0 enabled=false` without treating either result as an installation failure. This release does not yet create or resolve entities.

Build the Graph V1 conformance harness in this release. Against 0.98.0, positive Graph V1 tests must skip or block according to the advertised capability state. Negative tests must prove that `pg_mdm` fails closed and never reads private catalogs or calls provisional internal APIs.

Exit evidence: clean install, upgrade, privilege, hostile-`search_path`, rollback, capability-discovery, and negative conformance tests run on the single PostgreSQL major inherited from the selected `pg_trickle` release. Additional PostgreSQL majors are post-V1 scope.

### v0.2 — Definitions and validation (5–7 person-weeks)

Implement the entity, source, field, match, and golden-value definition model together with immutable definition versions, semantic digests, source-key validation, optimistic concurrency, and the initial `create` and `describe` surfaces. Compile each valid definition into a deterministic `pg_trickle` graph specification stored as data, but do not execute the graph. Invalid or unsupported definitions must fail without leaving partially installed state.

Exit evidence: definitions round-trip through `describe`, equivalent definitions produce the same graph specification and semantic digest, semantic no-ops remain no-ops, and invalid or concurrent updates leave no partial state. Re-estimate v0.3–v0.7 from the effort measured through this release.

### v0.3 — Normalization (4–6 person-weeks)

Implement the V1 built-in cleaners, typed normalized-value states, deterministic ordering rules, and source-record identity handling. Tests must cover supported scalar and composite keys, cleaner versioning, invalid and absent values, authoritative fields, and equivalent results across clean rebuilds.

### v0.4 — Candidate generation (6–9 person-weeks)

Compile exact, composite, prefix, token, and other bounded V1 candidate channels into complete candidate blocks and canonical pairs. Enforce per-block and aggregate completeness limits so resource pressure fails the operation rather than truncating required work or treating an unexamined pair as a non-match.

Exit evidence: generated small datasets match an all-pairs reference, while oversized blocks and aggregate limits fail without emitting partial results.

### v0.5 — Evidence and pair decisions (7–10 person-weeks)

Evaluate built-in exact and fuzzy comparisons for discovered pairs, group correlated evidence, apply authority conflicts, and produce deterministic automatic pair decisions. Add durable steward `MATCH` and `NOT_MATCH` decisions with precedence, optimistic concurrency, and contradiction checks.

### v0.6 — Conservative clustering (7–10 person-weeks)

Implement the full-reference resolver, deterministic edge order, must-link closure, cannot-link enforcement, and the V1 component-admission rule that prevents weak chain accretion. Generated tests must exercise edge appearance and disappearance, conflicting constraints, large components, and stable results under input and plan reordering.

### v0.7 — Identity and publication model (8–12 person-weeks)

Implement stable `mdm_id` allocation and reconciliation, merge and split continuity, aliases, golden-value selection, anchored overrides, provenance, reviews, and bounded machine-readable explanation. Publish the three V1 output-table shapes in resolver tests, without yet advancing a live `pg_trickle` graph.

Exit evidence: generated histories preserve the specified identities and produce identical memberships, goldens, reviews, and explanations under replay and input reordering. Re-estimate integration and qualification after this release.

### v0.8 — `pg_trickle` graph integration (6–10 person-weeks after upstream availability)

Use the deterministic graph specification from v0.2 to create each definition version as a private stream-table graph. Create every member with `EXTERNAL` orchestration and initialization disabled. Obtain `stream_table_contract()` for each member and `graph_contract()` for the complete closure, then persist the canonical graph digest and member contracts. Add the supported lifecycle operations without reading private catalogs or calling provisional internal APIs.

This release starts only after `pg_trickle` advertises `external_graph_refresh` major 1 as enabled and passes the v0.1 conformance harness. Re-run the previously skipped positive tests as the admission gate. Private catalogs and provisional internal APIs are not substitutes for the public contract.

### v0.9 — Transactional refresh (6–9 person-weeks)

Implement `preview`, `refresh`, and administrative rebuild around strict graph refresh, complete source boundaries, full terminal-relation scans, full-entity resolution, and atomic publication. `mdm.refresh()` locks the entity and calls `refresh_graph_strict()` inside the caller's transaction. It records `graph_refresh_id`, `source_boundary`, and `source_boundary_digest`, reads the complete terminal evidence relations, runs the reference resolver from v0.6, and publishes memberships, golden values, and reviews in the same transaction. A failure after graph maintenance must roll back graph contents, consumed frontiers, MDM state, and public outputs.

Exit evidence: success, no-op, injected failure, concurrent source-write, concurrent lifecycle, and fallback tests prove that graph and MDM state commit or roll back together.

### v0.10 — Operational hardening (8–12 person-weeks)

Complete security, authorization, concurrency, lifecycle locking, crash recovery, clone isolation, backup and restore behavior, extension upgrades, full-fallback handling, and documented resource limits. Failures must use stable, actionable results and must never leave a partial publication or silently weaken candidate completeness.

### v0.11 — Release qualification (5–8 person-weeks)

Run the shared `pg_trickle` conformance suite and the generated differential-versus-full reference suite across inserts, updates, deletes, stewardship changes, merges, splits, golden-only changes, rollback, concurrency, fallback, rebuild, and upgrade. Publish the tested operating envelope and complete the installation, administration, recovery, and user documentation required for a supported release.

## v1.0 release gate

V1.0 freezes the public compatibility contract only after every V1 acceptance criterion passes. The supported `pg_trickle` release must advertise `external_graph_refresh` major 1 as enabled, and the shared suite must prove canonical graph contracts, durable external orchestration, strict transactional refresh, complete source boundaries, rollback, concurrency, clone isolation, recovery, and supported upgrades. V1 does not use `output_delta_consumer` and supports only trigger capture.

The milestone ranges total 65–98 engineering person-weeks before cross-cutting rework. Because there is no implementation evidence and the critical integration API does not yet exist, use an initial funding envelope of 90–150 person-weeks rather than a single target. For one engineer sustaining 40–45 project weeks per year, that is roughly 24–45 calendar months after implementation starts. Upstream waiting time and unrelated maintenance extend that range.

Start v0.1 now and deliver v0.1 through v0.7 in order with a separate reviewed change for each release. If Graph V1 remains unavailable after v0.7, stop before live graph execution and keep the deterministic graph specification and conformance harness current. Start v0.8 only after Graph V1 passes its admission gate, then continue through v0.11. Publish a calendar forecast only after v0.2 establishes measured coding-agent delivery velocity.
