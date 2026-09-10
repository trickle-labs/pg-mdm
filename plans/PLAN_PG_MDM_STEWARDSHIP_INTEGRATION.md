# pg-mdm: Policy-Assisted Stewardship Implementation Plan

**Status:** Proposal; no implementation or API compatibility promise  
**Date:** 7 September 2026  
**Repository:** `grove/pg-mdm`  
**Companion:** [pg-react implementation plan](PLAN_PG_REACT_MDM_STEWARDSHIP.md)  
**Proposed shared contract:** `MDM-STEWARDSHIP/1`

## 1. Goal and ownership

Let pg-react recommend where an identity-review case should go, when it should be escalated, and—later—which approval policy applies. **pg-mdm remains the authority for review state, human approvals, permitted decisions, and identity changes.**

The existing repository is design-stage. V1 already specifies a review queue, durable pair `MATCH` / `NOT_MATCH` decisions, and anchored golden overrides. Assignment and approval workflows are V2 proposals. This plan implements an optional integration into that V2 direction; it does not make pg-react, advanced approvals, or automatic review resolution prerequisites for MDM V1. [1][2][3]

```text
MDM publishes a review case
    -> React evaluates a policy
    -> React submits a bounded intent through an MDM API
    -> MDM validates and records the permitted control change
    -> A later MDM refresh publishes any resulting identity change
```

There is no synchronous MDM -> React -> MDM refresh cycle. Human decisions remain possible without pg-react.

## 2. Entry gate

Follow the selected conservative start policy: substantive implementation waits for a **released, feature-frozen pg-trickle build at the planned 0.97 milestone or later**, with its published assurance evidence. The milestone is currently planned, not a claim of availability. [5]

The selected build must advertise stable `external_graph_refresh` major 1 and pass the MDM graph, rollback, recovery, and clone-isolation conformance suite. Delta V1 can be part of the preferred upstream baseline, but neither this integration nor MDM V1 requires delta consumption or an incremental MDM resolver. The current MDM roadmap explicitly retains full-entity resolution. [4]

Until that gate passes, limit work to contract review, specifications, and test-case design. Before joint implementation, qualify pg-react against the same upstream build; its currently documented qualified environment uses pg-trickle 0.81.0. [6]

## 3. First release scope

Ship **queue assignment, due dates, escalation state, and audit receipts**. Use one administrative scope in one PostgreSQL database. Keep sensitive identity evidence behind MDM permissions; policy inputs expose only approved metadata.

Defer multi-person approvals to the next optional milestone. Defer automatic `MATCH` / `NOT_MATCH`, direct merge/split commands, bulk actions, cross-database delivery, and a reviewer UI. Existing MDM matching rules continue to make their normal automatic decisions.

## 4. Shared interface: `MDM-STEWARDSHIP/1`

This section is the proposed normative interface for both plans. Names below are **new proposed names**, not existing SQL functions. Freeze exact PostgreSQL types and signatures in the contract milestone.

| Proposed surface | Contract |
| --- | --- |
| `mdm_steward.policy_cases_v1` | Permission-controlled, logged policy-input table. MDM alone maintains it. Expose a unique, non-null `case_key bigint`, the original `review_id`, entity, reason, open/resolved state, permitted actions, queue, due date, escalation level, and concurrency/basis tokens. |
| `mdm_steward.submit_policy_intent(...)` | Typed, allowlisted command API. Initial actions are `ASSIGN_QUEUE`, `SET_DUE_AT`, and `ESCALATE`. No arbitrary SQL, private-table writes, or identity mutations. |
| `mdm_steward.policy_receipts_v1` | Authorized results keyed by binding and request key; record action, outcome, reason, policy provenance, actor, and any resulting publication revision. |

**Identity and freshness.** Allocate `case_key` once per review occurrence, retain its mapping, and never reuse it. Do not hash an opaque review ID into a bigint. The MDM V1 design gives recurring issues new review occurrences; the integration must preserve that distinction. [2]

Each intent carries `binding_id`, `request_key`, `case_key`, typed action arguments, expected review version, definition version, publication revision, stewardship epoch, evidence-basis digest, and policy/evaluation/work references. The basis includes relevant subject membership and evidence. Relevant changes invalidate an unexecuted proposal. The first implementation may reject conservatively when the wider boundary changes.

**Authorization and concurrency.** Authenticate the caller, then check for an existing request. For a new request, lock and validate the case and automation binding in the same transaction as the control update. Check allowed actions, queue membership, deadline limits, manual assignment locks, and the binding's currently authorized policy digest. Conflicting requirements fail closed; an automated request cannot weaken an administrator's minimum review requirements.

**Idempotency.** Enforce uniqueness on `(binding_id, request_key)`. An identical authorized retry returns the existing receipt without repeating the action. Reuse with different arguments returns `IDEMPOTENCY_CONFLICT`. Preserve request identity across retries and ambiguous connection outcomes.

**Publication.** Evidence fields in the policy table change only with a successful MDM publication; administrative fields change with their control transaction. `APPLIED_CONTROL` means a routing or deadline update committed. Later identity-decision support must distinguish `ACCEPTED_PENDING_PUBLICATION` from `APPLIED_PUBLICATION`; only the latter records a successful resulting MDM publication. Resolving identity is never inferred from an intent receipt alone. This preserves the design's separation between accepted stewardship input and published semantic state. [2][3]

## 5. Implementation milestones

| Milestone | Deliverables | Exit evidence |
| --- | --- | --- |
| **M0 — Contract and scope** | Update the V2 stewardship section; document the shared interface, permission matrix, state transitions, error outcomes, and canonical fixtures. Pin one supported stack. | Both projects approve the same versioned contract; core V1 scope remains unchanged. |
| **M1 — Review projection** | Build the policy-input table and occurrence-key mapping. Separate published evidence from administrative control state. Add masking and publication-consistency tests. | React can read complete review cases without private access; rollback exposes no partial publication. |
| **M2 — Routing and escalation** | Implement bindings, the typed intent API, receipts, manual-assignment protection, bounded due-date changes, and deduplication. | Assignment and escalation work; duplicate, stale, unauthorized, and disabled-binding requests cannot alter state. |
| **M3 — Optional approval requirements** | Add an MDM-owned proposal and approval ledger. A proposal pins the exact action, subjects, evidence basis, and requirement version. Add a human approval API and final validation. | Two required approvals mean two distinct authorized humans; self-approval, stale evidence, and requirement downgrades fail. |
| **M4 — Joint qualification** | Run shared concurrency, restore, privilege, failure, upgrade, and end-to-end tests. Document pause, reconciliation, and retention. | The first-release scope passes on the packaged compatible stack before enablement. M3 is independently gated. |

## 6. Approval and automation safeguards

An approval is approval of a **specific proposed action**, not blanket approval of an entity ID that may later merge or split. Capture the human identity from authenticated database access or a separately trusted identity gateway—not from an arbitrary actor string supplied by pg-react. The React worker cannot supply human votes.

MDM checks approval requirements at acceptance and checks directive coherence again during publication. Changed evidence, membership, definitions, or requirements require a new review of the affected proposal. Rejecting a proposal is not the same as creating a durable `NOT_MATCH` constraint.

Automatic resolution requires a separate future capability and review. Do not implement the earlier illustrative “95% confidence” shortcut: V1 specifies evidence strengths and deterministic rules, not a calibrated probability contract. A machine policy must not acquire the manual `MATCH` operation's ability to override authoritative conflicts. Preserve cannot-link, lock, completeness, and component-integrity checks. [2]

## 7. Acceptance and recovery

| Scenario | Required result |
| --- | --- |
| Repeated request or retry after uncertain commit | One control change and one logical receipt. |
| Human changes the case while React work is queued | Stale work is rejected; manual intent is not overwritten. |
| Case closes, reopens, or changes definition | Old occurrence work cannot affect the new occurrence. |
| MDM refresh fails after a later identity decision is accepted | Previous publication stays intact; receipt remains pending, never falsely published. |
| Restore, clone, or policy replacement | New automated writes remain disabled until bindings, deduplication state, and case bases are reconciled. |
| Unauthorized or cross-scope access | No state mutation or disclosure of protected case details. |

Retain receipts, key mappings, and approval evidence for the documented replay/audit horizon. Out-of-horizon requests fail closed rather than being treated as new. A pause blocks new automated intents; accepted directives require explicit supersession to reverse them.

**First joint demonstration:** an ambiguous customer case is assigned, becomes overdue without further source edits, is escalated once, receives a human decision, and is closed only after a successful MDM publication.

## Sources reviewed

[1] [pg-mdm README: project status and responsibility boundary](https://github.com/grove/pg-mdm/blob/main/README.md).

[2] [MDM V1 design: sections 2, 8–12, and 14](https://github.com/grove/pg-mdm/blob/main/DESIGN_V1.md).

[3] [MDM V2 design: sections 2, 12, and 15](https://github.com/grove/pg-mdm/blob/main/DESIGN_V2.md).

[4] [MDM implementation roadmap: upstream entry gate and full-resolution baseline](https://github.com/grove/pg-mdm/blob/main/ROADMAP.md).

[5] [pg-trickle 0.97 plan: assurance and feature freeze](https://github.com/trickle-labs/pg-trickle/blob/main/roadmap/v0.97.0.md).

[6] [pg-react support matrix: qualified stack, keys, and RLS limits](https://github.com/trickle-labs/pg-react/blob/main/docs/support-matrix.md).
