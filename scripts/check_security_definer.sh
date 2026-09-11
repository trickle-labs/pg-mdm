#!/usr/bin/env bash
set -euo pipefail

archive=${1:-sql/archive/pg_mdm--0.10.0.sql}

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
    signature = f"{name}({', '.join(arg.split('DEFAULT', 1)[0].split()[-1] for arg in arguments.split(',') if arg.strip())})"
    assert re.search(r'SECURITY DEFINER\s+SET search_path TO pg_catalog, mdm_internal, pg_temp\s', body), signature
    assert f'REVOKE ALL ON FUNCTION {signature} FROM PUBLIC;' in sql, signature
    helpers[name] = signature
assert set(helpers) == {
    'mdm_admin.verify_installation', 'mdm_internal.persist_entity', 'mdm_internal.describe_entity',
    'mdm_internal.prepare_rebind', 'mdm_internal.persist_rebind', 'mdm_internal.persist_decision',
    'mdm_internal.explain_entity', 'mdm_internal.persist_golden_override',
    'mdm_internal.persist_drop_entity', 'mdm_internal.persist_refresh',
    'mdm_internal.preview_entity', 'mdm_internal.refresh_access',
    'mdm_graph.normalize_text',
    'mdm_graph.normalize_date', 'mdm_graph.normalized_levenshtein_score',
    'mdm_graph.evidence_digest'
}, helpers
PY

if rg -n 'Spi::(run|run_with_args)\(&|client\.(select|update)\(&' src; then
    echo 'dynamic SQL passed to SPI' >&2
    exit 1
fi

if rg -n 'pgtrickle_changes|pgt_[a-z_]+|set_orchestration_mode' src sql; then
    echo 'private or gated pg_trickle API referenced' >&2
    exit 1
fi

echo 'security-definer and pg_trickle boundary checks passed'
