#!/usr/bin/env bash
set -euo pipefail

archive=${1:-sql/archive/pg_mdm--0.6.0.sql}

test -f "$archive"

python3 - "$archive" <<'PY'
from pathlib import Path
import re
import sys

sql = Path(sys.argv[1]).read_text()
helpers = {}
for name, arguments, body in re.findall(r'CREATE FUNCTION ([\w.]+)\((.*?)\)(.*?);', sql, re.S):
    if 'SECURITY DEFINER' not in body:
        continue
    signature = f"{name}({', '.join(arg.split()[-1] for arg in arguments.split(',') if arg.strip())})"
    assert re.search(r'SECURITY DEFINER\s+SET search_path TO pg_catalog, mdm_internal, pg_temp\s', body), signature
    assert f'REVOKE ALL ON FUNCTION {signature} FROM PUBLIC;' in sql, signature
    helpers[name] = signature
assert set(helpers) == {
    'mdm_admin.verify_installation', 'mdm_internal.persist_entity', 'mdm_internal.describe_entity',
    'mdm_internal.prepare_rebind', 'mdm_internal.persist_rebind', 'mdm_internal.persist_decision'
}, helpers
PY

if rg -n 'Spi::(run|run_with_args)\(&|client\.(select|update)\(&' src; then
    echo 'dynamic SQL passed to SPI' >&2
    exit 1
fi

if rg -n 'pgtrickle_changes|pgt_[a-z_]+|stream_table_contract|graph_contract|set_orchestration_mode|refresh_graph_strict' src sql; then
    echo 'private or gated pg_trickle API referenced' >&2
    exit 1
fi

echo 'security-definer and pg_trickle boundary checks passed'
