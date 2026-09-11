# Changelog

## Unreleased

- Pin and admit `pg_trickle` v0.104.0 with stable Graph V1 capability, contract, ownership, RLS, source-boundary, and transactional rollback coverage.
- Keep Delta V1 outside the V1 publication path and record Graph V1's owner-equivalent source requirement for v0.8.

## 0.7.0

- Add deterministic stable identity reconciliation with merge aliases, split history, and membership tombstones.
- Add golden selectors, anchored override history, review occurrences, bounded explanations, and publication/output metadata.
- Add the v0.6.0 to v0.7.0 upgrade path and archived release artifact.

## 0.6.0

- Add deterministic conservative clustering with manual closure, cannot-links, authority conflicts, and component admission.
- Return complete memberships plus accepted and rejected union facts with fail-closed resolver limits.
- Add the v0.5.0 to v0.6.0 upgrade path and archived release artifact.

## 0.5.0

- Add exact and bounded normalized-Levenshtein evidence with deterministic pair precedence and edge ordering.
- Add durable steward `MATCH` and `NOT_MATCH` decisions with optimistic concurrency, supersession history, and contradiction checks.
- Add the v0.4.0 to v0.5.0 upgrade path and archived release artifact.

## 0.4.0

- Add deterministic exact, composite, prefix, and token candidate channels.
- Canonicalize and deduplicate candidate pairs by stable source sort key.
- Fail closed on oversized blocks, aggregate candidate limits, invalid plans, and sort-key collisions.
- Compile separate block, block-statistics, overflow, pair-statistics, and canonical-pair graph stages.
- Extend `mdm.describe()` with the logical candidate plan and versioned candidate limits.
- Add the 0.3.0 to 0.4.0 upgrade path and archived release artifact.

## 0.3.0

- Add versioned built-in cleaners: `text`, `person_name`, `company_name`, `email`, `phone`, `tax_id`, `date`, and `none`.
- Add seven normalized-value states: `value`, `absent`, `empty`, `invalid`, `unknown`, `redacted`, and `unsupported`.
- Implement canonical binary ordering for normalized values (`canonical_bytes`) with total locale-independent ordering.
- Add composite type `mdm_internal.normalized_value` and internal normalizers `mdm_internal.normalize_text` and `mdm_internal.normalize_date`.
- Add durable table `mdm_internal.source_records` and `get_or_create_source_record` with `pgtrickle.encode_row_id_v2` key generation and pg_dump configuration.
- Generate executable record and normalization stage SQL compilation in graph artifacts (compiler version 2).
- Extend `mdm.describe()` summary output with source-key encoding version 2, selected cleaners, cleaner versions, and options.
- Provide tested upgrade paths for `0.1.0 -> 0.2.0 -> 0.3.0` and direct `0.2.0 -> 0.3.0`.

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
