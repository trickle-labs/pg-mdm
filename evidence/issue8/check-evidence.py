"""Recheck evidence identity and exact workload coverage for parent issue #8."""
import gzip
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys

ROOT = Path.cwd()
WORK = Path(__file__).resolve().parent


def sha(data):
    return hashlib.sha256(data).hexdigest()


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def read_json(path):
    data = path.read_bytes()
    return json.loads(gzip.decompress(data) if path.suffix == ".gz" else data)


candidate = "d0c512ecac3d9393b1c199a405ec538ecd8f1eec"
ci_source = "ff570616e8f86c7ebc3c81dfb21f8ea83b57b38e"
runtime_source = "8f362be3080cc643a41670badfef29fcbea9c119"
assert git("rev-parse", "HEAD") == candidate
assert git("rev-parse", candidate + "^{tree}") == git("rev-parse", ci_source + "^{tree}")
runtime_paths = ["src", "tests", "sql", "Cargo.toml", "Cargo.lock", "pg_mdm.control", ".cargo"]
assert not git("diff", "--name-only", runtime_source, candidate, "--", *runtime_paths)
assert sha((WORK / "contract.md").read_bytes()) == "f09a262000f5c2615fce31399ea3548720c56a0a5b2619bb5f379b885463f1f5"
assert sha((WORK / "source-snapshot.md").read_bytes()) == "2ee03b51093184affdcd41d4041e6f6298060573418ae20435431c4d1e46ef23"
manifests = {}
for issue in (10, 11, 12):
    directory = ROOT / "evidence" / f"issue{issue}"
    entries = []
    for line in (directory / "manifest.sha256").read_text().splitlines():
        expected, name = line.split(maxsplit=1)
        path = directory / name
        assert sha(path.read_bytes()) == expected, str(path)
        if path.suffix == ".gz":
            gzip.decompress(path.read_bytes())
        entries.append(name)
    manifests[str(issue)] = entries

envelope_path = WORK / "ci-pr/e2e-evidence/docker-e2e-envelope.json.gz"
envelope = read_json(envelope_path)
assert envelope["source_commit"] == ci_source
assert envelope["result"] == "passed"
assert envelope["working_tree_status"] == []
assert envelope["candidate_patch_sha256"] == sha(b"")
assert envelope["collector_only_patch"] is False
assert envelope["collector_sha256"] == sha((ROOT / "scripts/record_e2e_evidence.py").read_bytes())
inputs = envelope["inputs"]
for key, path in {
    "cargo_lock_sha256": "Cargo.lock", "e2e_sql_sha256": "tests/e2e.sql",
    "e2e_policy_sql_sha256": "tests/e2e_policy.sql", "source_sync_sql_sha256": "tests/source_sync.sql",
    "scoped_state_sql_sha256": "tests/scoped_state.sql",
    "incremental_qualification_sql_sha256": "tests/incremental_qualification.sql",
    "operating_envelope_sql_sha256": "tests/operating_envelope.sql", "restore_sql_sha256": "tests/restore.sql",
}.items():
    assert inputs[key] == sha((ROOT / path).read_bytes()), key
assert inputs["pg_trickle_source_commit"] == "d8bb8eb5d45c5c9982e173e88914037433a1ee05"
assert inputs["pg_trickle_binary_sha256"] == "edb7345ed03dca50066737d421b4643b26f870435a964fb20379c23a77983ff9"
qualification = envelope["incremental_qualification"]
assert qualification["equivalence_checks"] == 70
assert [(s["population"], s["sample"], s["measured"], s["delta_rows"])
        for s in qualification["samples"]] == [
    (n, i, i > 5, 2) for n in (128, 2048) for i in range(1, 36)]
stages = {"source_access", "source_capture", "source_scan", "source_sync", "graph_helper_sync",
          "graph_refresh", "evaluation_state_loading", "merged_state_construction",
          "digest_generation", "snapshot_copying", "publication", "end_to_end"}
assert all(stages <= set(s["stage_timings_ms"]) for s in qualification["samples"])


def percentile(values, fraction):
    values = sorted(values)
    position = (len(values) - 1) * fraction
    lower = int(position)
    return values[lower] + (values[min(lower + 1, len(values) - 1)] - values[lower]) * (position - lower)


summary = read_json(ROOT / "evidence/issue12/measurement-summary.json")
before = read_json(ROOT / "evidence/issue12/before-qualification.json.gz")
after = read_json(ROOT / "evidence/issue12/after-envelope.json.gz")["incremental_qualification"]
for row in summary["populations"]:
    n = row["population"]
    for side, observations in (("before", before), ("after", after)):
        samples = [s for s in observations["samples"] if s["population"] == n and s["measured"]]
        assert len(samples) == 30
        for label, fraction in (("p50", 0.5), ("p95", 0.95)):
            actual = percentile([s["elapsed_ms"] for s in samples], fraction)
            assert math.isclose(actual, row[side + "_elapsed"][label], abs_tol=0.00051)
        if side == "after":
            for stage, reported in row["stages"].items():
                for label, fraction in (("p50", 0.5), ("p95", 0.95)):
                    actual = percentile([s["stage_timings_ms"][stage] for s in samples], fraction)
                    assert math.isclose(actual, reported[label], abs_tol=1e-9)
assert [(w["workload"], w["population"], w["changed_records"], w["result"]["affected_records"],
         w["result"]["affected_components"], w["result"]["resolver_strategy"],
         w["result"]["resolver_fallback_reason"], w["result"]["delta_row_count"], w["result"]["delta_lag"])
        for w in qualification["additional_workloads"]] == [
    ("dense", 8192, 32, 32, 1, "affected", None, 560, 0),
    ("larger", 8192, 1, 1, 1, "affected", None, 2, 0),
    ("no_change", 2048, 0, 0, 0, "skipped", None, 0, 0)]
isolated = [t for t in envelope["prior_state_work"] if t["entity_name"] == "incremental_qualification"
            and t["strategy"] == "affected" and t["scope_records"] == 1
            and t["entity_wide_baseline"]["memberships"] in (128, 2048)]
assert [t["entity_wide_baseline"]["memberships"] for t in isolated] == [128] * 36 + [2048] * 35
assert all(t["decoded"] == {"closure_membership_ids": 1, "registry": 1, "memberships": 1,
    "aliases": 0, "splits": 0, "reviews": 0, "golden": 1, "facts": 0, "overrides": 0} for t in isolated)
sys.path.insert(0, str(ROOT / "scripts"))
from check_publication_digests import check_cases

cases = read_json(ROOT / "evidence/issue12/digest-cases.json.gz")
digests = check_cases(cases)
assert sorted(c["label"] for c in cases) == sorted(c["label"] for c in envelope["publication_digest_checks"])
flag_mismatches = []
for case in cases:
    has_open_review = any(r["status"] == "open" for r in case["state"]["reviews"])
    for entity in case["state"]["entities"]:
        if entity["has_review"] != has_open_review:
            flag_mismatches.append([case["label"], entity["name"], entity["member_count"], entity["has_review"], has_open_review])
assert flag_mismatches == [
    ["reactivation", "Second", 2, False, True],
    ["retired_reactivation", "Second", 2, False, True],
    ["retirement", "Second", 2, False, True]]
result = {"candidate": candidate, "git_tree": git("rev-parse", candidate + "^{tree}"),
    "ci_source": ci_source, "ci_envelope_sha256": sha(gzip.decompress(envelope_path.read_bytes())),
    "runtime_source": runtime_source, "retained_manifest_entries": manifests,
    "issue12_percentiles_recomputed": True,
    "qualification_samples": 70, "traced_isolated_calls": 71, "checked_canonical_digests": digests,
    "retained_review_flag_mismatches": flag_mismatches, "evidence_identity_checks": "passed",
    "parent_acceptance": "changes_needed"}
(WORK / "evidence-checks.json").write_text(json.dumps(result, indent=2) + "\n")
print("PASS: candidate binding, retained hashes, 70 samples, 71 scoped traces, and 11 canonical digests")
print("CONFIRMED: three retained publication review-flag mismatches; parent acceptance needs changes")
