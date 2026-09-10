# pg_mdm

**Deterministic entity resolution and golden records, designed to run inside PostgreSQL.**

> [!IMPORTANT]
> v0.3 introduces normalization and durable source records with versioned cleaners. It does not execute graph SQL, resolve records, or create public output tables.

Most organizations have several records for the same customer, company, supplier, or product. Those records rarely agree perfectly: names are formatted differently, contact details go stale, source systems reuse identifiers, and one weak match can accidentally join two unrelated groups. `pg_mdm` resolves those records into durable real-world entities while keeping every automatic decision deterministic, conservative, and explainable.

The project is built around a deliberate division of responsibility. [`pg_trickle`](https://github.com/trickle-labs/pg-trickle) captures source changes and incrementally maintains relational facts such as normalized values, candidate pairs, and matching evidence. `pg_mdm` decides what those facts mean: which records belong together, which human decisions take precedence, which stable ID survives a merge or split, which value becomes golden, and which uncertain cases need review. In short, **`pg_trickle` maintains changing relational facts; `pg_mdm` decides identity.**

## Install v0.3 (developmental definitions and normalization)

v0.3 supports PostgreSQL 18 and requires `pg_trickle` 0.98.0. Add `pg_trickle` to `shared_preload_libraries`, restart PostgreSQL, and install `pg_trickle` first. [`DEPENDENCIES.md`](DEPENDENCIES.md) records the release artifact and image digests.

The v0.3 actions store and validate definitions and compile executable record and normalization stages. They do not execute graph SQL, resolve records, or create public output tables.

Build and copy the package:

```bash
cargo pgrx package --pg-config "$(command -v pg_config)"
sudo cp target/release/pg_mdm-pg18/*/lib/postgresql/18/lib/pg_mdm.* "$(pg_config --pkglibdir)/"
sudo cp target/release/pg_mdm-pg18/*/share/postgresql/18/extension/pg_mdm* "$(pg_config --sharedir)/extension/"
```

Install both extensions as a database administrator:

```sql
CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm;

CREATE ROLE mdm_helper_owner NOLOGIN NOSUPERUSER NOBYPASSRLS;
```

Assign the protected objects to the helper owner before you grant action access:

```bash
psql --set=helper_owner=mdm_helper_owner --file=sql/configure_helper.sql my_database
```

Create application roles outside the extension. v0.1 has one administrator action that verifies the installation and records the result:

```sql
CREATE ROLE app_mdm_admin NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE app_login LOGIN NOSUPERUSER NOBYPASSRLS;
GRANT app_mdm_admin TO app_login WITH SET TRUE, INHERIT FALSE;

GRANT USAGE ON SCHEMA mdm_admin TO app_mdm_admin;
GRANT EXECUTE ON FUNCTION mdm_admin.verify_installation() TO app_mdm_admin;

-- v0.2 definition actions
GRANT USAGE ON SCHEMA mdm TO app_mdm_admin;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA mdm TO app_mdm_admin;
```

Connect as `app_login`, select the action role, and run the check:

```sql
SET ROLE app_mdm_admin;
SELECT mdm_admin.verify_installation();
```

The function rejects superusers, `BYPASSRLS` roles, login-capable helper owners, helper owners with role memberships, and mismatched function and table owners. It records `session_user` as `actor_name` and the selected role as `actor_role_name`. Callers cannot supply either value, the outcome, the status, or the result code.

Run `sql/configure_helper.sql` again after an extension upgrade or a clean logical restore, then reapply action grants. After a logical restore, [rebind the restored entities](#rebind-restored-entities) before using definition actions. PostgreSQL includes `mdm_internal.operations` data in logical dumps. The helper writes only committed success records; a transaction rollback removes its operation row.

The v0.1 stable result codes are:

| Code | Meaning |
|---|---|
| `MDM_OK` | The installation check succeeded. |
| `MDM_PGT_CAPABILITY_MISSING` | Graph V1 is absent. |
| `MDM_PGT_CAPABILITY_VERSION` | Graph V1 has an unsupported major version. |
| `MDM_PGT_CAPABILITY_INVALID` | The capability response has duplicate or invalid rows. |
| `MDM_PGT_CAPABILITY_DISABLED` | Graph V1 1.x exists but is disabled. |
| `MDM_HELPER_OWNER_UNSAFE` | Protected objects do not have the documented helper owner. |
| `MDM_UNAUTHORIZED` | The authenticated or selected role is unsafe. |
| `MDM_OPERATION_STATE` | A running operation could not commit as succeeded. |
| `MDM_INTERNAL` | PostgreSQL SPI returned an unexpected error. |

## Rebind restored entities

Logical dumps preserve entity definitions, source identities, and graph artifacts. Database-local role and relation OIDs are derived bindings and are excluded from dumps.

After restoring the source tables and durable MDM data, run `sql/configure_helper.sql` and reapply action grants. Restore the execution role's source privileges and row-level security policies before rebinding.

Connect as a superuser, grant the stored execution role access to `rebind`, and select that role:

```sql
GRANT USAGE ON SCHEMA mdm_admin TO app_mdm_admin;
GRANT EXECUTE ON FUNCTION mdm_admin.rebind(text) TO app_mdm_admin;
SET ROLE app_mdm_admin;
SELECT mdm_admin.rebind('customer');
RESET ROLE;
```

Repeat the call for each restored entity under its stored execution role. `rebind` requires a superuser-authenticated session, so an application login cannot use it even with the function grant. Source validation runs under the selected role's privileges and row-level security.

Rebinding checks the desired definition's mappings and every retained source identity, including sources removed from the current definition. The portable relation names and frozen key contracts must match. The call replaces derived role and source bindings and records the operation without changing definitions, artifacts, or digests. It also supports an explicitly approved replacement of a role or source table under the same name and contract.

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

The implemented v0.3 release stores developmental definitions, compiles record and normalization stage SQL, and manages durable source records. The planned V1 release resolves records already stored in supported local or partitioned PostgreSQL tables. It covers tracked and soft-delete sources, deterministic built-in matching, bounded candidate generation, full-entity resolution, stable IDs, field-level golden records, pair-level stewardship, review, explanation, and atomic publication. V1 intentionally leaves complete snapshots, custom matching code, approximate retrieval, valid-time history, direct merge and split workflows, multi-entity dependencies, resumable runs, namespaces, quotas, and other enterprise controls outside its first compatibility promise.

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
