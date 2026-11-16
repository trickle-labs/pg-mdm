# Operate, upgrade, and restore pg_mdm

[Documentation home](../README.md) · How-to guide

Operating pg_mdm means maintaining a clear boundary between source changes, saved instructions, and published results. This guide explains refreshes, observations, compiler and producer changes, restore, and entity retirement so you can rehearse them in an isolated synthetic-data environment. It assumes that you know each entity's execution role and can preserve a coherent backup before a rehearsal migration. The current [demo preview is not production-qualified](../reference/compatibility.md): do not connect real workloads or enable live controls. These procedures do not replace the blocked release gates in [issue #28](https://github.com/trickle-labs/pg-mdm/issues/28).

## Refresh and grant readers access

Call `mdm.refresh()` as the entity's execution role when you want committed source changes and pending instructions reflected in its public tables. A scheduled job, application workflow, or external orchestrator can issue the call; defining an entity alone does not schedule publication. Keep the default full policy `ALLOW`, which permits complete evaluation during bootstrap, recovery, and transitions. Arrange the frequency around the freshness your consumers need and the refresh cost you observe.

```sql
SELECT jsonb_pretty(mdm.refresh('customer', 'ALLOW'));
```

After the first refresh creates the output tables, grant a consumer role only the public reads it needs. Run these statements as an administrator authorized to grant access, then make your application login a member of that consumer role according to your access policy:

```sql
CREATE ROLE customer_reader NOLOGIN NOSUPERUSER NOBYPASSRLS;
GRANT USAGE ON SCHEMA mdm_out TO customer_reader;
GRANT SELECT ON mdm_out.customer, mdm_out.customer_members,
    mdm_out.customer_review TO customer_reader;
```

Explicit column or view grants may be more appropriate when golden values contain personal or sensitive data. Explanation masking does not mask the ordinary golden output table. If an application runs several reads that must refer to one publication, use a suitable PostgreSQL transaction snapshot rather than assuming separate statements cannot straddle a successful refresh.

## Observe the complete refresh outcome

Record the refresh JSON together with the entity, call time, extension versions, and deployment identity. The [output reference](../reference/outputs.md#refresh-result) explains the key fields. At a minimum, monitor publication progress and age, failures by result code, source-boundary completeness, open reviews, pending stewardship, delta lag, resolver strategy, and unexpected full fallbacks. A successful no-change refresh can retain the previous publication revision; do not treat an unchanged revision by itself as a failure.

```sql
SELECT jsonb_pretty(mdm.describe('customer', 'summary'));

SELECT status, severity, reason_code, count(*) AS cases
FROM mdm_out.customer_review
GROUP BY status, severity, reason_code
ORDER BY status, severity, reason_code;
```

Expect full work during bootstrap, rebuild, graph transitions, resnapshots, and recovery. Investigate repeated full work during a previously qualified steady-state workload, but do not block the recovery path merely to improve an incremental metric. A reported `DIFFERENTIAL` graph mode is not sufficient evidence of complete terminal capture: inspect the terminal ranges and adapter fallback diagnostics as well.

## Roll out a compiler or producer change

Before rollout, run [scripts/run_e2e_tests.sh](../../scripts/run_e2e_tests.sh) against the actual deployment candidate and retain its source commit, package manifests, producer binary checksum, effective graph modes, and terminal traces. The suite includes differential/full comparisons and recovery scenarios; old evidence for another binary does not qualify a new build. Keep `full_policy = 'ALLOW'` during the rollout. `ERROR` is for controlled steady-state qualification after bootstrap, not for normal recovery or transition work.

1. Pause managed policy delivery and preserve a coherent backup. Follow the producer's migration requirements, including requesting resnapshots for active output-delta consumers when the upgrade requires them. Install the matching producer and adapter files, apply their extension updates, and restart when preload or shared-library changes require it.
2. Rerun `sql/configure_helper.sql` for the configured helper owner and review action grants. Select one entity as a canary and capture its public rows, current definition, and `mdm.describe()` result.
3. Grant the canary's execution role `EXECUTE` on `mdm_admin.recompile(text)` if needed. Connect through its restricted login, select that execution role, and call `SELECT mdm_admin.recompile('customer');`.
4. Call `SELECT mdm.refresh('customer', 'ALLOW');`. The first refresh after a graph transition uses the full resolver. Verify the intended output semantics, identity continuity, and zero delta lag before introducing further changes.
5. Apply a representative source change. For a qualified affected-refresh scenario, confirm `resolver_strategy = 'affected'` and no `unexpected_full_fallbacks`, then compare complete results with the expected publication. Investigate differences before continuing to other entities.
6. Repeat the canary checks for the remaining entities. Revalidate and explicitly activate policy bindings for the current server runtime before resuming managed delivery.

The current helper setup delegates reads on `pgtrickle.pgt_dependencies`, `pgtrickle.pgt_stream_tables`, and `pgtrickle.pgt_change_tracking` to execution roles bound to current graphs. Rerunning the supplied script after an upgrade or restore establishes those grants through the protected helper. Copying a fragment of an older setup recipe can miss a new requirement.

## Understand where incremental work still has a cost

Source registration consumes complete output-delta ranges from `records/*` nodes before the downstream graph refresh, while producer source locks keep registration and publication at the same transaction boundary. Qualified ordinary permanent tables without row-level security or inheritance can use changed encoded keys, including both sides of key changes and soft-delete transitions. Empty ranges read no source keys. Bootstrap, rebuild, incomplete capture, resnapshots, and other source shapes use a complete scan at that same boundary. Source privileges are checked on every refresh, including no-change calls, and the active-record limit applies to the entire entity.

Affected resolution loads prior identities, historical memberships, reviews, golden provenance, pair facts, and overrides through its closed scope. Some override changes can require entity-wide work. Complete publication assembly, canonical digest generation, retained snapshot copying, graph-helper synchronization, and global counts still have broader costs. Inspect `stage_timings_ms`, including `evaluation_state_loading`, `merged_state_construction`, `digest_generation`, and `snapshot_copying`, instead of equating a small affected set with a small total refresh.

The retained [source synchronization](../../evidence/issue11/README.md), [scoped-state](../../evidence/issue12/README.md), [publication acceptance](../../evidence/issue8/README.md), and [terminal-delta qualification](../../evidence/issue10/README.md) records explain these boundaries and their measurements. The scoped-state measurement did not show a complete-refresh speedup. Diagnostic `mdm.trace_graph_deltas` tracing is useful during qualification, but measure its effect before leaving it enabled in an operational workload.

## Upgrade the extension

Check installed and available versions, review the [changelog](../../CHANGELOG.md) and [dependency record](../../DEPENDENCIES.md), and rehearse the upgrade on a restored copy. After installing the new extension files, an administrator can use `ALTER EXTENSION pg_mdm UPDATE TO '0.15.0';` when the installed version has a supplied upgrade path. The [upgrade scripts](../../sql/) and [path checker](../../scripts/check_upgrade_paths.py) define the repository's migration chain. Do not treat replacing the shared library alone as an SQL-catalog upgrade.

Reapply helper ownership and grants using `configure_helper.sql`, verify the installation as the restricted application role, and inspect each entity before refreshing. When the producer changes row-identity or output-delta contracts, perform its required resnapshot process before the incompatible extension update. Preserve publication and identity continuity checks as part of the rehearsal, not only the server's ability to start.

## Restore and rebind

Logical dumps preserve entity definitions, durable source identities, graph artifacts, and committed operation data. Database-local role and relation OIDs are derived bindings and are excluded from logical dumps. Restore source tables and durable MDM data coherently, recreate the required roles, rerun `sql/configure_helper.sql`, and restore the execution role's source privileges and row-level security policies before rebinding.

`rebind` is a deliberate exception to the ordinary caller rule: it requires a **superuser-authenticated session** with the stored execution role selected. As that superuser, grant the action and select the role:

```sql
GRANT USAGE ON SCHEMA mdm_admin TO app_mdm_admin;
GRANT EXECUTE ON FUNCTION mdm_admin.rebind(text) TO app_mdm_admin;
SET ROLE app_mdm_admin;
SELECT mdm_admin.rebind('customer');
RESET ROLE;
```

Repeat for each entity under its stored execution role. Rebinding validates desired mappings and every retained source identity, including sources no longer present in the desired definition. Portable relation names and frozen key contracts must match. The action replaces derived bindings and records the operation without rewriting definitions, artifacts, or digests. It can also support an explicitly planned replacement of a role or source table under the same name and contract; it cannot turn an incompatible key into a compatible history.

After restart, restore, or clone, policy intents need runtime activation for the current environment. Keep delivery paused, inspect the restored installation, and use the [policy activation procedure](automate-stewardship.md#pause-replace-and-reactivate-bindings). An existing matching receipt can remain replayable while new intents are blocked, so replay success alone is not proof that fresh delivery has been activated.

## Rebuild or retire an entity

Use `mdm_admin.rebuild('customer', 'ALLOW')` when you need a complete resolver pass while retaining the identity ledger. Grant the action explicitly and run it as the entity execution role. Rebuild is useful for verification and recovery; it is not the same as dropping and recreating the entity, which discards a different set of state and identity guarantees.

To retire an entity, review downstream dependencies and preserve any required history first. `mdm_admin.drop_entity(entity_name, confirm)` requires the confirmation text to equal the entity name; for example, `SELECT mdm_admin.drop_entity('customer', 'customer');` is destructive. Treat that statement as a separately reviewed maintenance action. Removing only a public table or an internal graph member bypasses the lifecycle contract and can leave an unusable installation.
