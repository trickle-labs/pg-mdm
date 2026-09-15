"""Retain the measured Docker E2E envelope and its exact build inputs."""

import datetime as dt
import hashlib
import json
import platform
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
if len(sys.argv) != 6:
    raise SystemExit("usage: record_e2e_evidence.py CONTAINER IMAGE SOURCE_REVISION DB_JSON LOG")

container, image, source_revision, database_json, log_path = sys.argv[1:]


def run(args):
    return subprocess.run(args, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=True).stdout.strip()


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def sha256_file(path):
    return sha256_bytes(Path(path).read_bytes())


if run(["git", "rev-parse", "HEAD"]) != source_revision:
    raise SystemExit("E2E evidence source revision changed during the run")
if run(["git", "status", "--porcelain"]):
    raise SystemExit("commit all source changes before retaining Docker E2E evidence")

metrics = json.loads(Path(database_json).read_text(encoding="utf-8").strip())
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

container_info = json.loads(run(["docker", "inspect", container]))[0]
image_info = json.loads(run(["docker", "image", "inspect", image]))[0]
labels = container_info["Config"].get("Labels") or {}
if labels.get("pg_mdm.source_revision") != source_revision:
    raise SystemExit("running image label does not match the tested source revision")


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
evidence_dir = ROOT / "evidence" / "v0.12"
evidence_dir.mkdir(parents=True, exist_ok=True)
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
    "command": "scripts/run_e2e_tests.sh",
    "result": "passed",
    "retained_log": str(retained_log.relative_to(ROOT)),
    "retained_log_scope": "Main PostgreSQL script output plus a pass marker written after the complete E2E runner, including concurrency, physical and logical recovery, restore, clone, and retry assertions, returned successfully.",
    "metrics": metrics,
    "inputs": {
        "organization_fixture_sha256": sha256_file(ROOT / "tests/fixtures/organization_domain_v1.json"),
        "e2e_sql_sha256": sha256_file(ROOT / "tests/e2e.sql"),
        "operating_envelope_sql_sha256": sha256_file(ROOT / "tests/operating_envelope.sql"),
        "cargo_lock_sha256": sha256_file(ROOT / "Cargo.lock"),
        "archived_pg_mdm_0_13_0_sha256": sha256_file(ROOT / "sql/archive/pg_mdm--0.13.0.sql"),
        "pg_mdm_package_manifest_sha256": sha256_bytes(package_manifest.encode()),
        "pg_trickle_version": "0.106.1",
        "pg_trickle_artifact_sha256": "e6976e4e6477b5241008f5ec3b48edb395b600aea4fe78c1944e69e12046d2bf",
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
        "temporary_bytes is cumulative for the foundation database over the complete E2E run.",
        "retained_history_table_bytes_database_wide includes whole-table sizes for MDM history relations; row counts are scoped to customer.",
    ],
}
(evidence_dir / "docker-e2e-envelope.json").write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
