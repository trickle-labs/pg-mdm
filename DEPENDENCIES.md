# Dependency lock

This file records the current locked build inputs. `Cargo.lock` pins the Rust dependency graph. Run `just package-manifest` to record the checksums of every file in a built `pg_mdm` package.

| Input | Locked value |
|---|---|
| Rust edition | 2024 |
| Rust toolchain | 1.98.0 |
| Build target | `x86_64-unknown-linux-gnu` |
| `Cargo.lock` SHA-256 | `cb89eaf21de4d2cc3087fa561635d04198b28f45bc7df7113b83d475ea93c631` |
| `cargo-pgrx` and `pgrx` | 0.18.0 |
| Runtime PostgreSQL | 18.4, Debian Bookworm |
| Build/test `pg_config` | 18.6 (`18.6-1.pgdg12+2`) |
| PostgreSQL image | `postgres:18.4-bookworm@sha256:efef99e1558f86089bc84bece29208c0777a185ff717ec7fa288a652ce2d0adf` |
| `pg_trickle` version | 0.104.0 |
| `pg_trickle` commit | `c9eee742c2ac96eb12023bdfd3898aeedaf9b6b6` |
| `pg_trickle` tag object | `bb2601cd85e1b5223facb9a5a3566d708dc17f45` |
| `pg_trickle` artifact | `pg_trickle-0.104.0-pg18-linux-amd64.tar.gz` |
| Artifact URL | `https://github.com/trickle-labs/pg-trickle/releases/download/v0.104.0/pg_trickle-0.104.0-pg18-linux-amd64.tar.gz` |
| Artifact SHA-256 | `ee23aaa3c646ac6d4982a7bad78cb5628e4b6e07ba2ebbda259149198f427c3d` |
| Capture mode | trigger |

The baseline capability response contains `external_graph_refresh 1.0 enabled=true` and `output_delta_consumer 1.0 enabled=true`, both with stable status. V1 uses Graph V1 and deliberately leaves Delta V1 unused.

## Archived v0.1 Linux package checksums

| Packaged file | SHA-256 |
|---|---|
| `usr/lib/postgresql/18/lib/pg_mdm.so` | `3d341b86ecc616cba5810f43ae0fd1797179a721e6574edc99c8d1d1ec7a6704` |
| `usr/share/postgresql/18/extension/pg_mdm.control` | `4f909208703ce8242fb06c4c1f7958f15bc432ae4faca0a40e35d572872f7946` |
| `usr/share/postgresql/18/extension/pg_mdm--0.1.0.sql` | `d9902b5160793bc5eb0f8a2924b905d0aab525ff796a3f0455a5330157d6d548` |
