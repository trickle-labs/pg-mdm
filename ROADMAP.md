# `pg_mdm` implementation roadmap

## Status after v0.11

Reviewed 14 September 2026. pg-mdm v0.1 through v0.12 are released. MDM-STEWARDSHIP/1 and both owner approvals are frozen in the [joint sign-off record](contracts/MDM-STEWARDSHIP-1-review.md). The current main branch is updating its pg-trickle pin from v0.105.2 to v0.105.3 and qualifying the exact package; keep that candidate unadmitted until the cumulative checks pass. Remaining V1 evidence gaps and the v0.13 M1 implementation are tracked below and in [DESIGN_V2.md](DESIGN_V2.md).

The [V1 design](DESIGN_V1.md) controls existing semantics. The [V2 design](DESIGN_V2.md#23-recommended-delivery-scope-and-dependencies) defines the proposed additions and their prerequisites. This roadmap controls sequencing. V2 is a design generation, not a package-version commitment. The v1.0 compatibility gate remains separate from delivery of optional V2 capabilities.

The release numbers below are recommendations, not scheduled commitments. Write a reviewed implementation plan with executable exit cases before each release starts. Keep one implementation release in progress at a time. Estimate it from measured work and a fixed scope; the original V1 person-week estimates are no longer a useful forecast. Record required evidence before dependent work begins. Changes to V1 semantics require an explicit migration or correctness-repair plan.

## Released baseline

| Releases | Delivered foundation | Source of scope and evidence requirements |
|---|---|---|
| v0.1–v0.2 | Extension, roles, immutable definitions, validation, and artifacts | [v0.1 plan](plans/v0.1.md), [v0.2 plan](plans/v0.2.md) |
| v0.3–v0.4 | Normalization, source identity, and bounded candidate generation | [v0.3 plan](plans/v0.3.md), [v0.4 plan](plans/v0.4.md) |
| v0.5–v0.6 | Pair decisions, durable manual constraints, and conservative full resolution | [v0.5 plan](plans/v0.5.md), [v0.6 plan](plans/v0.6.md) |
| v0.7 | Stable IDs, goldens, provenance, reviews, and publication model | [v0.7 plan](plans/v0.7.md) |
| v0.8–v0.9 | Private Graph V1 installation, strict refresh, and atomic publication | [v0.8 plan](plans/v0.8.md); v0.9 has no separate `plans/v0.9.md`, see the [changelog entry](CHANGELOG.md#090), `src/api/refresh.rs` |
| v0.10–v0.11 | Operational checks, package upgrades, AUTO/FULL probes, and publication rollback/retry qualification | [v0.10 plan](plans/v0.10.md), [v0.11 plan](plans/v0.11.md), `tests/e2e.sql`, `scripts/run_e2e_tests.sh` |

pg-mdm v0.11.0 is tagged at `97b78c8` with pg-trickle v0.105.1. The v0.12.0 sign-off admitted v0.105.2; main now updates the lock and tests to v0.105.3. Versions v0.1 through v0.7 used v0.98.0. Released code and test coverage do not establish that every original V1 acceptance criterion has passed.

Known baseline limitations must remain visible:

- Candidate blocks and candidate-pair joins use explicit `FULL` refresh in `src/graph_spec.rs` because v0.105.1 could drop inserts for those shapes. The v0.105.3 release includes a stream-table repair path; keep `FULL` until the exact MDM insert/update/delete/rollback/retry histories pass on that package.
- `preview_entity()` returns metadata and counts for validation, sampled, and scoped modes. Its scoped `exact` flag currently lacks subproblem resolution behind it. Repair that claim and complete the V1 preview contract before using preview to authorize V2 actions.
- The committed organization corpus is a four-record seed. The repository does not yet provide the held-out quality report and combined operating-envelope evidence required below.
- MDM uses trigger capture and full terminal scans followed by full-entity resolution. Delta consumption, affected-set resolution, prepared runs, and V2 semantic events remain unimplemented.

## Current pg-trickle v0.105.3 qualification

The [pg-trickle v0.105.3 release](https://github.com/trickle-labs/pg-trickle/releases/tag/v0.105.3) is tagged at commit `7b7ecf16e320c6a38b3433f8470c6cbe2b305b8d`. Its PostgreSQL 18 Linux artifact is `pg_trickle-0.105.3-pg18-linux-amd64.tar.gz`, SHA-256 `c59da1ea543ea23c207f12294f40fbff41d58b5554ced81408e5f3efde7d84c6`. The package lock and E2E upgrade target are updated; exact-package admission still requires the cumulative run and populated 0.105.1-to-0.105.3 state comparison.

## v0.105.2 admission (v0.12 historical record)

The [pg-trickle v0.105.2 release](https://github.com/trickle-labs/pg-trickle/releases/tag/v0.105.2) is available at commit `33df4cc91fda4fbadba79470347a714c8509703a`. The release publishes `pg_trickle-0.105.2-pg18-linux-amd64.tar.gz` with SHA-256 `bf8d8dcff728a5cf9e09458b70f2c3ce2c92cc109f4e7c79ae2933ac8bb03416`. Treat it as the next admission candidate; this document does not change the build lock.

The [tagged capability manifest](https://github.com/trickle-labs/pg-trickle/blob/v0.105.2/docs/capability-manifest.json) advertises stable, enabled Graph V1, Delta V1, trigger capture, and WAL capture. It does not advertise `prepared_graph_generation` or `prepared_output_delta_binding`. Those remain in the [upstream post-1.0 proposal](https://github.com/trickle-labs/pg-trickle/blob/v0.105.2/plans/PROPOSAL_V2_PREPARED_GRAPH_GENERATIONS.md). Package/runtime qualification and benchmark smoke tests shipped; the 72-hour soak and seven-day longevity runs remain deferred upstream.

| Risk | Owner | Next required evidence |
|---|---|---|
| `RISK-PGT-GRAPH-V1` | pg-mdm release owner | Re-run public capability, canonical contract, durable `EXTERNAL`, delegated authorization/revocation, RLS, complete boundary, rollback/frontier, concurrency, lifecycle, clone, restore, upgrade, and private-API denial tests on the exact v0.105.3 package |
| Candidate insert loss | Graph compiler maintainer | Reproduce bytea block-membership and multi-row pair-insert cases against complete SQL expectations and FULL reference. Preserve `FULL` until each affected AUTO shape passes |
| Prepared execution unavailable | pg-mdm integration owner with upstream maintainer | Agree public SQL signatures, capability versions, leases, crash/restore behavior, and shared conformance; wait for a released enabled capability before MDM integration |
| Incomplete qualification evidence | pg-mdm release owner | Map the V1 criteria to actual commands and retained results, record gaps, and complete the quality and operating evidence below |

Keep the current PostgreSQL 18 and trigger-capture scope for the next release. Upstream WAL availability permits a later MDM admission effort; it does not change MDM's supported capture mode. Review these risks on each dependency or compiler revision. Version checks alone do not admit a contract.

## pg-react stewardship integration

The [MDM integration plan](../pg-react/plans/PLAN_PG_MDM_STEWARDSHIP_INTEGRATION.md) and [React integration plan](../pg-react/plans/PLAN_PG_REACT_MDM_STEWARDSHIP.md) provide the concrete use case for queue assignment, due dates, escalation, and audit receipts. These workspace links point to the maintained companion plans. The older React planning document redirects to the second plan. Their statement that MDM has no assigned post-v0.11 releases is superseded by the proposed mapping below; reconcile that mapping during M0 before setting joint dates.

Keep `MDM-STEWARDSHIP/1` owned by MDM. Section 4 of the MDM companion is the proposed normative interface; M0 must freeze its exact types, signatures, errors, roles, and fixtures. The named policy tables and intent API are not installed v0.11 APIs. React owns policy evaluation, managed deadlines, durable work, and the adapter. MDM owns review state, permitted controls, human approvals, and identity publication. Neither core extension gains a mandatory dependency on the other.

| MDM milestone and proposed release | Owner | React milestone unlocked | Required handoff |
|---|---|---|---|
| M0, v0.12 | MDM contract owner with React adapter owner | R0 compatibility work, React 0.46.0 | Freeze the shared contract and qualify the common packaged stack. R0 completion still requires M1 |
| M1, v0.13 | MDM implementation owner | R0 completion and R1 read-only qualification, React 0.46.0–0.47.0 | Real logged policy projection, persistent occurrence keys, protected metadata, and publication rollback evidence |
| M2, v0.14 | MDM implementation owner with React adapter owner | R2 durable intents and R3 deadlines, React 0.48.0–0.49.0 | Typed controls, automation bindings, manual protection, deduplication, receipts, and stale-work outcomes |
| M4, v0.15 | Joint release owners | R5 rollout, React 0.50.0 | Exact end-to-end results, time-only escalation, worker authorization, upgrade, restore, clone isolation, and supported limits |
| M3, separate optional milestone | MDM approval owner | R4 approval policy, React 0.51.0 | Exact-action proposal ledger, authenticated human approvals, versioned requirements, and final validation before R4 starts |

Deliver M0 → M1 → M2 → M4 for the initial integration. R5 intentionally precedes optional R4. Estimate the MDM packages independently; React's release budgets do not include them. M1/M2 implementation needs the admitted stack and frozen contract, but has no dependency on full exact preview, direct merge/split commands, semantic events, Delta V1, prepared execution, Tide, or a reviewer UI.

The initial joint profile is one administrative scope in one PostgreSQL 18 database, trigger CDC, scheduler off, and `READ COMMITTED`. Keep MDM's EXTERNAL graphs independently owned and retain React's explicit coordinator and differential-refresh safeguard. React must update its 0.98.0 admission assumptions and packaging to the same qualified artifact before the shared runtime is called jointly qualified; verify effective PostgreSQL runtime identity rather than assuming the two image manifests agree.

React currently rejects RLS-protected evaluated sources. M1 therefore supplies an authorized non-RLS policy table containing only approved metadata. Keep sensitive evidence under MDM permissions. Reject unsupported scopes; never hide an RLS source behind a view or grant `BYPASSRLS` to make it eligible. Human stewardship remains usable with the adapter absent or paused.

## Recommended post-v0.11 releases

### v0.12: Admit v0.105.2 and reconcile V1 acceptance

1. Download and checksum the published artifact. Update `DEPENDENCIES.md`, the E2E image, `src/version.rs`, and installation documentation together in the implementation change.
2. Run the cumulative Graph V1 admission and package checks against that artifact. Test pg-trickle 0.105.1-to-0.105.2 upgrade with existing MDM entities and publications, and the pg-mdm 0.11-to-next-release upgrade. Preserve IDs, definitions, grants, and consumer objects.
3. Add the two candidate-insert regression cases and inspect actual `node_results`. Keep exact FULL fallbacks if the upstream defects persist. A strategy change produces a new immutable compiler artifact and a tested adoption/rebuild path.
4. Audit every V1 acceptance criterion. Replace the scoped-preview exactness claim with a truthful result until the complete induced subproblem is evaluated, then finish the V1 sampled/scoped behavior through the production path. Record any other missing behavior as blocking work with an owner.
5. Turn the organization fixture into executable quality checks with approved thresholds and held-out cases. Measure graph refresh, full MDM resolution, and publication separately. Record the supported envelope and remaining acceptance gaps.
6. Complete M0 with React R0. Freeze `MDM-STEWARDSHIP/1`, including the mapping from review concurrency version, definition versions, decision epoch, publication revision, evidence digest, and action-driving revision to intent tokens. Define an auditable opening-time backfill and the shared replay horizon. Agree owners and exact shared fixtures before M1/M2 implementation.

Exit when the admitted pins, upgrade archive, public API behavior, regression cases, and cumulative CI pass with linked results. Every unresolved V1 requirement must remain an explicit v1.0 blocker; resolve prerequisites before dependent V2 work. This release establishes a measured baseline and makes no prepared-execution claim.

### v0.13: Policy review projection, M1

Implement the proposed logged `mdm_steward.policy_cases_v1` table and persistent occurrence mapping. Allocate one unique, non-null `case_key bigint` per review occurrence; retain its `review_id uuid`, issue key, occurrence, and closed mapping. Never hash the UUID into a bigint or reuse a key. Publish status, reason, approved metadata, permitted actions, queue/deadline/escalation fields, and the frozen concurrency and basis tokens. MDM alone writes this table.

Record immutable `opened_at` when an occurrence first publishes. Backfill existing occurrences from retained publication evidence. If that evidence is unavailable, expose an unknown opening time and withhold automatic deadline assignment until an authorized backfill. Installation and retry times are not opening times. Recurrence gets a new key and opening time.

Update evidence fields atomically with MDM publication; later M2 controls update administrative fields in their own transaction. Derive `pending_stewardship` from the latest successful publication observation, including no-change observations. Keep action-driving revisions separate from unrelated publication and audit changes so they cannot generate repeat policy work.

Exit when React can read the exact authorized rows from real MDM with no effects. Test failed-publication rollback, recurrence, no-change observations, opening-time backfill, masking, grants, upgrade, and restore. Include committed policy-table visibility, shared-source CDC, skipped ordinary refreshes, and FULL fallback in joint R0/R1 checks. A fixture-only projection does not close M1.

### v0.14: Policy intents, bindings, and receipts, M2

Implement the proposed `mdm_steward.submit_policy_intent(...)` and `mdm_steward.policy_receipts_v1` contract. Allow only `ASSIGN_QUEUE`, `SET_DUE_AT`, and `ESCALATE` initially. Add binding administration, pause/replacement controls, allowed queues, deadline and escalation limits, and manual assignment protection. Bind automation to its currently authorized policy digest and a distinct role that cannot invoke human decision or golden-override APIs.

Authenticate before looking up a prior request. An identical authorized retry of `(binding_id, request_key)` returns the existing receipt; changed arguments return `IDEMPOTENCY_CONFLICT`. For a new request, lock the case and binding, validate every expected token and permission, then commit the control update and receipt together. Preserve request keys and mappings through the shared replay horizon; out-of-horizon requests fail closed. Stale, denied, paused, and replaced-binding results must have explicit outcomes.

React persists immutable request bodies and keys, calls the local intent API, records the receipt reference, and completes its database work in the same transaction. It never calls `mdm.refresh()` from a consequence. `APPLIED_CONTROL` confirms a control update, not identity resolution. Reserve `ACCEPTED_PENDING_PUBLICATION` and `APPLIED_PUBLICATION` for separately enabled identity-decision support. React must reevaluate stale work instead of retrying stale payloads indefinitely.

Exit evidence covers concurrent human edits, manual locks, policy replacement, pause, privilege revocation, duplicate and conflicting requests, uncertain commits, and complete transaction rollback through the actual React worker identity, including role changes and privileged helpers. Verify exact requests, receipts, control rows, and work outcomes. Audit-only writes and unrelated publication advances must not generate new intents. M2 can proceed while React implements R1 after M0/M1.

### v0.15: Joint stewardship qualification, M4

Run M4 with React R5 after M1/M2 and React R1–R3 pass. Build `showcase/mdm-stewardship/` in pg-react with shared fixtures used by both release suites. Publish an ambiguous case, assign its queue and due date, advance managed time without source edits, escalate once per permitted level, accept an authenticated human decision, and independently refresh MDM to close the review. Prove the original deadline survives retries unless an authorized policy replacement changes it; a materialized `now()` condition alone is insufficient.

Assert exact policy rows, canonical request bytes, receipts, work outcomes, and final review state. Include ambiguous and missing routing candidates, changed evidence, recurring cases, stale work, missing capabilities, unauthorized access, output failure after a human decision, restart, upgrade, restore, and clone isolation. Restore, clone, and policy replacement disable new automated writes until bindings, request deduplication, and case bases are reconciled. Pause never reverses accepted human directives or published identity.

Start with side-effect-free comparison over a fixed population, respecting React's bounded comparison limits. Enable only routing, deadlines, and escalation for a named cohort after joint evidence passes. Record exact artifacts, measured limits, replay/receipt retention, recovery steps, and queries for stale/denied work, overdue age, backlog, repeated escalations, and accepted-but-unpublished human decisions. Neither NOTIFY delivery nor enqueue success proves a completed control action or MDM publication.

### v0.16: Exact entity preview and regression fixtures

Depends on the v0.12 integration and preview prerequisites. This release follows the recommended routing rollout and implements design sections 8 and 17. Its technical prerequisites do not include pg-react.

Reuse the existing compiler, terminal readers, full resolver, identity reconciler, and golden selector in a shared non-publishing evaluation path. Add a separate private graph for a proposed definition, `exact_entity` preview within the measured transaction envelope, and labeled pair, partition, forbidden-co-membership, and golden fixtures. Extend existing metadata only with the manifest fields needed to bind the result: definition/artifact, source boundary, execution role, base publication, decision epoch, and semantic versions.

Report membership, merge/split, survivor, golden, review, and schema effects. Use preview-local handles for new IDs. Do not allocate durable IDs or write public outputs during preview. Require a fresh preview or complete recomputation after any pinned input changes. Keep bounded explanation and cleanup rules in this release.

Exit evidence must compare preview with subsequent refresh from the same manifest, including rejected definitions, source/RLS changes, stewardship changes, merges, splits, resource failure, and schema changes. Assert that preview leaves the ID ledger, active definition, directives, and current publication unchanged. An entity too large for exact preview receives an explicit resource failure, not an exact label on a sample. No prepared capability is required.

### v0.17: Explicit merge and split stewardship

Depends on exact impact preview. Implement design section 12's merge and complete split partitions over durable source-record subjects. Reuse the immutable directive ledger, contradiction checks, and V1 identity policy. Bind writes to the preview digest, expected base revision, and decision epoch. Publish their effects through normal refresh.

Exit evidence covers complete constraint compilation, contradictory and dormant directives, stale previews, concurrent writes, explicit supersession, retry, reactivation, and restore. A partial split specification or incomplete affected closure must be rejected atomically. Verify memberships, ID continuity, provenance, and bounded reasons after refresh.

Queue assignment and manual assignment protection belong to M1/M2. Keep move-member, identity and membership locks, alternate continuity, and bulk operations in the optional backlog. Approval requirements have the separate M3 gate below. The routing adapter gains no authority to issue merge/split or human pair decisions.

### v0.18: Semantic change feed

Depends on the publication comparison path and the supported merge/split semantics. Implement the optional change table from design section 15 with deterministic `(publication_revision, event_no)` order, stable event IDs, bounded protected payloads, and retained details for large member sets.

Ship cursor replay, idempotent consumption, retention promises, gap detection, and snapshot resynchronization together. Observation-only refreshes emit no events. Semantic-only changes advance the revision when the enabled feed requires an event. Implement event types only for available features.

Exit evidence covers merge, split, retirement, reactivation, member movement, golden/provenance and review changes, no-op refresh, retry, output-write rollback, authorization, restore, and retention gaps. Compare events with the semantic before-and-after publication and prove that events and current outputs commit together. Use ordinary SQL consumers; external delivery is optional later work.

### M3: Optional human approval requirements before React 0.51.0

Schedule and estimate this milestone independently after the initial rollout. Implement an exact-action proposal ledger, a separate authenticated human approval API, requirement versions, and final validation. M3 needs action-specific subject/evidence preview; definition preview alone does not satisfy that requirement. It need not wait for semantic events or prepared execution.

Capture human identity from authenticated database access or a separately trusted identity gateway. Two required approvals mean two distinct authorized humans. Reject self-approval, worker-supplied votes, stale evidence or membership, and requirement downgrades below administrator minima. Revalidate at acceptance and check directive coherence during publication. Proposal rejection does not create `NOT_MATCH`. Preserve approved actions and their evidence through the audit horizon.

React R4 may propose bounded approval requirements and display MDM outcomes only after M3 passes real integration tests and advertises support. Keep it separately disabled by default. Automatic MATCH/NOT_MATCH, a probability shortcut, and direct identity mutation by the worker remain outside this contract.

## Optional tracks after the first V2 scope

The dependency order and smallest implementation for every remaining catalogue area are recorded in [design section 23.5](DESIGN_V2.md#235-optional-capability-tracks). Do not assign release numbers before selecting a deployment need and completing its prerequisites.

| Track | Entry condition | Required order |
|---|---|---|
| Full source contracts and valid time | A named source cannot use tracked/soft-delete semantics | Local snapshot or ordered events, completeness/replay, retained versions, corrections, then historical projections |
| Configuration reuse and inference | Repeated definitions or a measured onboarding problem | Flat versioned fragments and provenance, fixture verification, then inferred proposals; deeper nesting only if needed |
| Matching, clustering, continuity, and goldens | Labeled failures of the existing policies | One versioned built-in improvement with exact preview; custom code only after dependency inspection and invalidation are proven |
| Extended stewardship | Pair decisions and merge/split cannot express a named workflow | Move-member, identity/membership locks, and bounded bulk action after exact impact and precedence tests. Routing/manual assignment protection is M1/M2; approvals are M3 |
| Resolved entities as sources | A consumer needs revision-bound MDM dependencies | Complete feed replay/resynchronization first, then cycle rejection, revision pinning, and merge/split propagation |
| Prepared full resolution | Measurements show MDM resolution exceeds the transaction envelope and upstream ships the capability | Public capability admission, one run/worker, logged checkpoints, open-before-read, compare-and-swap promotion, abandonment, crash/restore verification, then multiple workers if justified |
| Affected-set resolution | Full-resolution cost dominates and closure can be proved | Synchronous Delta V1 read/ack and FULL_INVALIDATION fallback, generated equivalence, then optional prepared delta binding after upstream delivery |
| Diagnostics, verification, shared operations, and retention | An enabled feature or deployment needs the contract | Per-feature permissions, retention, and recovery first; richer diagnostics, namespaces, quotas, fairness, service objectives, holds, and compaction as required |

Prepared full resolution needs `prepared_graph_generation`, not Delta V1. Affected-set resolution can use Delta V1 synchronously without prepared generations. Combining them adds the separate `prepared_output_delta_binding` prerequisite. Prepared major 1 still refreshes relational evidence in one transaction and freezes storage in place; it cannot solve an oversized graph-refresh transaction.

The initial routing integration is complete when M0/M1/M2/M4 and React R0/R1/R2/R3/R5 pass together on the admitted stack. The broader first V2 scope also requires exact preview, merge/split stewardship, and semantic events in v0.16–v0.18 while preserving V1 behavior. M3 remains an independent optional approval gate. Declare deferred capabilities explicitly. Completing the entire catalogue requires plans and acceptance evidence for every optional track, including their interactions; the first V2 scope does not make that claim.

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
