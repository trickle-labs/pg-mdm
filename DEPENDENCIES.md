# Dependency lock

This file records the current locked build inputs. `Cargo.lock` pins the Rust dependency graph. Run `just package-manifest` to record the checksums of every file in a built `pg_mdm` package.

| Input | Locked value |
|---|---|
| Rust edition | 2024 |
| Rust toolchain | 1.98.0 |
| Build target | `x86_64-unknown-linux-gnu` |
| `Cargo.lock` SHA-256 | `f96a398612cfe55176933349dbb84d7156691d5e1714d124909ce0b143638a01` |
| `cargo-pgrx` and `pgrx` | 0.18.0 |
| Runtime PostgreSQL | 18.4, Debian Bookworm |
| Build/test `pg_config` | 18.6 (`18.6-1.pgdg12+2`) |
| PostgreSQL image | `postgres:18.4-bookworm@sha256:efef99e1558f86089bc84bece29208c0777a185ff717ec7fa288a652ce2d0adf` |
| `pg_trickle` version | 0.108.2 |
| `pg_trickle` commit | `99348f2fcfb9dfbfef5b22968d3a62b7d3620535` |
| `pg_trickle` tag | lightweight `v0.108.2` tag targeting the commit above |
| `pg_trickle` artifact | `pg_trickle-0.108.2-pg18-linux-amd64.tar.gz` |
| Artifact URL | `https://github.com/trickle-labs/pg-trickle/releases/download/v0.108.2/pg_trickle-0.108.2-pg18-linux-amd64.tar.gz` |
| Artifact SHA-256 | `b2d8c2a429a6cb6a61b18ea97712ab585e111108a80b6f6a9f3c1a13854090f8` |
| Capture mode | trigger |
| E2E database `LC_COLLATE` / `LC_CTYPE` | `en_US.utf8` / `en_US.utf8` |
| Normalization fixture SHA-256 | `c01d1b0fe2b938dd09d3afa48e4e43bf648b47d0715595aa0b208ffca68826aa` |
| Candidate fixture SHA-256 | `a1e25c791b20c4420b665ee5d36fd4b9ea4946be0d60deb87ac1e824858b6478` |
| Comparator fixture SHA-256 | `87802cdfad5c4ed7a917bf4e0b184beb24836203a8c58b1477403fc5fe44cfd0` |
| Pair-precedence fixture SHA-256 | `ed2324ebcb1744ed5cf945189b7a5b30bb1838a38cd1c6f537a206f79986bd33` |
| Organization-quality fixture SHA-256 | `f79acb4273716f9b6d9ca9c3bf7999edb21440798a4a06ae1e135c909c70dbbb` |

The adapter accepts the exact Graph V1.2 feature vectors with either `stable_row_identity_encoder_v2` or `stable_row_identity_encoder_v3`, alongside `custom_table_srf_out_columns` and `lateral_immutable_composite_function`. pg_mdm's graph SQL calls `pgtrickle.encode_row_id_v2`; pg_trickle 0.108.2 preserves that function and its bytes. Its v3 encoder changes only `bpchar` trailing-space canonicalization. pg_mdm records the output-delta row-identity version returned by pg_trickle instead of assuming a fixed version.

Existing v2-backed pg_trickle stream tables still require pg_trickle's protected FULL reinitialization after upgrading to v0.108.2. Accepting the v3 capability does not skip that upstream migration requirement.

The pg-trickle v0.108.2 release includes package/runtime qualification and benchmark smoke tests. Its 72-hour soak and seven-day longevity runs remain deferred upstream; these are deployment limits, not pg-mdm admission evidence.

## Archived v0.1 Linux package checksums

| Packaged file | SHA-256 |
|---|---|
| `usr/lib/postgresql/18/lib/pg_mdm.so` | `3d341b86ecc616cba5810f43ae0fd1797179a721e6574edc99c8d1d1ec7a6704` |
| `usr/share/postgresql/18/extension/pg_mdm.control` | `4f909208703ce8242fb06c4c1f7958f15bc432ae4faca0a40e35d572872f7946` |
| `usr/share/postgresql/18/extension/pg_mdm--0.1.0.sql` | `d9902b5160793bc5eb0f8a2924b905d0aab525ff796a3f0455a5330157d6d548` |
