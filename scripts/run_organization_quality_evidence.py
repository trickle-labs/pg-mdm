#!/usr/bin/env python3
"""Run the frozen organization regression gate and retain its exact inputs."""

import datetime as dt
import hashlib
import json
import platform
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "evidence" / "v0.12"
COMMAND = [
    "cargo",
    "test",
    "--test",
    "organization_quality_tests",
    "--features",
    "pg18",
    "--",
    "--nocapture",
]


def run(args):
    return subprocess.run(args, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, check=True).stdout.strip()


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


OUT.mkdir(parents=True, exist_ok=True)
output = subprocess.run(COMMAND, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT)
print(output.stdout, end="")
if output.returncode:
    raise SystemExit(output.returncode)

marker = next((line.partition("=")[2] for line in output.stdout.splitlines()
               if line.startswith("ORGANIZATION_FIXTURE_REPORT=")), None)
if marker is None:
    raise SystemExit("quality test passed without its machine-readable report")
fixture_report = json.loads(marker)
fixture = ROOT / "tests" / "fixtures" / "organization_domain_v1.json"
fixture_digest = sha256(fixture)
if fixture_report["fixture_sha256"] != fixture_digest:
    raise SystemExit("quality report fixture digest differs from the input fixture")

dirty = run(["git", "status", "--porcelain"])
if dirty:
    raise SystemExit("commit source and fixture changes before retaining evidence")

log = OUT / "organization-quality.log"
log.write_text(f"$ {' '.join(COMMAND)}\n{output.stdout}", encoding="utf-8")
report = {
    "evidence_version": 1,
    "recorded_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
    "source_commit": run(["git", "rev-parse", "HEAD"]),
    "command": " ".join(COMMAND),
    "result": "passed",
    "retained_log": str(log.relative_to(ROOT)),
    "inputs": {
        "organization_fixture_sha256": fixture_digest,
        "cargo_lock_sha256": sha256(ROOT / "Cargo.lock"),
    },
    "release_context": {
        "pg_trickle_artifact_sha256": "bf8d8dcff728a5cf9e09458b70f2c3ce2c92cc109f4e7c79ae2933ac8bb03416",
        "postgres_image_digest": "sha256:efef99e1558f86089bc84bece29208c0777a185ff717ec7fa288a652ce2d0adf",
    },
    "environment": {
        "runner_os": platform.platform(),
        "runner_architecture": platform.machine(),
        "rustc": run(["rustc", "--version"]),
        "cargo": run(["cargo", "--version"]),
        "postgres_runtime": "not used; this is the pure production-semantics fixture runner",
    },
    "metrics": fixture_report,
    "interpretation": "Synthetic semantic regression evidence only; it is not a production accuracy claim.",
}
(OUT / "organization-quality.json").write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
