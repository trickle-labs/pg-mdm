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
| `pg_trickle` version | 0.106.1 |
| `pg_trickle` commit | `df0e9c1d89dc4ccf920f3e1519d2b7f86990a4bf` |
| `pg_trickle` tag object | annotated tag targeting the commit above |
| `pg_trickle` artifact | `pg_trickle-0.106.1-pg18-linux-amd64.tar.gz` |
| Artifact URL | `https://github.com/trickle-labs/pg-trickle/releases/download/v0.106.1/pg_trickle-0.106.1-pg18-linux-amd64.tar.gz` |
| Artifact SHA-256 | `e6976e4e6477b5241008f5ec3b48edb395b600aea4fe78c1944e69e12046d2bf` |
| Capture mode | trigger |
| E2E database `LC_COLLATE` / `LC_CTYPE` | `en_US.utf8` / `en_US.utf8` |
| Normalization fixture SHA-256 | `c01d1b0fe2b938dd09d3afa48e4e43bf648b47d0715595aa0b208ffca68826aa` |
| Candidate fixture SHA-256 | `a1e25c791b20c4420b665ee5d36fd4b9ea4946be0d60deb87ac1e824858b6478` |
| Comparator fixture SHA-256 | `87802cdfad5c4ed7a917bf4e0b184beb24836203a8c58b1477403fc5fe44cfd0` |
| Pair-precedence fixture SHA-256 | `ed2324ebcb1744ed5cf945189b7a5b30bb1838a38cd1c6f537a206f79986bd33` |
| Organization-quality fixture SHA-256 | `f79acb4273716f9b6d9ca9c3bf7999edb21440798a4a06ae1e135c909c70dbbb` |

The baseline capability response contains `external_graph_refresh 1.1 enabled=true` and `output_delta_consumer 1.0 enabled=true`, both with stable status. v0.13 uses Graph V1 and Delta V1.

The pg-trickle 0.106.1 release includes package/runtime qualification and benchmark smoke tests. Its 72-hour soak and seven-day longevity runs remain deferred upstream; these are deployment limits, not pg-mdm admission evidence.

## Archived v0.1 Linux package checksums

| Packaged file | SHA-256 |
|---|---|
| `usr/lib/postgresql/18/lib/pg_mdm.so` | `3d341b86ecc616cba5810f43ae0fd1797179a721e6574edc99c8d1d1ec7a6704` |
| `usr/share/postgresql/18/extension/pg_mdm.control` | `4f909208703ce8242fb06c4c1f7958f15bc432ae4faca0a40e35d572872f7946` |
| `usr/share/postgresql/18/extension/pg_mdm--0.1.0.sql` | `d9902b5160793bc5eb0f8a2924b905d0aab525ff796a3f0455a5330157d6d548` |
