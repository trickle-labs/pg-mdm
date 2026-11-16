# Publication, incremental work, and durable identity

[Documentation home](../README.md) · Explanation

Entity resolution becomes useful to an application when its results can be trusted across time. A correct result that appears one table at a time can still break a consumer, and a customer identifier that changes with every rebuild is difficult to use in orders or support history. pg_mdm addresses both problems through a publication boundary and a retained identity ledger. Understanding those two ideas makes refreshes, previews, upgrades, and stewardship much easier to reason about.

## A publication is a coherent observation

A successful refresh connects a proven source boundary, graph maintenance, resolution, identities, golden values, reviews, and consumed delta progress in one transaction. If the operation fails or the transaction rolls back, its partial work is not a new publication. Readers retain the previous committed result instead of seeing new memberships paired with old golden values. An application using several separate SQL statements should still choose an appropriate transaction snapshot when those statements must observe the same revision: a successful refresh can commit between ordinary reads.

A publication revision is consequently more useful than a vague “last processed” timestamp when comparing results. A source update, a steward instruction, and a completed publication are distinct events. `mdm.create()` records a desired definition; a successful refresh makes its effects active. `override_golden()` records a directive; a later refresh publishes its selected value. A no-change refresh can confirm the current state without allocating another semantic publication revision. The [refresh result](../reference/outputs.md#refresh-result) exposes these distinctions.

## Stable identity is reconciled, not recalculated from a name

An `mdm_id` is not a hash of a customer's current name or email. It is assigned and reconciled against retained memberships and identity history. That is why correcting `Acme Ltd` to `Acme Trading Ltd` in the first tutorial keeps the same customer UUID. The record still belongs to the same resolved component, and the identity ledger provides continuity independently of the selected display text.

When components merge or split, identity reconciliation uses deterministic continuity rules. An old identity's anchor is chosen among its surviving active memberships by first-membership revision, source sort key, and source-record UUID. If multiple identities claim the same new component, the oldest created identity wins, with UUID order as a tie break. A component without a surviving continuity claim receives a new ID. The ledger retains merge aliases, split relationships, and retired identities so these transitions are not erased. The precise implementation is in [src/identity.rs](../../src/identity.rs).

These rules preserve a meaningful history; they do not promise that every former customer ID stays active after a legitimate merge or split. Consumers should distinguish an active current identity from a retained historical one, and use the member table to understand active versus inactive source records. Dropping and recreating an entity discards a different lifecycle boundary from a full rebuild; `mdm_admin.rebuild()` retains the identity ledger while reevaluating the complete result.

## Deterministic semantics still require retained state

Determinism means the declared cleaner, candidate, comparator, decision, clustering, and value-selection policies resolve the same logical evidence consistently. Those semantic versions and canonical digests are retained so a physical optimization need not redefine what “same customer” means. New identity allocation and historical continuity nevertheless depend on the retained ledger; rebuilding a brand-new database from only today's rows is not the same operation as restoring the original state.

This is also why logical restore has an explicit rebinding step. Portable definitions and source identities belong in durable state, while role and relation OIDs belong to a particular database instance. Rebinding reconstructs those derived links under the correct role and validates the retained contracts. It neither invents a new source history nor waives a changed key definition. The [restore guide](../how-to/operate.md#restore-and-rebind) explains the administrator-session exception needed for that operation.

## Incremental describes a strategy, not a weaker answer

The producer can maintain graph stages differentially, and pg_mdm can resolve a closed affected set when complete terminal ranges prove that doing so is safe. A missing range, invalidation, graph transition, or uncertain limit can require the full resolver instead. Keeping `ALLOW` available is part of recovery correctness: forcing every refresh to remain incremental would make bootstrap and recovery fail even when a complete result could be produced safely.

The affected set also does not bound every cost of a refresh. Graph-helper synchronization, global checks, complete publication assembly, result digests, and retained snapshot copies can still involve the whole entity. Measurements in the repository distinguish these stages and do not establish a universal latency improvement. Read `resolver_strategy`, fallback reasons, effective node modes, and `stage_timings_ms` together. [Operations](../how-to/operate.md#understand-where-incremental-work-still-has-a-cost) links the specific evidence and its artifact identities.

## A preview is an observation with a scope

Previews let you examine a desired definition before publishing it, but the evidence boundary matters. Data-bearing previews use the last successful publication, so source changes made afterward may not be included. A sample helps you learn about a proposal; it does not establish the result for every record. A scoped preview can describe its selected subjects exactly while omitted records still change the full-entity result. Read the returned limitations and `data_as_of` as part of the result, especially when a user interface presents a reassuring preview to a reviewer.

## Receipts and publications answer different questions

Policy automation records an immutable receipt for a request outcome. That receipt answers what happened to that request under its observed versions and policy; it does not make a later publication appear retroactively. The 0.15 candidate adds a separate receipt-publication observation so integrations can distinguish a control that reached a publication from one superseded first. Historical missing timestamps or tracking rows remain unknown. The [policy guide](../how-to/automate-stewardship.md) explains how to retry an identical request and how to treat a new evaluation as new work.

**Continue with:** [Operate and restore](../how-to/operate.md), or [Output and result reference](../reference/outputs.md).
