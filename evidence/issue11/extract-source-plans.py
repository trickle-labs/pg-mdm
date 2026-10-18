import gzip
import json
import re
import sys
from pathlib import Path

header = re.compile(r"^\d{4}-.*\[(\d+)\] LOG:  duration: ([\d.]+) ms  plan:")
records = []
block = []
current = None

def retain():
    if not current or not block:
        return
    text = "".join(block)
    if "Query Text: SELECT source_record_key FROM (" in text:
        category = "source_scan"
    elif "Query Text: WITH current_records AS MATERIALIZED" in text:
        category = "source_sync"
    elif any("Query Text: " + sql in text for sql in (
        "INSERT INTO mdm_graph.source_records ", "DELETE FROM mdm_graph.source_records ")):
        category = "graph_helper_sync"
    else:
        return
    rows = re.search(r"actual rows=([\d.]+) loops=([\d.]+)", text)
    records.append(dict(pid=int(current[1]), duration_ms=float(current[2]), category=category,
                        returned_rows=float(rows[1]) * float(rows[2]) if rows else None,
                        plan=text))

with (sys.stdin if sys.argv[1] == "-" else Path(sys.argv[1]).open()) as stream:
    for line in stream:
        if re.match(r"^\d{4}-", line):
            retain()
            current = header.match(line)
            block = [line] if current else []
        elif current:
            block.append(line)
retain()
pids = {r["pid"] for r in records if r["category"] == "source_scan"
        and "FROM public.incremental_qualification_source)" in r["plan"]}
selected = [r for r in records if r["pid"] in pids]
assert selected and pids
with gzip.open(sys.argv[2], "wt", encoding="utf-8") as stream:
    json.dump(selected, stream, indent=2)
print(json.dumps([dict(pid=r["pid"], stage=r["category"], duration_ms=r["duration_ms"],
                       returned_rows=r["returned_rows"]) for r in selected], indent=2))
