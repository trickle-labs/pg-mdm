# The security and ownership model

[Documentation home](../README.md) · Explanation

pg_mdm needs to maintain protected definitions, identity history, graph state, operations, and publication data while honoring the source access of the role responsible for an entity. Giving every application direct ownership of those objects would blur that responsibility. The extension instead separates installation administration, a sealed helper owner, an entity execution role, and consumers. This explains why a setup can have enough PostgreSQL privileges to read a table and still be rejected as an unsafe pg_mdm caller.

## Four responsibilities, separate identities

| Identity | Responsibility | Important boundary |
| --- | --- | --- |
| Installation administrator | Install extensions, configure helper ownership, create roles, grant actions, and perform restore administration. | Ordinary entity work uses a restricted application session. |
| Helper owner | Own protected objects used by tightly controlled helper functions. | Must be `NOLOGIN NOSUPERUSER NOBYPASSRLS`, with no members and no memberships. |
| Entity execution role | Define, preview, refresh, explain, and perform explicitly granted actions for its bound entity. | Must have the required source access and match the entity's retained role binding. |
| Consumer role | Read approved golden, membership, or review outputs. | Ordinary PostgreSQL grants control the data it can see. |

An automation worker is another restricted role with a narrower policy binding. Its permitted writes are queue assignment, due-date control, and escalation for bound cases; it is not the helper or entity administrator. Read privileges on the policy projection remain table privileges, so write authorization must not be confused with per-entity read isolation.

## Why both session and selected role matter

PostgreSQL distinguishes the authenticated `session_user` from the selected `current_user`. `SET ROLE` changes the selected role but leaves the authenticated identity in place. pg_mdm validates both: ordinary callers cannot be superusers or have `BYPASSRLS`, and they cannot cross the protected helper boundary. This prevents a misleading setup in which an operation appears to run under a restricted role while its authenticated context still has privileges that invalidate the intended assumptions.

The tutorials use `SET SESSION AUTHORIZATION tutorial_login` inside a disposable administrator session, followed by `SET ROLE tutorial_admin`, so both identities match the learning setup. An application normally connects directly as its restricted login and then selects the action role. `mdm_admin.rebind()` is an intentional exception: restoring database-local bindings requires a superuser-authenticated session with the stored execution role selected. That exception is limited to the rebinding workflow and does not change the normal caller rule.

## The helper is sealed by ownership and grants

Run `sql/configure_helper.sql` to establish the protected ownership layout instead of reproducing it with a few hand-written `ALTER` statements. The script checks the helper role, sets the required ownership and privileges, and establishes producer-metadata access needed by current graph execution roles. It must be rerun after extension upgrade or a clean logical restore because new or restored objects can otherwise have the wrong owner or grants.

The extension's protected helpers perform controlled actions through fixed interfaces. Applications call the public `mdm`, `mdm_admin`, and `mdm_steward` functions for which they have grants; they do not receive broad write access to `mdm_internal` or private graph tables. Committed operations record the authenticated actor and selected role. A rollback removes its transaction's success record along with the rest of the work, so the operation log represents committed outcomes rather than a separate guarantee that every attempted request left an audit row.

## Source access is checked where it matters

An entity's execution role must have owner authority or delegated source-schema `USAGE` and table `SELECT, MAINTAIN`. Row-level security remains part of source validation. If the execution role has owner privileges on an RLS source, the contract requires `FORCE ROW LEVEL SECURITY` to avoid silently relying on the usual owner bypass. Source privileges are rechecked during refresh, including no-change calls, rather than treated as a one-time installation assumption.

The same principle explains retained source contracts. A restored or replaced table with the same name is not automatically interchangeable if its keys, column types, or identity contract changed. Rebinding validates that contract before deriving a new relation OID. This keeps recovery from becoming an accidental permission or identity migration.

## Output visibility is an application decision

Golden tables can contain personal names, addresses, or identifiers selected from the sources. A field's `display = 'masked'` is not a substitute for PostgreSQL access control and does not turn the golden output column into masked data. Grant consumers only the tables, columns, or application views they need. Review membership access as carefully as golden-value access, because source identifiers and joins can themselves reveal sensitive relationships.

Policy projections deliberately contain approved case metadata, versions, and controls rather than serving as an unrestricted editing interface. Before using a worker role across services or tenants, review the actual relation grants and metadata included in the projection. The binding constrains submitted actions; it is not a general-purpose tenant-isolation layer.

## Runtime activation protects automation after recovery

A policy binding records durable authorization, while its active runtime state is tied to the current database, role identity, and preloaded server epoch. After restart, restore, or clone, an administrator must validate the environment and explicitly activate delivery using the current runtime version. This prevents a copied database from immediately behaving as another authorized worker destination merely because it contains old binding rows. An existing matching receipt may still be replayable; that preserves request history without authorizing a fresh action.

**Continue with:** [Install and verify roles](../how-to/install.md), [Restore and rebind](../how-to/operate.md#restore-and-rebind), or [Automate stewardship](../how-to/automate-stewardship.md).
