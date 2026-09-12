# Dependency lock

This file records the current locked build inputs. `Cargo.lock` pins the Rust dependency graph. Run `just package-manifest` to record the checksums of every file in a built `pg_mdm` package.

| Input | Locked value |
|---|---|
| Rust edition | 2024 |
| Rust toolchain | 1.98.0 |
| Build target | `x86_64-unknown-linux-gnu` |
| `Cargo.lock` SHA-256 | `3c2d7dc9c9ddb5c8e882d4b0fd106f83f57b6f3c8eb0c10b92bb7988ab7126eb` |
| `cargo-pgrx` and `pgrx` | 0.18.0 |
| Runtime PostgreSQL | 18.4, Debian Bookworm |
| Build/test `pg_config` | 18.6 (`18.6-1.pgdg12+2`) |
| PostgreSQL image | `postgres:18.4-bookworm@sha256:efef99e1558f86089bc84bece29208c0777a185ff717ec7fa288a652ce2d0adf` |
| `pg_trickle` version | 0.105.1 |
| `pg_trickle` commit | `5bfdea89bbcca2bf6d606ddbf034b62eddf3b2ae` |
| `pg_trickle` tag object | `fa6fbb0c17cdd3ac64f87b8d7611c828edaf5614` |
| `pg_trickle` artifact | `pg_trickle-0.105.1-pg18-linux-amd64.tar.gz` |
| Artifact URL | `https://github.com/trickle-labs/pg-trickle/releases/download/v0.105.1/pg_trickle-0.105.1-pg18-linux-amd64.tar.gz` |
| Artifact SHA-256 | `a6b1942ce5d2517dc8ad94a04ba3f2508fad4be21d6fc247df873502bcf2e69e` |
| Capture mode | trigger |

The baseline capability response contains `external_graph_refresh 1.0 enabled=true` and `output_delta_consumer 1.0 enabled=true`, both with stable status. V1 uses Graph V1 and deliberately leaves Delta V1 unused.

## Archived v0.1 Linux package checksums

| Packaged file | SHA-256 |
|---|---|
| `usr/lib/postgresql/18/lib/pg_mdm.so` | `3d341b86ecc616cba5810f43ae0fd1797179a721e6574edc99c8d1d1ec7a6704` |
| `usr/share/postgresql/18/extension/pg_mdm.control` | `4f909208703ce8242fb06c4c1f7958f15bc432ae4faca0a40e35d572872f7946` |
| `usr/share/postgresql/18/extension/pg_mdm--0.1.0.sql` | `d9902b5160793bc5eb0f8a2924b905d0aab525ff796a3f0455a5330157d6d548` |
