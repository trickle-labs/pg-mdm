#!/usr/bin/env bash
set -euo pipefail

archive=${1:-sql/archive/pg_mdm--0.1.0.sql}

test -f "$archive"

count=$(rg -c 'SECURITY DEFINER' "$archive")
test "$count" -eq 1

rg -U -q 'SECURITY DEFINER[[:space:]]+SET search_path TO pg_catalog, mdm_internal, pg_temp' "$archive"
rg -q 'REVOKE ALL ON FUNCTION mdm_admin\.verify_installation\(\) FROM PUBLIC' "$archive"

if rg -n 'Spi::(run|run_with_args)\(&|client\.(select|update)\(&' src; then
    echo 'dynamic SQL passed to SPI' >&2
    exit 1
fi

if rg -n 'pgtrickle_changes|pgt_[a-z_]+|stream_table_contract|graph_contract|set_orchestration_mode|refresh_graph_strict' src sql; then
    echo 'private or gated pg_trickle API referenced' >&2
    exit 1
fi

echo 'security-definer and pg_trickle boundary checks passed'
