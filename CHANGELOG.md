# Changelog

## 0.2.0

- Add developmental `mdm.source`, `mdm.field`, `mdm.match`, `mdm.golden_value`, and `mdm.entity` constructors.
- Store immutable, versioned definitions and deterministic non-executable graph artifacts.
- Validate source relations, keys, privileges, role bindings, candidate channels, and output names.
- Add `mdm.create()` and `mdm.describe()`; Graph V1 remains disabled and no public output tables are created.
- Fix installation wrappers, role checks, catalog locking, and action access to private helpers.
- Reject inherited ownership that bypasses source row-level security.
- Keep definition digests independent of live capability status and reject unsupported preset versions.
- Add administrator `mdm_admin.rebind()` to restore derived role and source bindings without rewriting durable definitions.
- Assert release checks for installation, application privileges, concurrency, and logical restore with rebinding.

## 0.1.0

- Add the PostgreSQL 18 extension package and five V1 schemas.
- Add the durable operation log and hardened installation check.
- Add the `pg_trickle` v0.98.0 capability adapter and Graph V1 gate.
- Add pinned packaging, security, restore, and end-to-end checks.
