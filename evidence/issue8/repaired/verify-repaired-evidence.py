"""Check the repaired assembled candidate, regression and replacement work evidence."""
import gzip
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path.cwd()
WORK = Path(__file__).resolve().parent
CANDIDATE = "2523414d2c1e57cc8c0b73a8017de6f5941d21e7"


def read_bytes(name):
    path = WORK / name
    if not path.exists():
        path = WORK / (name + ".gz")
    data = path.read_bytes()
    return gzip.decompress(data) if path.suffix == ".gz" else data


def read_json(name):
    return json.loads(read_bytes(name))


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


paths = ["src", "tests", "sql", "contracts", "scripts", "Cargo.toml", "Cargo.lock", "pg_mdm.control", ".cargo"]
changed = subprocess.check_output(["git", "diff", "--name-only", CANDIDATE, "HEAD", "--", *paths], text=True).splitlines()
assert set(changed) <= {"sql/archive/pg_mdm--0.14.1.sql"}, changed
assert subprocess.check_output(["git", "diff", "HEAD", "--", *paths]) == b""
original_archive = subprocess.check_output(["git", "show", CANDIDATE + ":sql/archive/pg_mdm--0.14.1.sql"])
current_archive = (ROOT / "sql/archive/pg_mdm--0.14.1.sql").read_bytes()
assert current_archive == read_bytes("generated-pg_mdm--0.14.1.sql")
position = rb"(?m)^-- src/api/refresh.rs:\d+$"
assert re.sub(position, b"-- source-position", original_archive) == re.sub(position, b"-- source-position", current_archive)
envelope = read_json("final/evidence/docker-e2e-envelope.json")
assert envelope["source_commit"] == CANDIDATE and envelope["result"] == "passed"
assert envelope["working_tree_status"] == []
assert envelope["candidate_patch_sha256"] == hashlib.sha256(b"").hexdigest()
assert envelope["collector_only_patch"] is False
assert envelope["collector_sha256"] == sha(ROOT / "scripts/record_e2e_evidence.py")
assert envelope["inputs"]["scoped_state_sql_sha256"] == sha(ROOT / "tests/scoped_state.sql")
samples = envelope["incremental_qualification"]["samples"]
assert [(s["population"], s["sample"], s["measured"], s["delta_rows"]) for s in samples] == [
    (n, i, i > 5, 2) for n in (128, 2048) for i in range(1, 36)]
stages = {"source_access", "source_capture", "source_scan", "source_sync", "graph_helper_sync", "graph_refresh",
          "evaluation_state_loading", "merged_state_construction", "digest_generation", "snapshot_copying", "publication", "end_to_end"}
assert all(stages <= set(s["stage_timings_ms"]) for s in samples)
assert envelope["incremental_qualification"]["equivalence_checks"] == 70
assert read_json("sdk-status.json") == [{"check":name,"exit_code":0}
    for name in ("fmt", "clippy", "unit", "pgrx", "security", "upgrades", "schema")]
assert read_json("unfixed-status.json") == [["bootstrap",0],["new production regression on unfixed binary",3]]
assert "scoped-state reactivation public review flags differ" in read_bytes("unfixed-new.log").decode()
runner = read_bytes("final-run.log").decode()
for marker in ("same-boundary affected and full entity/member/review publications match",
               "complete review flags, unrelated components, first/last reviews and exact change revisions",
               "physical backup recovery with populated and pending graph state",
               "affected-resolution qualification matches full rebuilds at 128 and 2,048 records"):
    assert "PASS: " + marker in runner, marker
volumes = {}
for side in ("baseline", "candidate"):
    plans = read_json(side + "-publication-plans.json.gz")
    volumes[side] = {}
    for n in (128, 2048):
        volumes[side][str(n)] = {}
        for category in ("registry", "memberships", "golden", "entities", "members"):
            selected = [p for p in plans if p["population"] == n and p["category"] == category]
            assert [p["input_rows"] for p in selected] == ([1]*n if side == "baseline" else [128]*(n//128))
            volumes[side][str(n)][category] = {"calls":len(selected),"rows":sum(p["input_rows"] for p in selected)}
    assert volumes[side] == read_json(side + "-publication-summary.json")
identities = read_json("trace-runtime-identities.json")
assert identities["baseline"]["source_revision"] == "9a07830d062b31c108880071c27124e2f09c446a"
assert identities["candidate"]["source_revision"] == CANDIDATE
assert identities["baseline"]["binaries"][1] == identities["candidate"]["binaries"][1]
assert envelope["environment"]["docker_image_id"] == identities["candidate"]["image_id"]
assert identities["candidate"]["binaries"][1].split()[0] == envelope["inputs"]["pg_trickle_binary_sha256"]
cases = read_json("fresh-digest-cases.json")
assert [c["label"] for c in cases] == [c["label"] for c in envelope["publication_digest_checks"]]
sys.path.insert(0, str(ROOT / "scripts"))
from check_publication_digests import check_cases

digest_checks = check_cases(cases)
assert digest_checks == envelope["publication_digest_checks"]
for case in cases:
    expected_flag = any(r["status"] == "open" for r in case["state"]["reviews"])
    assert all(e["has_review"] == expected_flag for e in case["state"]["entities"]), case["label"]
unit_log = read_bytes("unit.log").decode()
assert sum(map(int, re.findall(r"test result: ok\. (\d+) passed", unit_log))) == 142
result = {"candidate":CANDIDATE,"evidence_verification":"PASS","acceptance_report":"README.md",
    "archive_matches_qualified_package":True,"archive_difference":"source-position comments only",
    "fresh_host_tests_passed":142,"qualification_comparisons":70,"canonical_digest_checks":digest_checks,
    "replacement_publication_work":volumes,"runtime_identities":identities}
(WORK / "repaired-evidence-checks.json").write_text(json.dumps(result,indent=2) + "\n")
print("PASS: repaired candidate binding, regression, SDK, 70 comparisons, 11 digests and complete paired SPI traces")
