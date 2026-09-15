#!/usr/bin/env bash
set -euo pipefail

image=${PG_MDM_E2E_IMAGE:-pg_mdm:0.13.0-e2e}
if [[ -n $(git status --porcelain) ]]; then
    echo 'E2E image builds require a clean source tree so the image label identifies the tested code.' >&2
    exit 1
fi
source_revision=$(git rev-parse HEAD)
set --
if [[ -n ${PG_MDM_E2E_TARGET:-} ]]; then
    set -- --target "$PG_MDM_E2E_TARGET"
fi
docker build --platform linux/amd64 \
    --build-arg "PG_MDM_SOURCE_REVISION=$source_revision" \
    "$@" -f tests/Dockerfile.e2e -t "$image" .
if [[ ${CI:-} == true ]]; then
    docker builder prune --force
fi
