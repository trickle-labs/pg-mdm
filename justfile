set shell := ["bash", "-cu"]

pg_config := env_var_or_default("PG_CONFIG", "pg_config")
package_dir := "target/release/pg_mdm-pg18"

fmt:
    cargo fmt -- --check

lint:
    cargo clippy --all-targets --features pg18 -- -D warnings

test-unit:
    cargo test --lib --features pg18

test-pgrx:
    cargo pgrx test pg18 --features pg_test

package:
    cargo pgrx package --pg-config "{{pg_config}}"

package-manifest: package
    mkdir -p target/release
    python3 scripts/package_manifest.py "{{package_dir}}" > target/release/package-manifest.json

test-e2e:
    scripts/run_e2e_tests.sh

check-security:
    scripts/check_security_definer.sh

check-upgrades:
    python3 scripts/check_upgrade_paths.py

check-archive: package
    generated=$(find "{{package_dir}}" -name 'pg_mdm--0.2.0.sql' -type f -print -quit); test -n "$generated"; cmp "$generated" sql/archive/pg_mdm--0.2.0.sql
