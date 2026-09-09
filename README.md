# pg_mdm

**Deterministic entity resolution and golden records, designed to run inside PostgreSQL.**

> [!IMPORTANT]
> `pg_mdm` is currently a design-stage project. This repository contains the proposed V1 contract and the V2 roadmap, not an installable extension.

Most organizations have several records for the same customer, company, supplier, or product. Those records rarely agree perfectly: names are formatted differently, contact details go stale, source systems reuse identifiers, and one weak match can accidentally join two unrelated groups. `pg_mdm` is a proposed PostgreSQL extension for resolving those records into durable real-world entities while keeping every automatic decision deterministic, conservative, and explainable.

The project is built around a deliberate division of responsibility. [`pg_trickle`](https://github.com/trickle-labs/pg-trickle) captures source changes and incrementally maintains relational facts such as normalized values, candidate pairs, and matching evidence. `pg_mdm` decides what those facts mean: which records belong together, which human decisions take precedence, which stable ID survives a merge or split, which value becomes golden, and which uncertain cases need review. In short, **`pg_trickle` maintains changing relational facts; `pg_mdm` decides identity.**

## How it works

A user defines an entity such as `customer`, maps columns from PostgreSQL source relations to logical fields, declares which agreements count as identity evidence, and chooses how each golden value should be selected. On refresh, `pg_trickle` updates a private evidence graph and `pg_mdm` runs a conservative resolver over the complete evidence set. The resolver publishes the new membership map, golden records, and review queue in the same transaction as the consumed source frontier, so readers see either the previous complete result or the next complete result, never a half-published state.

```text
PostgreSQL source relations
            |
            v
Private pg_trickle graph
  normalized values, candidate pairs, evidence
            |
            v
pg_mdm resolver
  decisions, clustering, stable IDs, golden values
            |
            v
Public PostgreSQL tables
  mdm_out.<entity>
  mdm_out.<entity>_members
  mdm_out.<entity>_review
```

The first release keeps the public model small. Its five nouns are `source`, `field`, `match`, `entity`, and `golden_value`. Its five normal actions are `create`, `describe`, `preview`, `refresh`, and `explain`. Every mastered entity produces three ordinary PostgreSQL tables: one row per resolved entity, a durable mapping from source records to entities, and a review queue for ambiguity or conflict. Consumers can query those tables with SQL and observe their transactionally complete changes through standard PostgreSQL triggers or logical decoding.

## What makes the design different

`pg_mdm` treats candidate discovery as part of correctness, not merely a performance detail. A configured matching rule must examine every pair inside its declared candidate predicates. If an oversized block, missing dependency, unsupported extension version, or resource limit prevents complete evaluation, refresh fails and the previous publication remains readable. The system never turns skipped work into evidence that two records do not match.

Clustering is conservative for the same reason. One matching email or phone number may join two individual records, but a chain of shared values cannot keep growing an established entity without stronger independent support. Explicit steward decisions remain durable, cannot-link decisions are enforced across whole components, and automatic authoritative conflicts go to review. Stable `mdm_id` values follow documented merge and split rules, while every golden value retains the source, policy, tie break, definition version, and publication revision that selected it.

The design also separates semantic choices from physical execution. Cleaners, candidate predicates, pair decisions, clustering rules, stable-ID reconciliation, and golden-value policies are pinned by version. Indexes, query plans, and maintenance strategies may change without redefining identity, provided they produce the same logical evidence and result. This makes rebuilds, upgrades, explanations, and regression tests part of the product contract rather than afterthoughts.

## Project status

The proposed V1 release resolves records already stored in supported local or partitioned PostgreSQL tables. It covers tracked and soft-delete sources, deterministic built-in matching, bounded candidate generation, full-entity resolution, stable IDs, field-level golden records, pair-level stewardship, review, explanation, and atomic publication. V1 intentionally leaves complete snapshots, custom matching code, approximate retrieval, valid-time history, direct merge and split workflows, multi-entity dependencies, resumable runs, namespaces, quotas, and other enterprise controls outside its first compatibility promise.

The post-V1 capability catalogue is cumulative rather than a replacement for V1. It lists candidate work selected only when a deployment demonstrates the need, while preserving the same five nouns, five actions, and three primary outputs. Each optional feature must declare its dependencies, deterministic semantics, migration path, failure boundary, and retention needs; unsupported combinations fail closed instead of silently producing a weaker answer.

Read the design documents for the normative details:

- [DESIGN_V1.md](DESIGN_V1.md) defines the proposed first open-source release, including its SQL model, algorithms, invariants, security boundary, and acceptance criteria.
- [DESIGN_V2.md](DESIGN_V2.md) is the post-V1 capability catalogue and dependency order. It is not an approved delivery backlog.
- [ROADMAP.md](ROADMAP.md) divides the V1 implementation into small development releases and defines the V1.0 release gate.

## Contributing

The most useful contributions at this stage are concrete design reviews. A good review identifies a source-system behavior, matching failure, stewardship workflow, PostgreSQL constraint, or operational recovery case that the current contracts do not handle. Proposed changes should preserve complete candidate evaluation, deterministic results, atomic publication, durable identity, and bounded explanation, or state plainly why one of those guarantees needs to change.

Implementation work should follow the V1 release boundary rather than pulling roadmap features into the first compatibility promise. In particular, the public SQL API requires `external_graph_refresh` major 1 through `pgtrickle.integration_capabilities()` and a shared conformance suite for graph contracts, transactional refresh, rollback, concurrency, source boundaries, durable `EXTERNAL` orchestration, clone isolation, and recovery.

## License

`pg_mdm` is licensed under the [Apache License 2.0](LICENSE).
