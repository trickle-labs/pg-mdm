#!/usr/bin/env bash
set -euo pipefail

image=${PG_MDM_E2E_IMAGE:-pg_mdm:0.10.0-e2e}
container="pg-mdm-e2e-$$"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/pg-mdm-e2e.XXXXXX")
dump_file="$work_dir/foundation.dump"
missing_log="$work_dir/missing.log"

cleanup() {
    docker rm -fv "$container" >/dev/null 2>&1 || true
    rm -rf "$work_dir"
}
trap cleanup EXIT

"$(dirname "$0")/build_e2e_image.sh"
docker run --detach --name "$container" -e POSTGRES_PASSWORD=postgres "$image" >/dev/null

for _ in $(seq 1 60); do
    if docker exec "$container" pg_isready -U postgres >/dev/null 2>&1; then
        break
    fi
    sleep 1
done
docker exec "$container" pg_isready -U postgres >/dev/null

docker exec "$container" createdb -U postgres missing_dependency
if docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d missing_dependency \
    -c 'CREATE EXTENSION pg_mdm' >"$missing_log" 2>&1; then
    echo 'FAIL: installation without pg_trickle succeeded' >&2
    exit 1
fi
grep -q 'required extension "pg_trickle" is not installed' "$missing_log"

docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -f /tests/e2e.sql

docker exec -e PGAPPNAME=mdm_writer_one "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; BEGIN;
        DO \$\$ DECLARE result record; BEGIN
            SELECT * INTO STRICT result FROM mdm.create(jsonb_set(mdm.describe('customer', 'definition'), '{limits,max_candidate_pairs}', '200'), 3);
            IF NOT result.changed OR result.desired_version <> 4 THEN RAISE EXCEPTION 'concurrent update failed'; END IF;
        END \$\$;
        SELECT pg_sleep(10); COMMIT" >"$work_dir/writer_one.log" 2>&1 &
writer_one=$!
for _ in $(seq 1 60); do
    if [[ $(docker exec "$container" psql -X -At -U postgres -d foundation \
        -c "SELECT EXISTS (SELECT FROM pg_stat_activity WHERE application_name = 'mdm_writer_one' AND wait_event = 'PgSleep')") == t ]]; then
        break
    fi
    sleep 0.1
done
docker exec -e PGAPPNAME=mdm_writer_two "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; SET statement_timeout = '20s';
        SELECT * FROM mdm.create(jsonb_set(mdm.describe('customer', 'definition'), '{limits,max_candidate_pairs}', '300'), 3)" \
        >"$work_dir/writer_two.log" 2>&1 &
writer_two=$!
overlapped=false
for _ in $(seq 1 60); do
    if [[ $(docker exec "$container" psql -X -At -U postgres -d foundation \
        -c "SELECT EXISTS (SELECT FROM pg_stat_activity a, pg_stat_activity b WHERE a.application_name = 'mdm_writer_one' AND b.application_name = 'mdm_writer_two' AND a.pid = ANY(pg_blocking_pids(b.pid)))") == t ]]; then
        overlapped=true
        break
    fi
    sleep 0.1
done
if ! wait "$writer_one"; then cat "$work_dir/writer_one.log"; exit 1; fi
if wait "$writer_two"; then echo 'FAIL: concurrent stale version succeeded' >&2; exit 1; fi
if [[ $overlapped != true ]]; then echo 'FAIL: writers did not contend on the entity lock' >&2; exit 1; fi
if ! grep -q 'MDM_VERSION_CONFLICT' "$work_dir/writer_two.log"; then cat "$work_dir/writer_two.log"; exit 1; fi

original_source_oid=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "SELECT 'public.crm_customer'::regclass::oid")
original_role_oid=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "SELECT 'mdm_administrator'::regrole::oid")
artifact_query="SELECT md5(string_agg(encode(artifact_bytes, 'hex'), ',' ORDER BY definition_version)) FROM mdm_internal.definition_artifacts"
original_artifacts=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "$artifact_query")
docker exec "$container" pg_dump -Fc -U postgres foundation >"$dump_file"
before_revision=$(docker exec "$container" psql -X -At -U postgres -d foundation \
    -c "SELECT publication_revision FROM mdm_internal.entities WHERE entity_name = 'customer'")
before_output_rows=$(docker exec "$container" psql -X -At -U postgres -d foundation \
    -c "SELECT count(*) FROM mdm_out.customer")
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation <<'SQL'
SET ROLE mdm_administrator;
DO $$
DECLARE
    failed boolean := false;
    create_result record;
    result jsonb;
BEGIN
    SELECT * INTO STRICT create_result
    FROM mdm.create(jsonb_set(mdm.describe('customer', 'definition'),
        '{limits,max_active_records}', '2'), 4);
    BEGIN
        PERFORM mdm.refresh('customer', 'ALLOW');
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_RESOLVER_LIMIT') = 0 THEN
            RAISE;
        END IF;
        failed := true;
    END;
    IF NOT failed THEN
        RAISE EXCEPTION 'resolver limit did not fail closed';
    END IF;

    SELECT * INTO STRICT create_result
    FROM mdm.create(jsonb_set(mdm.describe('customer', 'definition'),
        '{limits,max_active_records}', '100'), 5);
    IF create_result.desired_version <> 6 THEN
        RAISE EXCEPTION 'resolver-limit retry did not create version 6';
    END IF;
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'false'
       OR (result->>'publication_revision')::bigint <> 1 THEN
        RAISE EXCEPTION 'resolver-limit retry changed the publication: %', result;
    END IF;
END
$$;
RESET ROLE;
SQL
after_revision=$(docker exec "$container" psql -X -At -U postgres -d foundation \
    -c "SELECT publication_revision FROM mdm_internal.entities WHERE entity_name = 'customer'")
after_output_rows=$(docker exec "$container" psql -X -At -U postgres -d foundation \
    -c "SELECT count(*) FROM mdm_out.customer")
test "$after_revision" = "$before_revision"
test "$after_output_rows" = "$before_output_rows"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c 'REVOKE ALL ON SCHEMA pgtrickle FROM mdm_administrator CASCADE;
        REVOKE ALL ON FUNCTION pgtrickle.encode_row_id_v2(text, anyelement) FROM mdm_administrator CASCADE;
        SET ROLE mdm_helper_owner;
        REVOKE ALL ON SCHEMA pgtrickle FROM mdm_administrator CASCADE;
        RESET ROLE;
        DROP OWNED BY mdm_administrator;
        DROP ROLE mdm_administrator;
        CREATE ROLE mdm_administrator NOLOGIN NOSUPERUSER NOBYPASSRLS;
        GRANT mdm_administrator TO mdm_test_login WITH SET TRUE, INHERIT FALSE;
        GRANT USAGE ON SCHEMA mdm TO mdm_administrator' >/dev/null
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation <<'SQL'
SET ROLE mdm_administrator;
DO $$ BEGIN
    BEGIN
        PERFORM mdm.describe('customer', 'definition');
        RAISE EXCEPTION 'same-name role replacement was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_UNAUTHORIZED') = 0 THEN RAISE; END IF;
    END;
END $$;
SQL
docker exec "$container" createdb -U postgres restored
chmod 0644 "$dump_file"
docker cp "$dump_file" "$container:/tmp/foundation.dump" >/dev/null
docker exec "$container" sh -c \
    "pg_restore -l /tmp/foundation.dump | grep -v 'TABLE DATA pgtrickle ' > /tmp/foundation.list"
docker exec "$container" pg_restore -v -U postgres -d restored \
    --use-list=/tmp/foundation.list /tmp/foundation.dump >/dev/null
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d restored \
    -v helper_owner=mdm_helper_owner -f /sql/configure_helper.sql >/dev/null
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d restored \
    -v original_source_oid="$original_source_oid" -v original_role_oid="$original_role_oid" -f /tests/restore.sql
restored_artifacts=$(docker exec "$container" psql -X -At -U postgres -d restored -c "$artifact_query")
test "$original_artifacts" = "$restored_artifacts"

docker exec "$container" createdb -U postgres --template=restored pg_mdm_clone
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d pg_mdm_clone <<'SQL'
DO $$
DECLARE
    clone_entity_id uuid;
    clone_source_identity_id uuid;
BEGIN
    SELECT e.entity_id INTO STRICT clone_entity_id
    FROM mdm_internal.entities e
    WHERE e.entity_name = 'customer';
    SELECT s.source_identity_id INTO STRICT clone_source_identity_id
    FROM mdm_internal.source_identities s
    WHERE s.entity_id = clone_entity_id AND s.source_name = 'crm';
    INSERT INTO mdm_internal.source_records
        (entity_id, source_identity_id, source_record_key, active)
    VALUES (clone_entity_id, clone_source_identity_id, '\x01020307'::bytea, true);
END
$$;
SQL
clone_sources=$(docker exec "$container" psql -X -At -U postgres -d pg_mdm_clone \
    -c "SELECT count(*) FROM mdm_internal.source_records WHERE active")
restored_sources=$(docker exec "$container" psql -X -At -U postgres -d restored \
    -c "SELECT count(*) FROM mdm_internal.source_records WHERE active")
test "$clone_sources" = 4
test "$restored_sources" = 3

echo 'PASS: installation, Graph V1 admission, authorization, definition history, concurrency, and restore/rebind'
echo 'PASS: resolver-limit rollback and retry, backup/restore, and clone isolation'
echo 'SKIPPED: Delta V1 positive conformance, output_delta_consumer is not used by V1'
