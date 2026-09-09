# Dependency lock

This file records the immutable v0.1 build inputs. `Cargo.lock` pins the Rust dependency graph. Run `just package-manifest` to record the checksums of every file in a built `pg_mdm` package.

| Input | Locked value |
|---|---|
| Rust edition | 2024 |
| Rust toolchain | 1.98.0 |
| Build target | `x86_64-unknown-linux-gnu` |
| `Cargo.lock` SHA-256 | `178cf6ee22df2d1f6b7cda838c5856bf7e33e6530527ef03fa6ecbd0625a63ab` |
| `cargo-pgrx` and `pgrx` | 0.18.0 |
| Runtime PostgreSQL | 18.4, Debian Bookworm |
| Build/test `pg_config` | 18.6 (`18.6-1.pgdg12+2`) |
| PostgreSQL image | `postgres:18.4-bookworm@sha256:efef99e1558f86089bc84bece29208c0777a185ff717ec7fa288a652ce2d0adf` |
| `pg_trickle` version | 0.98.0 |
| `pg_trickle` commit | `737927d336aabfff3b69bc2b0c29917b556c3626` |
| `pg_trickle` tag object | `168faa71074a81ba4315bf3a162dc5c32cc914dd` |
| `pg_trickle` artifact | `pg_trickle-0.98.0-pg18-linux-amd64.tar.gz` |
| Artifact URL | `https://github.com/trickle-labs/pg-trickle/releases/download/v0.98.0/pg_trickle-0.98.0-pg18-linux-amd64.tar.gz` |
| Artifact SHA-256 | `6b8a9cd3bb4761ede6150c29ba01f27eecbc2e3cffd5a85096c5683809c17fa7` |
| Capture mode | trigger |

The baseline capability response contains `external_graph_refresh 1.0 enabled=false` and `output_delta_consumer 1.0 enabled=false`. These disabled rows permit installation. `mdm_internal.require_graph_v1()` rejects graph work with `MDM_PGT_CAPABILITY_DISABLED`.

## Linux package checksums

| Packaged file | SHA-256 |
|---|---|
| `usr/lib/postgresql/18/lib/pg_mdm.so` | `3d341b86ecc616cba5810f43ae0fd1797179a721e6574edc99c8d1d1ec7a6704` |
| `usr/share/postgresql/18/extension/pg_mdm.control` | `4f909208703ce8242fb06c4c1f7958f15bc432ae4faca0a40e35d572872f7946` |
| `usr/share/postgresql/18/extension/pg_mdm--0.1.0.sql` | `d9902b5160793bc5eb0f8a2924b905d0aab525ff796a3f0455a5330157d6d548` |
