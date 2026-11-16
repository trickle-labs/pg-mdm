# Troubleshoot pg_mdm

[Documentation home](../README.md) · How-to guide

Start with the failed operation, its `MDM_*` code, and the role that issued it. pg_mdm deliberately rejects an unsafe caller, incomplete candidate evaluation, or unproven source boundary rather than publishing a plausible but incomplete result. A failed transaction leaves its previous committed publication available. The most productive response is to identify which contract failed, correct that condition, and retry the intended operation with current versions.

## Establish the environment

As an administrator, record the installed versions and preload setting. In the application session, record both authenticated and selected identities. These two observations often explain an installation failure more quickly than changing permissions broadly.

```sql
SELECT extname, extversion
FROM pg_extension
WHERE extname IN ('pg_mdm', 'pg_trickle')
ORDER BY extname;
SHOW shared_preload_libraries;
SELECT session_user, current_user;
```

For a running entity, obtain `mdm.describe('customer', 'summary')` as its execution role. Read the desired and active versions, graph blocking errors, pending stewardship, and latest refresh outcome. If even `describe` fails after a logical restore, complete [Restore and rebind](operate.md#restore-and-rebind) first rather than rewriting the definition.

## Installation and authorization

| Symptom or code | What to check and do |
| --- | --- |
| Extension is unavailable | Confirm its files were installed in the paths returned by the server's PostgreSQL 18 `pg_config`. A client installation of `psql` is not an extension installation. |
| `MDM_HELPER_OWNER_UNSAFE` | Run `sql/configure_helper.sql` as the installation administrator with the intended helper. Confirm `NOLOGIN`, `NOSUPERUSER`, `NOBYPASSRLS`, no members, no memberships, and consistent protected-object ownership. |
| `MDM_UNAUTHORIZED` | Check both session and selected role. Use a restricted login, then select the entity's bound execution role. `SET ROLE` from a superuser session alone is insufficient for ordinary actions. |
| `MDM_PGT_CAPABILITY_*` | Compare the installed producer with the pinned baseline and capability contract. Install and qualify a compatible build; a similar release number does not establish compatibility. |
| Permission denied on source | Grant the execution role source-schema `USAGE` and table `SELECT, MAINTAIN`, then examine any row-level security policies under that role. |
| Helper include file not found | Run `psql` from the repository root, or use `\cd` to that root before `\i sql/configure_helper.sql`. The Docker guide mounts the repository at `/work`. |
| Docker image build rejects the checkout | The supplied image script requires a clean tree so its source label is accurate. Commit the intended changes or use a clean checkout of the same revision. |

Do not make the helper a login or give application roles superuser privileges to get around a failed check. Those changes alter the boundary the installation check is designed to verify. The [security explanation](../explanation/security.md) shows the role of each identity.

## Definition and matching behavior

| Symptom or code | What to check and do |
| --- | --- |
| `MDM_VERSION_CONFLICT` | Read the current desired definition and its version, reconcile concurrent edits, and retry the reviewed proposal with that version. |
| `MDM_SOURCE_INVALID` | Check relation shape, key-column order, immediate unique index, null handling, field mappings, types, and source privileges. |
| `MDM_DEFINITION_INVALID` | Check the exact argument names, supported cleaners/comparators, evidence lineage, candidate specification, and limit bounds in [Definition reference](../reference/definitions.md). |
| Duplicate-looking rows stay separate | Inspect their cleaned values, candidate channel, match evidence, manual constraints, and component admission. A fuzzy comparator only sees pairs admitted by the candidate predicates. |
| Unexpected merge or review | Revisit whether the evidence identifies the intended real-world entity. Shared emails and names may be weaker than the business rule assumes. Inspect a pair or review explanation before broadening rules. |
| Reversing `sources` does not change the golden winner | The current implementation uses canonical source-name order as priority. The golden `sources` list filters eligibility. |
| Output field cannot be removed or changed | Published field order and types are append-only. Plan an output migration rather than trying to bypass the retained schema contract. |
| `MDM_DECISION_VERSION_CONFLICT` or `MDM_GOLDEN_OVERRIDE_CONFLICT` | Reload the current instruction and its version. A clear or replacement does not reset history to version 0. |
| `MDM_DECISION_CONTRADICTION` | Review the complete must-link/cannot-link component. Replacing one instruction may conflict with another durable constraint. |

## Refresh, limits, and recovery

| Symptom or code | What to check and do |
| --- | --- |
| `MDM_CANDIDATE_BLOCK_LIMIT` or `MDM_CANDIDATE_TOTAL_LIMIT` | Inspect the oversized value or candidate predicate. Split a poorly discriminating block or deliberately raise a tested bound; the engine does not silently sample away candidates. |
| `MDM_COMPARATOR_LIMIT`, `MDM_RESOLVER_LIMIT`, or `MDM_DECISION_CHECK_LIMIT` | Examine the specific resource and workload. Evaluate a smaller representative case, then redesign the definition or qualify a larger bound. |
| `MDM_DELTA_PROTOCOL`, `MDM_GRAPH_CONTRACT`, or `MDM_REFRESH_BOUNDARY` | Preserve the error and refresh evidence, verify producer/adapter identities, and follow the required recompile or resnapshot path. Do not edit cursors or acknowledge unconsumed ranges manually. |
| Full resolver after an upgrade or rebuild | This is expected during transition and recovery. Investigate repeated full fallback only after the workload has reached a qualified steady state. |
| `changed = false` | A successful no-change refresh can keep the previous revision. Compare source changes, desired/active definitions, and pending instructions before treating it as a failure. |
| Saved override is absent from output | Refresh after the instruction, then inspect the publication and override anchor. Saving an instruction and publishing it are separate steps. |
| `MDM_EXPLANATION_NOT_RETAINED` | The requested revision's facts are unavailable. Use a retained revision; do not substitute current evidence as historical proof. |
| New policy intents fail after restart | Check pg_mdm preload and explicitly reactivate the binding with its current runtime version after validating the environment. Receipt replay may still work before fresh delivery is activated. |

## Prepare a useful problem report

Include the exact source or release revision, extension versions, producer source and binary identities when available, the operation and result code, and the smallest definition and dataset that reproduce the failure. Include the complete refresh JSON or relevant `describe` output, whether the problem followed an upgrade or restore, and the role setup without credentials. For performance questions, identify dataset shape, candidate density, affected records, fallback reasons, and stage timings; an isolated elapsed-time number is rarely enough to explain the cost.

Remove personal values, secrets, connection passwords, and unrelated records before sharing a fixture. Preserve the structural feature that causes the problem, such as a reused key, a shared email, a null timestamp, or a large candidate block. Open an issue in [trickle-labs/pg-mdm](https://github.com/trickle-labs/pg-mdm/issues) with those details so the result can be investigated and reproduced.
