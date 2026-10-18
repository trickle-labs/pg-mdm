"""Extract actual prior-state SPI row reads from qualification auto_explain logs."""
import gzip
import json
import re
import sys
from pathlib import Path

mode, server_log, qualification_log, output = sys.argv[1:]
pids = {int(pid): int(pop) for pop, pid in re.findall(
    r"spi trace population=(128|2048) pid=(\d+)", Path(qualification_log).read_text())}
assert set(pids.values()) == {128, 2048}, pids
prefixes = {
    "registry": "SELECT mdm_id, created_revision, retired_revision, status FROM",
    "memberships": "SELECT m.source_record_id, r.source_record_key, m.mdm_id,",
    "aliases": "SELECT alias_mdm_id, canonical_mdm_id, publication_revision FROM",
    "splits": "SELECT parent_mdm_id, child_mdm_id, publication_revision FROM",
    "reviews": "SELECT review_id, issue_key, occurrence, status, severity,",
    "golden": "SELECT mdm_id, field_name::text, value, normalized_value, status,",
    "facts": "SELECT pg_catalog.convert_from(subject_key, 'UTF8'), fact_kind,",
    "overrides": "SELECT field_name::text, override_id, anchor_source_record_id,",
    "closure_membership_ids": "SELECT source_record_id FROM mdm_internal.memberships WHERE",
}
header = re.compile(r"^\d{4}-.*\[(\d+)\] LOG:  duration: ([\d.]+) ms  plan:")
records = []
current = None
block = []


def retain():
    if not current or int(current[1]) not in pids:
        return
    text = "".join(block)
    for category, prefix in prefixes.items():
        if "Query Text: " + prefix not in text:
            continue
        rows = re.search(r"actual rows=([\d.]+) loops=([\d.]+)", text)
        assert rows, text
        records.append({"population": pids[int(current[1])], "pid": int(current[1]),
                        "category": category, "duration_ms": float(current[2]),
                        "returned_rows": float(rows[1]) * float(rows[2]), "plan": text})
        break


with Path(server_log).open() as stream:
    for line in stream:
        if re.match(r"^\d{4}-", line):
            retain()
            current = header.match(line)
            block = [line] if current else []
        elif current:
            block.append(line)
retain()
summary = {str(pop): {category: sorted({r["returned_rows"] for r in records
    if r["population"] == pop and r["category"] == category})
    for category in prefixes} for pop in (128, 2048)}
expected = {"before": {"128": [0.0, 128.0], "2048": [128.0, 2048.0]},
            "after": {"128": [0.0, 1.0], "2048": [0.0, 1.0]}}[mode]
for pop in ("128", "2048"):
    for category in ("registry", "memberships", "golden"):
        assert summary[pop][category] == expected[pop], (pop, category, summary)
with gzip.open(output, "wt", encoding="utf-8") as stream:
    json.dump(records, stream, indent=2)
print(json.dumps(summary, indent=2))
