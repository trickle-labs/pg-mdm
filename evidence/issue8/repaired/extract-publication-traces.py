"""Bind and check every main publication SPI call in the paired fixed fixtures."""
import gzip
import json
from pathlib import Path
import re
import sys

directory = Path(__file__).resolve().parent
side = sys.argv[1]
assert side in ("baseline", "candidate")
pids = {}
for population in (128, 2048):
    text = (directory / f"{side}-probe-{population}.log").read_text()
    markers = re.findall(r"PG_MDM_PUBLICATION_PROBE population=(\d+) pid=(\d+) result=(.*)", text)
    assert len(markers) == 1 and int(markers[0][0]) == population
    pids[int(markers[0][1])] = population
    assert "PASS: complete registry, memberships, golden provenance and public entity/member rows" in text
header = re.compile(r"^\d{4}-.*\[(\d+)\] LOG:  duration: ([\d.]+) ms  plan:")
plans = []
current = None
block = []


def retain():
    if current is None or int(current[1]) not in pids:
        return
    text = "".join(block)
    target = re.search(r"Query Text: INSERT INTO (mdm_internal|mdm_out)\.\"?(\w+)\"?", text)
    if target is None:
        return
    population = pids[int(current[1])]
    category = {"identity_registry": "registry", "memberships": "memberships", "golden_provenance": "golden",
                f"publication_probe_{population}": "entities", f"publication_probe_{population}_members": "members"}.get(target[2])
    if category is None:
        return
    child = re.search(r"->[^\n]*actual rows=([\d.]+) loops=([\d.]+)", text)
    assert child, text
    plans.append({"population": population, "category": category, "pid": int(current[1]),
        "duration_ms": float(current[2]), "input_rows": float(child[1]) * float(child[2]), "plan": text})


with (directory / f"{side}-server.log").open() as stream:
    for line in stream:
        if re.match(r"^\d{4}-", line):
            retain()
            current = header.match(line)
            block = [line] if current else []
        elif current:
            block.append(line)
retain()
summary = {}
for population in (128, 2048):
    summary[str(population)] = {}
    for category in ("registry", "memberships", "golden", "entities", "members"):
        selected = [p for p in plans if p["population"] == population and p["category"] == category]
        expected_rows = [1] * population if side == "baseline" else [128] * (population // 128)
        assert [p["input_rows"] for p in selected] == expected_rows, (population, category, [p["input_rows"] for p in selected])
        summary[str(population)][category] = {"calls": len(selected), "rows": sum(p["input_rows"] for p in selected)}
with gzip.open(directory / f"{side}-publication-plans.json.gz", "wt") as output:
    json.dump(plans, output, indent=2)
(directory / f"{side}-publication-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print("PASS:", side, json.dumps(summary))
