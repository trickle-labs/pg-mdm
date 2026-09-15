#!/usr/bin/env bash
set -euo pipefail

image=${PG_MDM_E2E_IMAGE:-pg_mdm:0.12.0-e2e}
pg_trickle_ref=79f38ab4e3e50d3f82e3aa78a5969b90a1bb9d27
pg_trickle_image="pg_trickle:e2e-${pg_trickle_ref:0:12}"
if [[ -n $(git status --porcelain) ]]; then
    echo 'E2E image builds require a clean source tree so the image label identifies the tested code.' >&2
    exit 1
fi
source_revision=$(git rev-parse HEAD)
if ! docker image inspect "$pg_trickle_image" >/dev/null 2>&1; then
    docker build --platform linux/amd64 \
        --build-arg BUILDER_IMAGE=ghcr.io/trickle-labs/pg-trickle/builder:pg18@sha256:7fe7daaab319b2f2749f5bc1bdae2387d13239b7d6a911648d5cdd177bde70b3 \
        -f tests/Dockerfile.e2e -t "$pg_trickle_image" \
        "https://github.com/trickle-labs/pg-trickle.git#$pg_trickle_ref"
    if [[ ${CI:-} == true ]]; then
        docker builder prune --force
    fi
fi
set --
if [[ -n ${PG_MDM_E2E_TARGET:-} ]]; then
    set -- --target "$PG_MDM_E2E_TARGET"
fi
docker build --platform linux/amd64 \
    --build-arg "PG_MDM_SOURCE_REVISION=$source_revision" \
    --build-arg "PG_TRICKLE_IMAGE=$pg_trickle_image" \
    "$@" -f tests/Dockerfile.e2e -t "$image" .
if [[ ${CI:-} == true ]]; then
    docker builder prune --force
fi
