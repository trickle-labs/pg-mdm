# Compatibility and project status

[Documentation home](../README.md) · Reference

These pages describe pg_mdm **0.15.0**, published as a [demo preview prerelease](https://github.com/trickle-labs/pg-mdm/releases/tag/v0.15.0) for **synthetic/demo data only**. The release is not production-qualified and explicitly excludes connection to real workloads or activation of live controls. [Issue #28](https://github.com/trickle-labs/pg-mdm/issues/28) tracks the blocked R1–R8 and C01–C12 qualification gates; all 20 must pass before `release_qualified: true` is justified. The version in [Cargo.toml](../../Cargo.toml), the [control file](../../pg_mdm.control), and the [SQL archive](../../sql/archive/pg_mdm--0.15.0.sql) agree on 0.15.0. Available package assets and a passing local demonstration do not change the qualification disposition.

## Current build baseline

| Component | Repository baseline |
| --- | --- |
| pg_mdm | 0.15.0 demo preview; compiler v11 |
| PostgreSQL | Major 18; Docker runtime image 18.4 Bookworm, pinned by digest |
| Target | `linux/amd64`, `x86_64-unknown-linux-gnu` |
| Rust | 1.98.0; edition 2024 |
| `pgrx` / `cargo-pgrx` | 0.18.0 |
| pg_trickle | 0.108.4 in the current executable test setup |
| pg_trickle source | `c36783a9889014a01f0a34694a40608975211560` |
| Source archive SHA-256 | `b7ac0dd0c97514c081015e30153d1287f2f21ae008a9401ebc3c14073e4b400c` |
| Preload for the learning environment | `pg_trickle,pg_mdm` |

The exact image digest and build commands live in [tests/Dockerfile.e2e](../../tests/Dockerfile.e2e); [tests/e2e.sql](../../tests/e2e.sql) selects pg_trickle 0.108.4 explicitly. [DEPENDENCIES.md](../../DEPENDENCIES.md) records the pins and explains the archive checksum. PostgreSQL 18 is the only feature target declared by this repository. Do not infer compatibility with another major version or an arbitrary pg_trickle build from a similar version number.

## Integration capabilities

pg_mdm installs a private graph through pg_trickle's Graph V1.2 integration and consumes terminal changes through Delta V1.1. The adapter checks feature vectors, contracts, source boundaries, and output-delta metadata. It accepts the supported v2/v3 row-identity profiles while its graph SQL uses the preserved `encode_row_id_v2` function. A missing, disabled, malformed, or incompatible capability fails validation rather than silently weakening the result. [The producer requirements](../../PG_TRICKLE_INCREMENTAL_REQUIREMENTS.md) retain the detailed qualification contract.

## Current limitations that affect users

| Area | Boundary |
| --- | --- |
| Release qualification | Synthetic/demo evaluation only; no real workload or live controls. R1–R8 and C01–C12 are blocked in issue #28, including the candidate's review and M4 qualification boundaries. |
| Golden source ordering | `prefer_source` uses canonical source-name order; the `sources` array filters eligibility and does not define priority. |
| Definition presets | `person`, `company`, and `product` names are version-pinned metadata. Supply the actual sources, fields, matches, and golden policies explicitly. |
| Refresh orchestration | Applications or operators call `mdm.refresh()`; defining an entity does not make its public tables update continuously by itself. |
| Incremental performance | Affected resolution and scoped state loading reduce particular kinds of work. Counts, complete publication assembly, digest generation, and retained snapshot copying can still involve global work. Existing scoped-state evidence did not show a complete-refresh speedup. |
| Policy controls | Supported automated actions are `ASSIGN_QUEUE`, `ESCALATE`, and `SET_DUE_AT`; they do not merge records or choose golden values. |
| Restart, restore, and clone | Policy runtime activation must be re-established deliberately with the current runtime version. Missing pg_mdm preload prevents activation. |
| Historical timing | Older pending decisions and receipt history can have unknown age or no receipt-to-publication row. Missing historical timestamps are not inferred. |

## Reading historical evidence

Earlier qualification artifacts remain valuable because they record exact source and binary identities, workloads, failures, and outcomes. They describe those artifacts, not every later checkout. The repository retains [v0.14 evidence](../../evidence/v0.14/pg-mdm-m2-proof.md), [compiler and terminal-delta qualification](../../evidence/issue10/README.md), [source synchronization measurements](../../evidence/issue11/README.md), [scoped-state measurements](../../evidence/issue12/README.md), and [publication acceptance](../../evidence/issue8/README.md). In particular, historical 0.108.3 producer qualification must not be relabeled as proof for the current 0.108.4 binary.

Qualification work must follow the entry conditions and all release gates in issue #28. The existing [E2E script](../../scripts/run_e2e_tests.sh) supplies part of that evidence for an exact build, including complete publications, identity history, explanations, and recovery behavior; passing it alone does not qualify this preview. The [operations guide](../how-to/operate.md) describes synthetic-data rehearsals. [DESIGN_V2.md](../../DESIGN_V2.md) and the [roadmap](../../ROADMAP.md) contain future work; a proposal there does not extend the installed SQL contract.
