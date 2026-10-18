"""Retain the measured Docker E2E envelope and its exact build inputs."""

import datetime as dt
import hashlib
import json
import os
import platform
import subprocess
import sys
from pathlib import Path

from check_publication_digests import check_cases


ROOT = Path(__file__).resolve().parents[1]
if len(sys.argv) != 7:
    raise SystemExit(
        "usage: record_e2e_evidence.py CONTAINER IMAGE SOURCE_REVISION "
        "DB_JSON QUALIFICATION_JSON LOG"
    )

container, image, source_revision, database_json, qualification_json, log_path = sys.argv[1:]

source_work = []
prior_state_work = []
with Path(log_path).open(encoding="utf-8") as e2e_stream:
    for line in e2e_stream:
        prior_marker = "PG_MDM_PRIOR_STATE_TRACE "
        if prior_marker in line:
            prior_state_work.append(json.loads(line.split(prior_marker, 1)[1]))
        marker = "PG_MDM_SOURCE_SYNC_TRACE "
        if marker not in line:
            continue
        trace = json.loads(line.split(marker, 1)[1])
        source_work.append({
            "entity_name": trace["entity_name"],
            "stage_timings_ms": trace["stage_timings_ms"],
            "capture": trace["capture"],
            "sources": [{"source_name": s["source_name"], "complete_scan": s["complete_scan"],
                         "present_keys": len(s["keys"]), "deleted_keys": len(s["deleted_keys"])}
                        for s in trace["sources"]],
        })

# Fixed lifecycle ordering includes rolled-back attempts, whose notices remain observable.
fixture_work = [
    [(s["complete_scan"], s["present_keys"], s["deleted_keys"]) for s in trace["sources"]]
    for trace in source_work if trace["entity_name"] == "source_sync"
]
if fixture_work != [
    [(True, 2, 0), (True, 1, 0)],       # bootstrap
    [(False, 0, 0), (False, 0, 0)],     # no change
    [(False, 1, 1), (False, 0, 0)],     # old/new composite key
    [(False, 1, 1), (False, 0, 1)],     # insert, soft delete and delete
    [(False, 1, 0), (False, 1, 0)],     # reactivation
    [(False, 0, 0), (False, 1, 0)],     # entity-wide overflow, rolled back
    [(False, 0, 0), (False, 0, 0)],     # unregistered insert/delete cancel
    [(False, 1, 0), (False, 0, 0)],     # publication failure, rolled back
    [(False, 1, 0), (False, 0, 0)],     # retry
    [(True, 3, 0), (True, 1, 0)],       # explicit complete-scan rebuild
    [(True, 3, 0), (False, 0, 0)],      # resnapshot, sibling still pending
    [(False, 0, 0), (False, 0, 0)],
    [(True, 3, 0), (False, 0, 0)],      # gap recovery
    [(True, 3, 0), (False, 0, 0)],      # full invalidation
]:
    raise SystemExit(f"source-key work differs from the lifecycle fixture: {fixture_work}")


def run(args):
    return subprocess.run(args, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=True).stdout.strip()


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def sha256_file(path):
    return sha256_bytes(Path(path).read_bytes())


digest_cases = json.loads(run([
    "docker", "exec", container, "psql", "-XAt", "-U", "postgres", "-d", "foundation", "-c",
    "SELECT jsonb_agg(to_jsonb(c) ORDER BY label) FROM public.scoped_state_cases c",
]))
if [case["label"] for case in digest_cases] != [
    "bootstrap", "conflict_recurs", "control_only", "inactive_anchor", "inactive_control_only",
    "merge_conflict", "reactivation", "remote_override_dependency", "retired_reactivation", "retirement", "split_resolves",
]:
    raise SystemExit("scoped-state history scenarios are incomplete")
digest_checks = check_cases(digest_cases)
states = {case["label"]: case["state"] for case in digest_cases}
initial = states["bootstrap"]
first_memberships = sorted((m["source_record_id"], m["first_membership_revision"])
                           for m in initial["memberships"])
for case in digest_cases:
    actual = sorted((m["source_record_id"], m["first_membership_revision"])
                    for m in case["state"]["memberships"])
    if actual != first_memberships:
        raise SystemExit(f"first membership history changed: {case['label']}")
conflicts = sorted([r["occurrence"], r["status"], r["opened_revision"], r["resolved_revision"],
                    r["last_change_revision"], r["concurrency_version"]]
                   for r in states["retired_reactivation"]["reviews"]
                   if r["reason_code"] == "GOLDEN_OVERRIDE_CONFLICT")
expected_conflicts = [[occurrence, "resolved", states[opened]["publication_revision"],
                       states[closed]["publication_revision"], states[closed]["publication_revision"], 2]
                      for occurrence, opened, closed in [(1, "merge_conflict", "split_resolves"),
                                                         (2, "conflict_recurs", "inactive_anchor")]]
if conflicts != expected_conflicts:
    raise SystemExit("review occurrence/revision/version history changed")
old_id = next(m["mdm_id"] for m in initial["memberships"]
              if initial["source_ids"][m["source_id"]["source_record_key"]] == 5)
old_identity = next(r for r in initial["identity_registry"] if r["mdm_id"] == old_id)
expected_tombstone = dict(old_identity, status="retired",
                          retired_revision=states["retirement"]["publication_revision"])
actual_tombstone = next(r for r in states["retired_reactivation"]["identity_registry"] if r["mdm_id"] == old_id)
if actual_tombstone != expected_tombstone:
    raise SystemExit("reactivation changed the retained identity tombstone")
isolated_state_work = [trace for trace in prior_state_work
    if trace["entity_name"] == "incremental_qualification" and trace["strategy"] == "affected"
    and trace["scope_records"] == 1
    and trace["entity_wide_baseline"]["memberships"] in (128, 2048)]
for trace in isolated_state_work:
    expected = {"closure_membership_ids": 1, "registry": 1, "memberships": 1, "aliases": 0, "splits": 0,
                "reviews": 0, "golden": 1, "facts": 0, "overrides": 0}
    if trace["decoded"] != expected:
        raise SystemExit(f"isolated refresh decoded unexpected prior rows: {trace}")
# The 2048-record diagnostic session has tracing disabled; its 35 samples are traced.
if [trace["entity_wide_baseline"]["memberships"] for trace in isolated_state_work] != [128] * 36 + [2048] * 35:
    raise SystemExit("scoped-state work observations are incomplete")


if run(["git", "rev-parse", "HEAD"]) != source_revision:
    raise SystemExit("E2E evidence source revision changed during the run")
working_tree_status = run(["git", "status", "--porcelain", "--untracked-files=all"])
if any(line.startswith("??") for line in working_tree_status.splitlines()):
    raise SystemExit("untracked source files cannot be included in the E2E candidate identity")
candidate_patch = subprocess.run(
    ["git", "diff", "HEAD", "--binary"], cwd=ROOT,
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True,
).stdout
candidate_patch_sha256 = sha256_bytes(candidate_patch)
candidate_key = os.environ.get("PG_MDM_CANDIDATE_KEY", "unknown")
collector_only_patch = bool(working_tree_status) and run(
    ["git", "diff", "HEAD", "--name-only"]
) == "scripts/record_e2e_evidence.py"
if candidate_key == "unknown" and working_tree_status and not collector_only_patch:
    raise SystemExit("dirty E2E candidates require an explicit patch hash")
if candidate_key != "unknown" and candidate_key != candidate_patch_sha256:
    raise SystemExit("E2E candidate key does not match the current source patch")

metrics = json.loads(Path(database_json).read_text(encoding="utf-8").strip())
qualification = json.loads(Path(qualification_json).read_text(encoding="utf-8").strip())
required_metrics = {
    "source_rows", "block_memberships", "max_block_records",
    "repeated_pair_discoveries", "unique_candidate_pairs",
    "evidence_comparisons", "component_checks", "temporary_bytes",
    "outputs", "output_bytes", "retained_history_rows_customer",
    "retained_history_table_bytes_database_wide", "last_customer_refresh",
}
missing = sorted(required_metrics - metrics.keys())
if missing or not metrics["last_customer_refresh"].get("stage_timings_ms"):
    raise SystemExit(f"E2E metrics are incomplete: {missing or 'stage timings'}")

if set(qualification) != {
    "schema_version", "pg_trickle_version", "equivalence_checks", "populations", "scope",
    "additional_workloads", "samples",
}:
    raise SystemExit("incremental qualification fields are incomplete")
if qualification["schema_version"] != 1 or qualification["pg_trickle_version"] != "0.108.3":
    raise SystemExit("incremental qualification dependency contract changed")
if qualification["equivalence_checks"] != 70:
    raise SystemExit("incremental qualification did not complete all rebuild comparisons")
if [(sample["population"], sample["sample"], sample["measured"], sample["delta_rows"])
        for sample in qualification["samples"]] != [
            (population, sample, sample > 5, 2)
            for population in (128, 2048) for sample in range(1, 36)]:
    raise SystemExit("incremental qualification raw sample coverage changed")
if [(item["workload"], item["population"], item["changed_records"])
        for item in qualification["additional_workloads"]] != [
            ("dense", 8192, 32), ("larger", 8192, 1), ("no_change", 2048, 0)]:
    raise SystemExit("additional graph workload coverage changed")
if [item.get("population") for item in qualification["populations"]] != [128, 2048]:
    raise SystemExit("incremental qualification populations changed")
for item in qualification["populations"]:
    if set(item) != {
        "population", "measured_samples", "warmup_samples", "elapsed_ms",
        "graph_refresh_ms", "mdm_resolution_ms", "publication_ms", "delta_rows",
        "source_stage_timings_ms",
    }:
        raise SystemExit("incremental qualification population fields are incomplete")
    if item["measured_samples"] != 30 or item["warmup_samples"] != 5:
        raise SystemExit("incremental qualification sample coverage changed")
    if any(value < 0 for value in item["elapsed_ms"].values()):
        raise SystemExit("incremental qualification elapsed timing is invalid")
    if set(item["source_stage_timings_ms"]) != {
        "source_access", "source_capture", "source_scan", "source_sync", "graph_helper_sync", "end_to_end",
    }:
        raise SystemExit("source synchronization stage observations are incomplete")

container_info = json.loads(run(["docker", "inspect", container]))[0]
image_info = json.loads(run(["docker", "image", "inspect", image]))[0]
labels = container_info["Config"].get("Labels") or {}
if labels.get("pg_mdm.source_revision") != source_revision:
    raise SystemExit("running image label does not match the tested source revision")
image_labels = image_info["Config"].get("Labels") or {}
if image_labels.get("pg_mdm.candidate_key") != candidate_key:
    raise SystemExit("running image label does not match the tested source patch")


def in_container(command):
    return run(["docker", "exec", container, "sh", "-c", command])


memory_info = {
    "visible_total_bytes": int(in_container("awk '/^MemTotal:/ {printf \"%.0f\\n\", $2 * 1024}' /proc/meminfo")),
    "visible_available_bytes": int(in_container("awk '/^MemAvailable:/ {printf \"%.0f\\n\", $2 * 1024}' /proc/meminfo")),
    "cgroup_limit_bytes": in_container("cat /sys/fs/cgroup/memory.max"),
    "cgroup_peak_bytes": int(in_container("cat /sys/fs/cgroup/memory.peak")),
}
cpu_info = {
    "visible_processors": int(in_container("nproc")),
    "model": in_container("awk -F: '/model name/ {sub(/^[ \\t]+/, \"\", $2); print $2; exit}' /proc/cpuinfo"),
}
storage_info = in_container('df -B1 "$PGDATA"')
package_manifest = in_container("cat /evidence/pg_mdm-package-manifest.sha256") + "\n"
producer = dict(line.removeprefix("ARG ").split("=", 1)
                for line in (ROOT / "tests/Dockerfile.e2e").read_text().splitlines()
                if line.startswith("ARG PG_TRICKLE_"))
if set(producer) != {"PG_TRICKLE_REVISION", "PG_TRICKLE_URL", "PG_TRICKLE_SHA256"} \
        or len(bytes.fromhex(producer["PG_TRICKLE_REVISION"])) != 20 \
        or len(bytes.fromhex(producer["PG_TRICKLE_SHA256"])) != 32:
    raise SystemExit("pg_trickle immutable source pin is incomplete")
producer_binary_sha256 = in_container(
    "sha256sum /usr/lib/postgresql/18/lib/pg_trickle.so").split()[0]
evidence_dir = Path(os.environ.get("PG_MDM_E2E_EVIDENCE_DIR", ROOT / "evidence" / "v0.14"))
evidence_dir.mkdir(parents=True, exist_ok=True)
candidate_patch_path = evidence_dir / "candidate.patch"
candidate_patch_path.write_bytes(candidate_patch)
manifest_path = evidence_dir / "pg-mdm-package-manifest.sha256"
manifest_path.write_text(package_manifest, encoding="utf-8")
retained_log = evidence_dir / "docker-e2e.log"
retained_log.write_bytes(Path(log_path).read_bytes())
with retained_log.open("a", encoding="utf-8") as stream:
    stream.write("\nPASS: E2E runner reached evidence collection after all database assertions.\n")

report = {
    "evidence_version": 1,
    "recorded_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
    "source_commit": source_revision,
    "candidate_key": candidate_key,
    "candidate_patch": str(candidate_patch_path),
    "candidate_patch_sha256": candidate_patch_sha256,
    "collector_only_patch": collector_only_patch,
    "collector_sha256": sha256_file(Path(__file__)),
    "working_tree_status": working_tree_status.splitlines(),
    "command": "scripts/run_e2e_tests.sh",
    "result": "passed",
    "retained_log": str(retained_log),
    "retained_log_scope": "Main PostgreSQL script output plus policy qualification output and a pass marker written after the complete E2E runner, including concurrency, physical and logical recovery, restore, clone, and retry assertions, returned successfully.",
    "metrics": metrics,
    "incremental_qualification": qualification,
    "source_sync_work": source_work,
    "prior_state_work": prior_state_work,
    "publication_digest_checks": digest_checks,
    "inputs": {
        "organization_fixture_sha256": sha256_file(ROOT / "tests/fixtures/organization_domain_v1.json"),
        "e2e_sql_sha256": sha256_file(ROOT / "tests/e2e.sql"),
        "e2e_policy_sql_sha256": sha256_file(ROOT / "tests/e2e_policy.sql"),
        "source_sync_sql_sha256": sha256_file(ROOT / "tests/source_sync.sql"),
        "scoped_state_sql_sha256": sha256_file(ROOT / "tests/scoped_state.sql"),
        "restore_sql_sha256": sha256_file(ROOT / "tests/restore.sql"),
        "operating_envelope_sql_sha256": sha256_file(ROOT / "tests/operating_envelope.sql"),
        "incremental_qualification_sql_sha256": sha256_file(
            ROOT / "tests/incremental_qualification.sql"
        ),
        "cargo_lock_sha256": sha256_file(ROOT / "Cargo.lock"),
        "archived_pg_mdm_0_14_0_sha256": sha256_file(ROOT / "sql/archive/pg_mdm--0.14.0.sql"),
        "pg_mdm_package_manifest_sha256": sha256_bytes(package_manifest.encode()),
        "pg_trickle_version": "0.108.3",
        "pg_trickle_source_commit": producer["PG_TRICKLE_REVISION"],
        "pg_trickle_artifact_url": producer["PG_TRICKLE_URL"],
        "pg_trickle_artifact_sha256": producer["PG_TRICKLE_SHA256"],
        "pg_trickle_binary_sha256": producer_binary_sha256,
        "postgres_image": "postgres:18.4-bookworm@sha256:efef99e1558f86089bc84bece29208c0777a185ff717ec7fa288a652ce2d0adf",
        "rust_toolchain": "1.98.0",
    },
    "environment": {
        "docker_image_id": image_info["Id"],
        "docker_image_architecture": image_info["Architecture"],
        "docker_engine_architecture": platform.machine(),
        "postgres_build": metrics["postgres_build"],
        "cpu": cpu_info,
        "memory": memory_info,
        "storage": storage_info,
    },
    "limitations": [
        "This report measures the small synthetic customer workload in the E2E suite, not a production-scale capacity envelope.",
        "Incremental p95 values are observational measurements from this Docker host, not timing gates or production SLO evidence.",
        "temporary_bytes is cumulative for the foundation database over the complete E2E run.",
        "retained_history_table_bytes_database_wide includes whole-table sizes for MDM history relations; row counts are scoped to customer.",
    ],
}
(evidence_dir / "docker-e2e-envelope.json").write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
