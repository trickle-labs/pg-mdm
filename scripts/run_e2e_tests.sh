#!/usr/bin/env bash
set -euo pipefail

image=${PG_MDM_E2E_IMAGE:-pg_mdm:0.12.0-e2e}
container="pg-mdm-e2e-$$"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/pg-mdm-e2e.XXXXXX")
dump_file="$work_dir/foundation.dump"
missing_log="$work_dir/missing.log"
e2e_log="$work_dir/e2e-postgres.log"
repo_root=$(cd "$(dirname "$0")/.." && pwd)
source_revision=$(git -C "$repo_root" rev-parse HEAD)

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

docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -f /tests/e2e.sql | tee "$e2e_log"

docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation <<'SQL'
INSERT INTO public.crm_customer VALUES (3, 'Before boundary', 'before@example.test', statement_timestamp());
CREATE FUNCTION public.delay_release_output()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    -- Give the outer test a deterministic marker for the publication boundary.
    IF pg_catalog.current_setting('mdm.e2e_release_waited', true) IS DISTINCT FROM 'on' THEN
        PERFORM pg_catalog.set_config('mdm.e2e_release_waited', 'on', true);
        PERFORM pg_catalog.pg_advisory_xact_lock(718110, 110972);
        PERFORM pg_catalog.pg_sleep(5);
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER delay_release_output
BEFORE INSERT OR UPDATE ON mdm_out.customer
FOR EACH ROW EXECUTE FUNCTION public.delay_release_output();
SQL
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation <<'SQL' >"$work_dir/boundary_refresh.log" 2>&1 &
SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'true'
       OR (result->>'publication_revision')::bigint <> 6
       OR result->'source_boundary'->>'completeness' <> 'PROVEN'
       OR length(result->>'source_boundary_digest') <> 64 THEN
        RAISE EXCEPTION 'concurrent refresh returned an invalid boundary: %', result;
    END IF;
END
$$;
SQL
boundary_refresh=$!
boundary_captured=false
for _ in $(seq 1 600); do
    if [[ $(docker exec "$container" psql -X -At -U postgres -d foundation \
        -c "SELECT EXISTS (SELECT FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND classid = 718110::oid AND objid = 110972::oid AND objsubid = 2 AND granted)") == t ]]; then
        boundary_captured=true
        break
    fi
    sleep 0.1
done
if [[ $boundary_captured != true ]]; then
    kill "$boundary_refresh" 2>/dev/null || true
    wait "$boundary_refresh" || true
    cat "$work_dir/boundary_refresh.log"
    echo 'FAIL: refresh did not reach publication after graph maintenance' >&2
    exit 1
fi
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "INSERT INTO public.crm_customer VALUES (4, 'After boundary', 'after@example.test', statement_timestamp())" >/dev/null
if ! wait "$boundary_refresh"; then cat "$work_dir/boundary_refresh.log"; exit 1; fi
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation <<'SQL'
DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_out.customer c
        JOIN mdm_out.customer_members m USING (mdm_id)
        WHERE c.name = 'Before boundary' AND m.source_name = 'crm' AND m.active) <> 1
       OR (SELECT count(*) FROM mdm_out.customer WHERE name = 'After boundary') <> 0 THEN
        RAISE EXCEPTION 'source write after the returned boundary was published early';
    END IF;
END
$$;
SQL
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation <<'SQL'
SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'true'
       OR (result->>'publication_revision')::bigint <> 7
       OR result->'source_boundary'->>'completeness' <> 'PROVEN' THEN
        RAISE EXCEPTION 'next refresh did not consume the later source write: %', result;
    END IF;
END
$$;
RESET ROLE;
SQL
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation <<'SQL'
DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_out.customer c
        JOIN mdm_out.customer_members m USING (mdm_id)
        WHERE c.name = 'After boundary' AND m.source_name = 'crm' AND m.active) <> 1 THEN
        RAISE EXCEPTION 'retry did not publish the later source write: members %, outputs %',
            (SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.source_record_id), '[]'::jsonb)
             FROM mdm_out.customer_members m WHERE m.source_name = 'crm' AND m.active),
            (SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY c.mdm_id), '[]'::jsonb)
             FROM mdm_out.customer c);
    END IF;
END
$$;
DROP TRIGGER delay_release_output ON mdm_out.customer;
INSERT INTO public.crm_customer VALUES (5, 'Concurrent refresh', 'concurrent@example.test', statement_timestamp());
CREATE TRIGGER delay_concurrent_output
BEFORE INSERT OR UPDATE ON mdm_out.customer
FOR EACH ROW WHEN (NEW.name = 'Concurrent refresh')
EXECUTE FUNCTION public.delay_release_output();
SQL
docker exec -e PGAPPNAME=mdm_refresh_one "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$ DECLARE result jsonb; BEGIN
        result := mdm.refresh('customer', 'ALLOW');
        IF result->>'changed' <> 'true' OR (result->>'publication_revision')::bigint <> 8 THEN
            RAISE EXCEPTION 'first concurrent source refresh failed: %', result;
        END IF;
    END \$\$;" >"$work_dir/refresh_one.log" 2>&1 &
refresh_one=$!
refresh_paused=false
for _ in $(seq 1 600); do
    if [[ $(docker exec "$container" psql -X -At -U postgres -d foundation \
        -c "SELECT EXISTS (SELECT FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND classid = 718110::oid AND objid = 110972::oid AND objsubid = 2 AND granted)") == t ]]; then
        refresh_paused=true
        break
    fi
    sleep 0.1
done
if [[ $refresh_paused != true ]]; then
    kill "$refresh_one" 2>/dev/null || true
    wait "$refresh_one" || true
    cat "$work_dir/refresh_one.log"
    echo 'FAIL: first concurrent source refresh did not reach publication' >&2
    exit 1
fi
docker exec -e PGAPPNAME=mdm_refresh_two "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$ DECLARE result jsonb; BEGIN
        result := mdm.refresh('customer', 'ALLOW');
        IF result->>'changed' <> 'false' OR (result->>'publication_revision')::bigint <> 8 THEN
            RAISE EXCEPTION 'second concurrent source refresh failed: %', result;
        END IF;
    END \$\$;" >"$work_dir/refresh_two.log" 2>&1 &
refresh_two=$!
refreshes_overlapped=false
for _ in $(seq 1 600); do
    if [[ $(docker exec "$container" psql -X -At -U postgres -d foundation \
        -c "SELECT EXISTS (SELECT FROM pg_catalog.pg_stat_activity a, pg_catalog.pg_stat_activity b WHERE a.application_name = 'mdm_refresh_one' AND b.application_name = 'mdm_refresh_two' AND a.pid = ANY(pg_catalog.pg_blocking_pids(b.pid)))") == t ]]; then
        refreshes_overlapped=true
        break
    fi
    sleep 0.1
done
if ! wait "$refresh_one"; then cat "$work_dir/refresh_one.log"; exit 1; fi
if ! wait "$refresh_two"; then cat "$work_dir/refresh_two.log"; exit 1; fi
if [[ $refreshes_overlapped != true ]]; then
    echo 'FAIL: concurrent source refreshes did not contend on the entity lock' >&2
    exit 1
fi
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "DO \$\$
DECLARE
    source_state jsonb;
BEGIN
    SELECT COALESCE(pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
        'source_record_id', r.source_record_id,
        'source_active', r.active,
        'membership_source_record_id', m.source_record_id,
        'membership_active', m.active,
        'reader_source_record_id', om.source_record_id,
        'reader_active', om.active,
        'source_name', c.display_name,
        'output_name', o.name
    ) ORDER BY r.source_record_id), '[]'::jsonb)
      INTO source_state
      FROM mdm_internal.source_records r
      JOIN mdm_internal.source_identities s
        ON s.source_identity_id = r.source_identity_id AND s.entity_id = r.entity_id
      JOIN mdm_internal.entities e ON e.entity_id = r.entity_id
      JOIN public.crm_customer c
        ON r.source_record_key = pgtrickle.encode_row_id_v2(
            'SCAN_KEY', ROW(e.entity_id, s.source_identity_id, c.id))
      LEFT JOIN mdm_internal.memberships m
        ON m.entity_id = r.entity_id AND m.source_record_id = r.source_record_id
      LEFT JOIN mdm_out.customer_members om ON om.source_record_id = r.source_record_id
      LEFT JOIN mdm_out.customer o ON o.mdm_id = om.mdm_id
     WHERE e.entity_name = 'customer' AND c.id = 5;
    IF pg_catalog.jsonb_array_length(source_state) <> 1
       OR source_state->0->>'source_record_id' IS NULL
       OR source_state->0->>'source_record_id' IS DISTINCT FROM source_state->0->>'membership_source_record_id'
       OR source_state->0->>'source_record_id' IS DISTINCT FROM source_state->0->>'reader_source_record_id'
       OR source_state->0->>'source_active' IS DISTINCT FROM 'true'
       OR source_state->0->>'membership_active' IS DISTINCT FROM 'true'
       OR source_state->0->>'reader_active' IS DISTINCT FROM 'true'
       OR source_state->0->>'source_name' IS DISTINCT FROM 'Concurrent refresh'
       OR source_state->0->>'output_name' IS DISTINCT FROM 'Concurrent refresh' THEN
        RAISE EXCEPTION 'concurrent refreshes did not preserve one durable source record: %', source_state;
    END IF;
END
\$\$;
DROP TRIGGER delay_concurrent_output ON mdm_out.customer;
DROP FUNCTION public.delay_release_output();"

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

docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "DO \$\$
DECLARE
    state jsonb;
BEGIN
    SELECT pg_catalog.jsonb_build_object(
        'desired_version', e.desired_version,
        'v4_definition_count', (SELECT count(*) FROM mdm_internal.definitions d
                                WHERE d.entity_id = e.entity_id AND d.definition_version = 4),
        'v4_artifacts', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
            'compiler_version', a.compiler_version,
            'artifact_digest', pg_catalog.encode(a.artifact_digest, 'hex')))
            FROM mdm_internal.definition_artifacts a
            WHERE a.entity_id = e.entity_id AND a.definition_version = 4), '[]'::jsonb),
        'v4_bindings', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
            'artifact_id', b.artifact_id,
            'graph_binding_digest', pg_catalog.encode(b.graph_binding_digest, 'hex'),
            'member_count', (SELECT count(*) FROM mdm_internal.graph_members m
                            WHERE m.graph_binding_id = b.graph_binding_id),
            'unbound_members', (SELECT count(*) FROM mdm_internal.graph_members m
                                LEFT JOIN pg_catalog.pg_class c ON c.oid = m.relation_oid
                                WHERE m.graph_binding_id = b.graph_binding_id AND c.oid IS NULL))
            ORDER BY b.graph_binding_id)
            FROM mdm_internal.graph_bindings b
            WHERE b.entity_id = e.entity_id AND b.definition_version = 4), '[]'::jsonb),
        'newer_definitions', (SELECT count(*) FROM mdm_internal.definitions d
                              WHERE d.entity_id = e.entity_id AND d.definition_version > 4),
        'newer_artifacts', (SELECT count(*) FROM mdm_internal.definition_artifacts a
                            WHERE a.entity_id = e.entity_id AND a.definition_version > 4),
        'newer_bindings', (SELECT count(*) FROM mdm_internal.graph_bindings b
                           WHERE b.entity_id = e.entity_id AND b.definition_version > 4))
      INTO state
      FROM mdm_internal.entities e
     WHERE e.entity_name = 'customer';
    IF state->>'desired_version' IS DISTINCT FROM '4'
       OR state->>'v4_definition_count' IS DISTINCT FROM '1'
       OR pg_catalog.jsonb_array_length(state->'v4_artifacts') <> 1
       OR pg_catalog.jsonb_array_length(state->'v4_bindings') <> 1
       OR (state->'v4_bindings'->0->>'member_count')::bigint < 1
       OR (state->'v4_bindings'->0->>'unbound_members')::bigint <> 0
       OR state->>'newer_definitions' IS DISTINCT FROM '0'
       OR state->>'newer_artifacts' IS DISTINCT FROM '0'
       OR state->>'newer_bindings' IS DISTINCT FROM '0'
       OR pg_catalog.has_schema_privilege('mdm_administrator', 'mdm_graph', 'CREATE') THEN
        RAISE EXCEPTION 'losing concurrent installer left incomplete or duplicate graph state: %', state;
    END IF;
END
\$\$;"

original_source_oid=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "SELECT 'public.crm_customer'::regclass::oid")
original_role_oid=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "SELECT 'mdm_administrator'::regrole::oid")
artifact_query="SELECT md5(string_agg(encode(artifact_bytes, 'hex'), ',' ORDER BY definition_version)) FROM mdm_internal.definition_artifacts"
original_artifacts=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "$artifact_query")
source_identity_query="SELECT md5(COALESCE((SELECT string_agg(source_identity_id::text || ':' || encode(identity_digest, 'hex') || ':' || key_contract::text, '|' ORDER BY source_identity_id) FROM mdm_internal.source_identities), '') || '/' || COALESCE((SELECT string_agg(source_identity_id::text || ':' || encode(source_record_key, 'hex') || ':' || source_record_id::text || ':' || active::text, '|' ORDER BY source_identity_id, source_record_key) FROM mdm_internal.source_records), ''))"
original_source_identities=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "$source_identity_query")
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
    revision_before bigint;
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
    revision_before := (mdm.describe('customer', 'summary')->'publication'->>'publication_revision')::bigint;

    SELECT * INTO STRICT create_result
    FROM mdm.create(jsonb_set(mdm.describe('customer', 'definition'),
        '{limits,max_active_records}', '100'), 5);
    IF create_result.desired_version <> 6 THEN
        RAISE EXCEPTION 'resolver-limit retry did not create version 6';
    END IF;
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'false'
       OR (result->>'publication_revision')::bigint <> revision_before THEN
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
restored_source_identities=$(docker exec "$container" psql -X -At -U postgres -d restored -c "$source_identity_query")
test "$original_source_identities" = "$restored_source_identities"

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
test "$clone_sources" = 7
test "$restored_sources" = 6

docker exec -i "$container" psql -X -qAt -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -f /tests/operating_envelope.sql > "$work_dir/database-envelope.json"
python3 "$repo_root/scripts/record_e2e_evidence.py" \
    "$container" "$image" "$source_revision" \
    "$work_dir/database-envelope.json" "$e2e_log"

echo 'PASS: installation, Graph V1 admission, authorization, definition history, concurrency, and restore/rebind'
echo 'PASS: resolver-limit rollback and retry, backup/restore, and clone isolation'
echo 'PASS: candidate AUTO/FULL exact-row comparisons, FULL source oracle, and reported node strategies'
echo 'PASS: source writes after a returned boundary remain pending for the next refresh'
echo 'SKIPPED: Delta V1 positive conformance, output_delta_consumer is not used by V1'
