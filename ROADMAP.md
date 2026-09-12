# `pg_mdm` implementation roadmap

## Status after v0.11

Reviewed 12 September 2026. pg-mdm v0.1 through v0.11 are released. The next work is to admit pg-trickle v0.105.2, close the remaining V1 implementation and evidence gaps, and deliver a focused set of capabilities from [DESIGN_V2.md](DESIGN_V2.md).

The [V1 design](DESIGN_V1.md) controls existing semantics. The [V2 design](DESIGN_V2.md#23-recommended-delivery-scope-and-dependencies) defines the proposed additions and their prerequisites. This roadmap controls sequencing. V2 is a design generation, not a package-version commitment. The v1.0 compatibility gate remains separate from delivery of optional V2 capabilities.

The release numbers below are recommendations, not scheduled commitments. Write a reviewed implementation plan with executable exit cases before each release starts. Keep one implementation release in progress at a time. Estimate it from measured work and a fixed scope; the original V1 person-week estimates are no longer a useful forecast. Record required evidence before dependent work begins. Changes to V1 semantics require an explicit migration or correctness-repair plan.

## Released baseline

| Releases | Delivered foundation | Source of scope and evidence requirements |
|---|---|---|
| v0.1–v0.2 | Extension, roles, immutable definitions, validation, and artifacts | [v0.1 plan](plans/v0.1.md), [v0.2 plan](plans/v0.2.md) |
| v0.3–v0.4 | Normalization, source identity, and bounded candidate generation | [v0.3 plan](plans/v0.3.md), [v0.4 plan](plans/v0.4.md) |
| v0.5–v0.6 | Pair decisions, durable manual constraints, and conservative full resolution | [v0.5 plan](plans/v0.5.md), [v0.6 plan](plans/v0.6.md) |
| v0.7 | Stable IDs, goldens, provenance, reviews, and publication model | [v0.7 plan](plans/v0.7.md) |
| v0.8–v0.9 | Private Graph V1 installation, strict refresh, and atomic publication | [v0.8 plan](plans/v0.8.md), [changelog](CHANGELOG.md), `src/api/refresh.rs` |
| v0.10–v0.11 | Operational checks, package upgrades, AUTO/FULL probes, and publication rollback/retry qualification | [v0.10 plan](plans/v0.10.md), [v0.11 plan](plans/v0.11.md), `tests/e2e.sql`, `scripts/run_e2e_tests.sh` |

pg-mdm v0.11.0 is tagged at `97b78c8`. Its current dependency remains pg-trickle v0.105.1, as recorded in [DEPENDENCIES.md](DEPENDENCIES.md), `tests/Dockerfile.e2e`, and `src/version.rs`. Versions v0.1 through v0.7 used v0.98.0. Released code and test coverage do not establish that every original V1 acceptance criterion has passed.

Known baseline limitations must remain visible:

- Candidate blocks and candidate-pair joins use explicit `FULL` refresh in `src/graph_spec.rs` because v0.105.1 can drop inserts for those shapes. The supported AUTO scan probe is not proof that all compiled MDM stages are differential.
- `preview_entity()` returns metadata and counts for validation, sampled, and scoped modes. Its scoped `exact` flag currently lacks subproblem resolution behind it. Repair that claim and complete the V1 preview contract before using preview to authorize V2 actions.
- The committed organization corpus is a four-record seed. The repository does not yet provide the held-out quality report and combined operating-envelope evidence required below.
- MDM uses trigger capture and full terminal scans followed by full-entity resolution. Delta consumption, affected-set resolution, prepared runs, and V2 semantic events remain unimplemented.

## Upstream v0.105.2 admission

The [pg-trickle v0.105.2 release](https://github.com/trickle-labs/pg-trickle/releases/tag/v0.105.2) is available at commit `33df4cc91fda4fbadba79470347a714c8509703a`. The release publishes `pg_trickle-0.105.2-pg18-linux-amd64.tar.gz` with SHA-256 `bf8d8dcff728a5cf9e09458b70f2c3ce2c92cc109f4e7c79ae2933ac8bb03416`. Treat it as the next admission candidate; this document does not change the build lock.

The [tagged capability manifest](https://github.com/trickle-labs/pg-trickle/blob/v0.105.2/docs/capability-manifest.json) advertises stable, enabled Graph V1, Delta V1, trigger capture, and WAL capture. It does not advertise `prepared_graph_generation` or `prepared_output_delta_binding`. Those remain in the [upstream post-1.0 proposal](https://github.com/trickle-labs/pg-trickle/blob/v0.105.2/plans/PROPOSAL_V2_PREPARED_GRAPH_GENERATIONS.md). Package/runtime qualification and benchmark smoke tests shipped; the 72-hour soak and seven-day longevity runs remain deferred upstream.

| Risk | Owner | Next required evidence |
|---|---|---|
| `RISK-PGT-GRAPH-V1` | pg-mdm release owner | Re-run public capability, canonical contract, durable `EXTERNAL`, delegated authorization/revocation, RLS, complete boundary, rollback/frontier, concurrency, lifecycle, clone, restore, upgrade, and private-API denial tests on the exact v0.105.2 package |
| Candidate insert loss | Graph compiler maintainer | Reproduce bytea block-membership and multi-row pair-insert cases against complete SQL expectations and FULL reference. Preserve `FULL` until each affected AUTO shape passes |
| Prepared execution unavailable | pg-mdm integration owner with upstream maintainer | Agree public SQL signatures, capability versions, leases, crash/restore behavior, and shared conformance; wait for a released enabled capability before MDM integration |
| Incomplete qualification evidence | pg-mdm release owner | Map the V1 criteria to actual commands and retained results, record gaps, and complete the quality and operating evidence below |

Keep the current PostgreSQL 18 and trigger-capture scope for the next release. Upstream WAL availability permits a later MDM admission effort; it does not change MDM's supported capture mode. Review these risks on each dependency or compiler revision. Version checks alone do not admit a contract.

## Recommended post-v0.11 releases

### v0.12: Admit v0.105.2 and reconcile V1 acceptance

1. Download and checksum the published artifact. Update `DEPENDENCIES.md`, the E2E image, `src/version.rs`, and installation documentation together in the implementation change.
2. Run the cumulative Graph V1 admission and package checks against that artifact. Test pg-trickle 0.105.1-to-0.105.2 upgrade with existing MDM entities and publications, and the pg-mdm 0.11-to-next-release upgrade. Preserve IDs, definitions, grants, and consumer objects.
3. Add the two candidate-insert regression cases and inspect actual `node_results`. Keep exact FULL fallbacks if the upstream defects persist. A strategy change produces a new immutable compiler artifact and a tested adoption/rebuild path.
4. Audit every V1 acceptance criterion. Replace the scoped-preview exactness claim with a truthful result until the complete induced subproblem is evaluated, then finish the V1 sampled/scoped behavior through the production path. Record any other missing behavior as blocking work with an owner.
5. Turn the organization fixture into executable quality checks with approved thresholds and held-out cases. Measure graph refresh, full MDM resolution, and publication separately. Record the supported envelope and remaining acceptance gaps.

Exit when the admitted pins, upgrade archive, public API behavior, regression cases, and cumulative CI pass with linked results. Every unresolved V1 requirement must remain an explicit v1.0 blocker; resolve prerequisites before dependent V2 work. This release establishes a measured baseline and makes no prepared-execution claim.

### v0.13: Exact entity preview and regression fixtures

Depends on the v0.12 integration and preview prerequisites. This is the first recommended V2 capability release, corresponding to design sections 8 and 17.

Reuse the existing compiler, terminal readers, full resolver, identity reconciler, and golden selector in a shared non-publishing evaluation path. Add a separate private graph for a proposed definition, `exact_entity` preview within the measured transaction envelope, and labeled pair, partition, forbidden-co-membership, and golden fixtures. Extend existing metadata only with the manifest fields needed to bind the result: definition/artifact, source boundary, execution role, base publication, decision epoch, and semantic versions.

Report membership, merge/split, survivor, golden, review, and schema effects. Use preview-local handles for new IDs. Do not allocate durable IDs or write public outputs during preview. Require a fresh preview or complete recomputation after any pinned input changes. Keep bounded explanation and cleanup rules in this release.

Exit evidence must compare preview with subsequent refresh from the same manifest, including rejected definitions, source/RLS changes, stewardship changes, merges, splits, resource failure, and schema changes. Assert that preview leaves the ID ledger, active definition, directives, and current publication unchanged. An entity too large for exact preview receives an explicit resource failure, not an exact label on a sample. No prepared capability is required.

### v0.14: Explicit merge and split stewardship

Depends on exact impact preview. Implement design section 12's merge and complete split partitions over durable source-record subjects. Reuse the immutable directive ledger, contradiction checks, and V1 identity policy. Bind writes to the preview digest, expected base revision, and decision epoch. Publish their effects through normal refresh.

Exit evidence covers complete constraint compilation, contradictory and dormant directives, stale previews, concurrent writes, explicit supersession, retry, reactivation, and restore. A partial split specification or incomplete affected closure must be rejected atomically. Verify memberships, ID continuity, provenance, and bounded reasons after refresh.

Keep move-member, locks, alternate continuity, approvals, assignment, and bulk operations in the optional backlog until a named workflow requires them. Their narrower scope never weakens existing pair decisions or source-record uniqueness.

### v0.15: Semantic change feed

Depends on the publication comparison path and the supported merge/split semantics. Implement the optional change table from design section 15 with deterministic `(publication_revision, event_no)` order, stable event IDs, bounded protected payloads, and retained details for large member sets.

Ship cursor replay, idempotent consumption, retention promises, gap detection, and snapshot resynchronization together. Observation-only refreshes emit no events. Semantic-only changes advance the revision when the enabled feed requires an event. Implement event types only for available features.

Exit evidence covers merge, split, retirement, reactivation, member movement, golden/provenance and review changes, no-op refresh, retry, output-write rollback, authorization, restore, and retention gaps. Compare events with the semantic before-and-after publication and prove that events and current outputs commit together. Use ordinary SQL consumers; external delivery is optional later work.

## Optional tracks after the first V2 scope

The dependency order and smallest implementation for every remaining catalogue area are recorded in [design section 23.5](DESIGN_V2.md#235-optional-capability-tracks). Do not assign release numbers before selecting a deployment need and completing its prerequisites.

| Track | Entry condition | Required order |
|---|---|---|
| Full source contracts and valid time | A named source cannot use tracked/soft-delete semantics | Local snapshot or ordered events, completeness/replay, retained versions, corrections, then historical projections |
| Configuration reuse and inference | Repeated definitions or a measured onboarding problem | Flat versioned fragments and provenance, fixture verification, then inferred proposals; deeper nesting only if needed |
| Matching, clustering, continuity, and goldens | Labeled failures of the existing policies | One versioned built-in improvement with exact preview; custom code only after dependency inspection and invalidation are proven |
| Extended stewardship | Pair decisions and merge/split cannot express a named workflow | Move-member and scoped locks, precedence and exact impact tests, then approvals, assignment, or bounded bulk action |
| Resolved entities as sources | A consumer needs revision-bound MDM dependencies | Complete feed replay/resynchronization first, then cycle rejection, revision pinning, and merge/split propagation |
| Prepared full resolution | Measurements show MDM resolution exceeds the transaction envelope and upstream ships the capability | Public capability admission, one run/worker, logged checkpoints, open-before-read, compare-and-swap promotion, abandonment, crash/restore verification, then multiple workers if justified |
| Affected-set resolution | Full-resolution cost dominates and closure can be proved | Synchronous Delta V1 read/ack and FULL_INVALIDATION fallback, generated equivalence, then optional prepared delta binding after upstream delivery |
| Diagnostics, verification, shared operations, and retention | An enabled feature or deployment needs the contract | Per-feature permissions, retention, and recovery first; richer diagnostics, namespaces, quotas, fairness, service objectives, holds, and compaction as required |

Prepared full resolution needs `prepared_graph_generation`, not Delta V1. Affected-set resolution can use Delta V1 synchronously without prepared generations. Combining them adds the separate `prepared_output_delta_binding` prerequisite. Prepared major 1 still refreshes relational evidence in one transaction and freezes storage in place; it cannot solve an oversized graph-refresh transaction.

For the first V2 scope, completion means v0.13–v0.15 pass their gates on an admitted baseline and preserve existing V1 behavior. Declare deferred capabilities explicitly. Completing the entire catalogue requires plans and acceptance evidence for every optional track, including their interactions; the first V2 scope does not make that claim.

## Release evidence traceability

Record the exact test name, command, commit, artifact and fixture digests, result, and reviewed evidence link for each release row below. A blank or skipped blocking test does not satisfy its invariant. The release owner signs off the evidence before dependent work starts. The owning-release column records the original obligation; it is not a completion status.

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
| Authorization and RLS preservation | v0.1, v0.2, v0.7, v0.8 | Direct helper calls, role replacement, visibility invalidation, and explanation disclosure tests |
| Restore and upgrade continuity | Every release from v0.2 | Archived upgrade starts; clean restore of durable state, output schema, grants, and supported consumer objects |

## Quality and operating evidence

Expand `tests/fixtures/organization_domain_v1.json` into a runnable, de-identified organization-resolution corpus. Include labeled pairs and clusters for shared contact details, weak chains, authoritative conflicts, deletions, reactivations, and identifiers that need business context. Record candidate recall, false merges, missed matches, cluster errors, and review volume. The current four-record seed does not satisfy this requirement.

Use synthetic records when permission to commit source data is unavailable. Record label origin, permitted use, and unresolved ambiguities. Freeze metric definitions, denominators, and numeric acceptance thresholds with the release owner before tuning defaults. Separate tuning and held-out cases. Require zero false merges in explicit cannot-link, authoritative-conflict, and weak-chain safety fixtures. Report performance and accuracy separately; an algorithm oracle does not establish business correctness. Do not lower a threshold merely to pass a release.

Measure source rows, block memberships and skew, repeated discoveries, unique pairs, comparisons, component checks, memory, temporary storage, publication rows and bytes, retained history, and transaction duration. Record PostgreSQL, CPU, memory, storage, data distribution, fixture digests, compiler artifact, and upstream package with every result. Split timing among source/evidence work, full resolution, and publication so the decision between SQL optimization, affected-set work, and prepared execution has evidence.

The v0.11 AUTO/FULL probes and MDM histories remain regression evidence. Complete the broader generated differential-versus-full comparison over source and stewardship changes, merges, splits, golden-only changes, fallback, rebuild, rollback, and upgrade. Compare all public outputs and durable identity/provenance state, not only row counts. Retain independent fixture and small-graph oracles alongside production-path comparisons.

## v1.0 release gate

V1.0 freezes the public compatibility contract only after every V1 acceptance criterion has executable, reviewed evidence. The admitted pg-trickle artifact must advertise enabled Graph V1 major 1 and pass canonical contract, durable external orchestration, strict transactional refresh, source-boundary, rollback, concurrency, clone, recovery, and supported-upgrade checks. V1 remains trigger-only and uses complete terminal scans with full-entity MDM resolution.

Close the preview behavior gap, missing quality thresholds and held-out results, measured operating envelope, and any other gaps found by the v0.12 audit. Preserve graph-specific regressions and truthful strategy reporting for every compiler revision. Require differential or scoped maintenance for qualifying steady-state shapes once their incremental path is proven; exact FULL remains the fallback for unqualified shapes. Retain package manifests, fixture digests, commands, CI results, installation/recovery instructions, and upgrade/restore evidence together. An unresolved blocking test, a skipped positive capability case, or a release tag alone cannot satisfy this gate.

Optional V2 work may proceed once its prerequisites pass. It must neither delay a qualified V1 compatibility release merely to fill the catalogue nor waive an unmet V1 requirement.
