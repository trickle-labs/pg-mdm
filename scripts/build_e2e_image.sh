#!/usr/bin/env bash
set -euo pipefail

image=${PG_MDM_E2E_IMAGE:-pg_mdm:0.10.0-e2e}
docker build --platform linux/amd64 -f tests/Dockerfile.e2e -t "$image" .
