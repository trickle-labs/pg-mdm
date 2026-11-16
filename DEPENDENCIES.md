# Dependency lock

This file records the current build inputs for the pg_mdm 0.15.0 demo preview. `Cargo.lock` pins the Rust dependency graph. The executable build recipe is [tests/Dockerfile.e2e](tests/Dockerfile.e2e), and [tests/e2e.sql](tests/e2e.sql) selects the producer extension version. Run `just package-manifest` to record the checksums of every file in a built package. For installation and the scope of qualification claims, see [Install pg_mdm](docs/how-to/install.md) and [Compatibility and status](docs/reference/compatibility.md).

| Input | Locked value |
|---|---|
| Rust edition | 2024 |
| Rust toolchain | 1.98.0 |
| Build target | `x86_64-unknown-linux-gnu` |
| `Cargo.lock` SHA-256 | `a789757499332d8d462391627fc305e4b35b7590fefb15a3c1187dedf900b110` |
| `cargo-pgrx` and `pgrx` | 0.18.0 |
| Runtime PostgreSQL | 18.4, Debian Bookworm |
| Build/test `pg_config` | PostgreSQL 18 development package from the build image's configured apt repository; record the resolved package version for each build |
| PostgreSQL image | `postgres:18.4-bookworm@sha256:efef99e1558f86089bc84bece29208c0777a185ff717ec7fa288a652ce2d0adf` |
| `pg_trickle` version | 0.108.4 |
| `pg_trickle` commit | `c36783a9889014a01f0a34694a40608975211560` |
| `pg_trickle` build source | Exact revision pinned in `tests/Dockerfile.e2e`; historical 0.108.3 qualification is described below |
| `pg_trickle` artifact | Complete Git source archive, compiled with the locked toolchain above |
| Artifact URL | `https://github.com/trickle-labs/pg-trickle.git` at the commit above |
| Artifact SHA-256 | `b7ac0dd0c97514c081015e30153d1287f2f21ae008a9401ebc3c14073e4b400c` |
| Capture mode | trigger |
| E2E database `LC_COLLATE` / `LC_CTYPE` | `en_US.utf8` / `en_US.utf8` |
| Normalization fixture SHA-256 | `c01d1b0fe2b938dd09d3afa48e4e43bf648b47d0715595aa0b208ffca68826aa` |
| Candidate fixture SHA-256 | `a1e25c791b20c4420b665ee5d36fd4b9ea4946be0d60deb87ac1e824858b6478` |
| Comparator fixture SHA-256 | `87802cdfad5c4ed7a917bf4e0b184beb24836203a8c58b1477403fc5fe44cfd0` |
| Pair-precedence fixture SHA-256 | `ed2324ebcb1744ed5cf945189b7a5b30bb1838a38cd1c6f537a206f79986bd33` |
| Organization-quality fixture SHA-256 | `f79acb4273716f9b6d9ca9c3bf7999edb21440798a4a06ae1e135c909c70dbbb` |

The adapter accepts the exact Graph V1.2 feature vectors with either `stable_row_identity_encoder_v2` or `stable_row_identity_encoder_v3`, alongside `custom_table_srf_out_columns` and `lateral_immutable_composite_function`. pg_mdm's graph SQL calls `pgtrickle.encode_row_id_v2` and records the output-delta row-identity version returned by pg_trickle instead of assuming a fixed version. Validate the capabilities and actual installed binaries together; a version string alone does not establish conformance.

The source checksum covers `git archive --format=tar` with `* -export-ignore` in `.git/info/attributes`. This retains `Cargo.lock`, benches, and the build-script generator that GitHub's default source archive excludes. The build verifies that packaging leaves `Cargo.lock` unchanged. Each deployment candidate still needs runtime qualification with its actual binary checksum; the preview label does not assert a completed stable release gate.

## Historical 0.108.3 qualification

Earlier evidence used pg_trickle commit `d8bb8eb5d45c5c9982e173e88914037433a1ee05` with archive SHA-256 `b8d4c59ca3770c305659255e09adde303b4d220228392c415ae363aa7be0a2da`, based on `v0.108.3` plus [exact graph delta repairs](https://github.com/trickle-labs/pg-trickle/pull/1171). Those inputs remain relevant when reading the retained evidence, but they are not the source pin used by the current Docker build. The historical 0.108.3 encoder preserved the v2 function and bytes; its v3 change concerned `bpchar` trailing-space canonicalization.

The 0.108.3 identity migration can require active output-delta consumers to resnapshot before the extension upgrade. For pg_mdm consumers, request the resnapshot before upgrading; pg_mdm's next refresh rebuilds its state from the new snapshot and acknowledges fresh tokens. The upstream migration retains its fail-closed block until consumers are ready.

The compiler v11 qualification required the source repairs: keyless scans must reconstruct pre-change join snapshots, and output capture must recognize explicit DML that writes no rows. The unpatched 0.108.3 release archive did not provide this conformance. The retained E2E evidence records both the source archive checksum and the installed binary checksum.

The pg-trickle v0.108.3 release passed package/runtime and upgrade qualification. Its 72-hour soak and seven-day longevity runs remain deferred upstream. Each build of the patched producer requires runtime qualification with its actual binary checksum.

## Archived v0.1 Linux package checksums

| Packaged file | SHA-256 |
|---|---|
| `usr/lib/postgresql/18/lib/pg_mdm.so` | `3d341b86ecc616cba5810f43ae0fd1797179a721e6574edc99c8d1d1ec7a6704` |
| `usr/share/postgresql/18/extension/pg_mdm.control` | `4f909208703ce8242fb06c4c1f7958f15bc432ae4faca0a40e35d572872f7946` |
| `usr/share/postgresql/18/extension/pg_mdm--0.1.0.sql` | `d9902b5160793bc5eb0f8a2924b905d0aab525ff796a3f0455a5330157d6d548` |
