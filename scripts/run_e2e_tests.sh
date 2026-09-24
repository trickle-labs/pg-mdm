#!/usr/bin/env bash
set -euo pipefail

image=${PG_MDM_E2E_IMAGE:-pg_mdm:0.14.0-e2e}
container="pg-mdm-e2e-$$"
physical_container="pg-mdm-physical-$$"
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/pg-mdm-e2e.XXXXXX")
dump_file="$work_dir/foundation.dump"
physical_data="$work_dir/physical-data"
missing_graph_data="$work_dir/missing-graph-data"
missing_log="$work_dir/missing.log"
e2e_log="$work_dir/e2e-postgres.log"
qualification_json="$work_dir/incremental-qualification.json"
repo_root=$(cd "$(dirname "$0")/.." && pwd)
source_revision=$(git -C "$repo_root" rev-parse HEAD)

cleanup() {
    docker rm -fv "$container" "$physical_container" >/dev/null 2>&1 || true
    docker run --rm --user root \
        -v "$physical_data:/cleanup/physical" -v "$missing_graph_data:/cleanup/missing" \
        "$image" chown -R "$(id -u):$(id -g)" /cleanup/physical /cleanup/missing >/dev/null 2>&1 || true
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
grep -Eqi 'pg_trickle.*(not installed|not available)|(not installed|not available).*pg_trickle' "$missing_log"

docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -f /tests/e2e.sql | tee "$e2e_log"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -f /tests/e2e_policy.sql | tee -a "$e2e_log"
docker exec "$container" psql -X -qAt -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -f /tests/incremental_qualification.sql > "$qualification_json"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "CREATE TABLE public.recompile_canary_source (
            id bigint PRIMARY KEY,
            display_name text NOT NULL,
            email_address text,
            updated_at timestamptz NOT NULL
        );
        INSERT INTO public.recompile_canary_source
        VALUES (1, 'Compiler canary', 'compiler-canary@example.test', statement_timestamp());
        GRANT SELECT, MAINTAIN ON public.recompile_canary_source TO mdm_administrator;
        GRANT EXECUTE ON FUNCTION mdm_admin.recompile(text) TO mdm_administrator;"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$
        DECLARE created record; refreshed jsonb;
        BEGIN
            SELECT * INTO STRICT created FROM mdm.create(
                pg_catalog.jsonb_set(
                    pg_catalog.jsonb_set(
                        mdm.describe('customer', 'definition'),
                        '{name}', pg_catalog.to_jsonb('recompile_canary'::text)),
                    '{sources,0,relation}', pg_catalog.to_jsonb('public.recompile_canary_source'::text)));
            refreshed := mdm.refresh('recompile_canary', 'ALLOW');
            IF NOT created.changed OR created.desired_version <> 1
               OR refreshed->>'changed' <> 'true'
               OR refreshed->>'publication_revision' <> '1' THEN
                RAISE EXCEPTION 'compiler canary bootstrap failed: create %, refresh %', created, refreshed;
            END IF;
        END \$\$;"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "CREATE TABLE public.recompile_canary_snapshot AS
        SELECT e.publication_revision,
               max(b.graph_generation) AS graph_generation,
               (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(o) ORDER BY o.mdm_id)
                  FROM mdm_out.recompile_canary o) AS output
          FROM mdm_internal.entities e
          JOIN mdm_internal.graph_bindings b USING (entity_id)
         WHERE e.entity_name = 'recompile_canary'
         GROUP BY e.entity_id, e.publication_revision;
        UPDATE mdm_internal.definition_artifacts a
           SET compiler_version = 9,
               artifact_digest = pg_catalog.decode(pg_catalog.repeat('00', 32), 'hex')
          FROM mdm_internal.entities e
         WHERE e.entity_id = a.entity_id
           AND e.entity_name = 'recompile_canary'
           AND a.definition_version = e.desired_version;"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$
        DECLARE recompiled jsonb; refreshed jsonb;
        BEGIN
            recompiled := mdm_admin.recompile('recompile_canary');
            refreshed := mdm.refresh('recompile_canary', 'ALLOW');
            IF recompiled->>'changed' <> 'false'
               OR recompiled->>'graph_recompiled' <> 'true'
               OR recompiled->>'desired_version' <> '1'
               OR refreshed->>'changed' <> 'false'
               OR refreshed->>'publication_revision' <> '1'
               OR refreshed->>'resolver_strategy' <> 'full'
               OR refreshed->>'resolver_fallback_reason' <> 'graph_generation_transition' THEN
                RAISE EXCEPTION 'compiler canary adoption failed: recompile %, refresh %', recompiled, refreshed;
            END IF;
        END \$\$;"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "DO \$\$
        DECLARE state jsonb;
        BEGIN
            SELECT pg_catalog.jsonb_build_object(
                'definition_version', e.desired_version,
                'publication_revision', e.publication_revision,
                'previous_publication_revision', s.publication_revision,
                'graph_generation', max(b.graph_generation),
                'previous_graph_generation', s.graph_generation,
                'compiler_versions', (SELECT pg_catalog.jsonb_agg(a.compiler_version ORDER BY a.compiler_version)
                    FROM mdm_internal.definition_artifacts a WHERE a.entity_id = e.entity_id),
                'terminal_consumers', (SELECT count(*) FROM mdm_internal.graph_delta_consumers c
                    WHERE c.graph_binding_id = (SELECT graph_binding_id FROM mdm_internal.graph_bindings
                        WHERE entity_id = e.entity_id ORDER BY graph_generation DESC LIMIT 1)),
                'output_equal', s.output IS NOT DISTINCT FROM (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(o) ORDER BY o.mdm_id)
                    FROM mdm_out.recompile_canary o))
              INTO state
              FROM mdm_internal.entities e
              JOIN mdm_internal.graph_bindings b USING (entity_id)
              CROSS JOIN public.recompile_canary_snapshot s
             WHERE e.entity_name = 'recompile_canary'
             GROUP BY e.entity_id, e.desired_version, e.publication_revision,
                      s.publication_revision, s.graph_generation, s.output;
            IF state IS DISTINCT FROM pg_catalog.jsonb_build_object(
                'definition_version', 1, 'publication_revision', 1,
                'previous_publication_revision', 1,
                'graph_generation', (state->>'previous_graph_generation')::bigint + 1,
                'previous_graph_generation', (state->>'previous_graph_generation')::bigint,
                'compiler_versions', '[9, 10]'::jsonb,
                'terminal_consumers', 2, 'output_equal', true) THEN
                RAISE EXCEPTION 'compiler v9 to v10 canary state is invalid: %', state;
            END IF;
        END \$\$;"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; SELECT mdm_admin.drop_entity('recompile_canary', 'recompile_canary');"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "DROP TABLE public.recompile_canary_snapshot;
        DROP TABLE public.recompile_canary_source;"
current_revision=$(docker exec "$container" psql -X -At -U postgres -d foundation \
    -c "SELECT publication_revision FROM mdm_internal.entities WHERE entity_name = 'customer'")
expected_revision=$((current_revision + 1))

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
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -v expected_revision="$expected_revision" -U mdm_test_login -d foundation <<'SQL' >"$work_dir/boundary_refresh.log" 2>&1 &
SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb; expected_revision bigint;
BEGIN
    SELECT (mdm.describe('customer', 'summary')->'publication'->>'publication_revision')::bigint + 1
    INTO expected_revision;
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'true'
       OR (result->>'publication_revision')::bigint <> expected_revision
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
current_revision=$expected_revision
expected_revision=$((current_revision + 1))
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
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -v expected_revision="$expected_revision" -U mdm_test_login -d foundation <<'SQL'
SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb; expected_revision bigint;
BEGIN
    SELECT (mdm.describe('customer', 'summary')->'publication'->>'publication_revision')::bigint + 1
    INTO expected_revision;
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'true'
       OR (result->>'publication_revision')::bigint <> expected_revision
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
current_revision=$expected_revision
expected_revision=$((current_revision + 1))
docker exec -e PGAPPNAME=mdm_refresh_one "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -v expected_revision="$expected_revision" -c "SET ROLE mdm_administrator; DO \$\$ DECLARE result jsonb; BEGIN
        result := mdm.refresh('customer', 'ALLOW');
        IF result->>'changed' <> 'true' OR (result->>'publication_revision')::bigint <> $expected_revision THEN
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
    -v expected_revision="$expected_revision" -c "SET ROLE mdm_administrator; DO \$\$ DECLARE result jsonb; BEGIN
        result := mdm.refresh('customer', 'ALLOW');
        IF result->>'changed' <> 'false' OR (result->>'publication_revision')::bigint <> $expected_revision THEN
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
current_revision=$expected_revision
expected_revision=$((current_revision + 1))
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

docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "GRANT EXECUTE ON FUNCTION mdm_steward.override_golden(text, uuid, text, jsonb, bigint, text) TO mdm_administrator;
        CREATE FUNCTION public.e2e_source_records(ids bigint[])
        RETURNS TABLE(source_record_id uuid, source_record_key bytea)
        LANGUAGE sql SECURITY DEFINER
        SET search_path = pg_catalog, mdm_internal, public
        AS \$\$
            SELECT r.source_record_id, r.source_record_key
            FROM mdm_internal.source_records r
            JOIN mdm_internal.source_identities s
              ON s.source_identity_id = r.source_identity_id AND s.entity_id = r.entity_id
            JOIN mdm_internal.entities e ON e.entity_id = r.entity_id
            JOIN public.crm_customer c
              ON r.source_record_key = pgtrickle.encode_row_id_v2(
                  'SCAN_KEY', ROW(e.entity_id, s.source_identity_id, c.id))
            WHERE e.entity_name = 'customer' AND c.id = ANY (ids)
        \$\$;
        GRANT EXECUTE ON FUNCTION public.e2e_source_records(bigint[]) TO mdm_administrator;
        INSERT INTO public.crm_customer VALUES (6, 'Directive race', 'directive-race@example.test', statement_timestamp());
        CREATE FUNCTION public.delay_release_output()
        RETURNS trigger LANGUAGE plpgsql AS \$\$
        BEGIN
            IF pg_catalog.current_setting('mdm.e2e_release_waited', true) IS DISTINCT FROM 'on' THEN
                PERFORM pg_catalog.set_config('mdm.e2e_release_waited', 'on', true);
                PERFORM pg_catalog.pg_advisory_xact_lock(718110, 110972);
                PERFORM pg_catalog.pg_sleep(5);
            END IF;
            RETURN NEW;
        END
        \$\$;
        CREATE TRIGGER delay_directive_race_output
        BEFORE INSERT OR UPDATE ON mdm_out.customer
        FOR EACH ROW WHEN (NEW.name IN ('Directive race', 'Pair decision race'))
        EXECUTE FUNCTION public.delay_release_output();"
docker exec -e PGAPPNAME=mdm_directive_race_refresh "$container" psql -X -v ON_ERROR_STOP=1 -v expected_revision="$expected_revision" -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$ DECLARE result jsonb; BEGIN
        result := mdm.refresh('customer', 'ALLOW');
        IF result->>'changed' <> 'true' OR (result->>'publication_revision')::bigint <> $expected_revision THEN
            RAISE EXCEPTION 'directive-race publication failed: %', result;
        END IF;
    END \$\$;" >"$work_dir/directive_race_refresh.log" 2>&1 &
directive_race_refresh=$!
directive_race_paused=false
for _ in $(seq 1 600); do
    if [[ $(docker exec "$container" psql -X -At -U postgres -d foundation \
        -c "SELECT EXISTS (SELECT FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND classid = 718110::oid AND objid = 110972::oid AND objsubid = 2 AND granted)") == t ]]; then
        directive_race_paused=true
        break
    fi
    sleep 0.1
done
if [[ $directive_race_paused != true ]]; then
    kill "$directive_race_refresh" 2>/dev/null || true
    wait "$directive_race_refresh" || true
    cat "$work_dir/directive_race_refresh.log"
    echo 'FAIL: directive-race refresh did not reach publication' >&2
    exit 1
fi
docker exec -e PGAPPNAME=mdm_directive_race_override "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; SELECT * FROM mdm_steward.override_golden(
        'customer', (SELECT source_record_id FROM public.e2e_source_records(ARRAY[9001]::bigint[])),
        'name', '\"Race override\"'::jsonb, 0, 'publication race');" \
    >"$work_dir/directive_race_override.log" 2>&1 &
directive_race_override=$!
directive_race_blocked=false
for _ in $(seq 1 600); do
    if [[ $(docker exec "$container" psql -X -At -U postgres -d foundation \
        -c "SELECT EXISTS (SELECT FROM pg_catalog.pg_stat_activity a, pg_catalog.pg_stat_activity b WHERE a.application_name = 'mdm_directive_race_refresh' AND b.application_name = 'mdm_directive_race_override' AND a.pid = ANY(pg_catalog.pg_blocking_pids(b.pid)))") == t ]]; then
        directive_race_blocked=true
        break
    fi
    sleep 0.1
done
if ! wait "$directive_race_refresh"; then cat "$work_dir/directive_race_refresh.log"; exit 1; fi
if ! wait "$directive_race_override"; then cat "$work_dir/directive_race_override.log"; exit 1; fi
if [[ $directive_race_blocked != true ]]; then
    echo 'FAIL: golden override did not contend with publication on the entity lock' >&2
    exit 1
fi
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -v expected_revision="$expected_revision" -U postgres -d foundation \
    -c "DO \$\$
DECLARE state jsonb;
DECLARE anchor_id uuid;
BEGIN
    SELECT source_record_id INTO STRICT anchor_id
      FROM public.e2e_source_records(ARRAY[9001]::bigint[]);
    SELECT pg_catalog.jsonb_build_object(
        'directives', (SELECT COALESCE(pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
            'action', d.action, 'value', d.value, 'value_type_name', d.value_type_name,
            'override_version', d.override_version, 'reason', d.reason,
            'created_by_name', d.created_by_name, 'created_as_role_name', d.created_as_role_name,
            'base_publication_revision', d.base_publication_revision, 'decision_epoch', d.decision_epoch,
            'supersedes', d.supersedes, 'is_current', d.is_current,
            'operation_id', d.operation_id) ORDER BY d.override_version), '[]'::jsonb)
            FROM mdm_internal.golden_override_directives d
            JOIN mdm_internal.entities e ON e.entity_name = 'customer' AND e.entity_id = d.entity_id
            WHERE d.field_name = 'name' AND d.anchor_source_record_id = anchor_id),
        'operations', (SELECT COALESCE(pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
            'operation_kind', o.operation_kind, 'status', o.status, 'result_code', o.result_code,
            'outcome', o.outcome, 'actor_name', o.actor_name, 'actor_role_name', o.actor_role_name)
            ORDER BY o.started_at), '[]'::jsonb)
            FROM mdm_internal.operations o
            WHERE o.operation_id IN (SELECT d.operation_id FROM mdm_internal.golden_override_directives d
                JOIN mdm_internal.entities e USING (entity_id)
                WHERE e.entity_name = 'customer' AND d.field_name = 'name'
                  AND d.anchor_source_record_id = anchor_id)),
        'publication_revision', (SELECT publication_revision FROM mdm_internal.entities WHERE entity_name = 'customer'),
        'race_output_count', (SELECT count(*) FROM mdm_out.customer c
            JOIN mdm_out.customer_members m USING (mdm_id)
            JOIN public.e2e_source_records(ARRAY[6]::bigint[]) r USING (source_record_id)
            WHERE c.name = 'Directive race' AND m.source_name = 'crm' AND m.active))
      INTO state;
    IF state IS DISTINCT FROM pg_catalog.jsonb_build_object(
        'directives', pg_catalog.jsonb_build_array(pg_catalog.jsonb_build_object(
            'action', 'SET', 'value', '\"Race override\"'::jsonb, 'value_type_name', 'text',
            'override_version', 1, 'reason', 'publication race',
            'created_by_name', 'mdm_test_login', 'created_as_role_name', 'mdm_administrator',
            'base_publication_revision', $expected_revision, 'decision_epoch', 4, 'supersedes', NULL, 'is_current', true,
            'operation_id', (SELECT d.operation_id FROM mdm_internal.golden_override_directives d
                JOIN mdm_internal.entities e USING (entity_id)
                WHERE e.entity_name = 'customer' AND d.field_name = 'name'
                  AND d.anchor_source_record_id = anchor_id))),
        'operations', pg_catalog.jsonb_build_array(pg_catalog.jsonb_build_object(
            'operation_kind', 'golden_override', 'status', 'succeeded', 'result_code', 'MDM_OK',
            'outcome', (SELECT pg_catalog.jsonb_build_object(
                    'field', 'name', 'action', 'SET', 'base_publication_revision', $expected_revision,
                    'override_id', d.override_id, 'decision_epoch', 4)
                FROM mdm_internal.operations o
                JOIN mdm_internal.golden_override_directives d USING (operation_id)
                JOIN mdm_internal.entities e USING (entity_id)
                WHERE e.entity_name = 'customer' AND d.field_name = 'name'
                  AND d.anchor_source_record_id = anchor_id),
            'actor_name', 'mdm_test_login', 'actor_role_name', 'mdm_administrator')),
        'publication_revision', $expected_revision, 'race_output_count', 1) THEN
        RAISE EXCEPTION 'golden override/publication race left unexpected durable history or output: %', state;
    END IF;
END
\$\$;"
current_revision=$expected_revision
expected_revision=$((current_revision + 1))
if ! docker exec -e PGAPPNAME=mdm_directive_race_apply "$container" psql -X -v ON_ERROR_STOP=1 -v expected_revision="$expected_revision" -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$ DECLARE result jsonb; BEGIN
        result := mdm.refresh('customer', 'ALLOW');
        IF result->>'changed' <> 'true'
           OR NOT (
               (result->>'resolver_strategy' = 'affected' AND result->'resolver_fallback_reason' = 'null'::jsonb)
               OR (result->>'resolver_strategy' = 'full'
                   AND result->>'resolver_fallback_reason' = 'delta_full_invalidation'))
           OR (result->>'publication_revision')::bigint <> $expected_revision THEN
            RAISE EXCEPTION 'golden override was not applied by the next refresh: %', result;
        END IF;
    END \$\$;" >"$work_dir/directive_race_apply.log" 2>&1; then
    cat "$work_dir/directive_race_apply.log"
    exit 1
fi
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "DO \$\$ BEGIN
        IF (SELECT count(*) FROM mdm_out.customer c
            JOIN mdm_out.customer_members m USING (mdm_id)
            JOIN public.e2e_source_records(ARRAY[9001]::bigint[]) r USING (source_record_id)
            WHERE c.name = 'Race override' AND m.source_name = 'crm' AND m.active) <> 1 THEN
            RAISE EXCEPTION 'golden override was not visible in the published output';
        END IF;
    END \$\$;"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "INSERT INTO public.crm_customer VALUES (7, 'Pair decision race', 'pair-race@example.test', statement_timestamp())" >/dev/null
current_revision=$expected_revision
expected_revision=$((current_revision + 1))
docker exec -e PGAPPNAME=mdm_pair_decision_refresh "$container" psql -X -v ON_ERROR_STOP=1 -v expected_revision="$expected_revision" -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$ DECLARE result jsonb; BEGIN
        result := mdm.refresh('customer', 'ALLOW');
        IF result->>'changed' <> 'true' OR (result->>'publication_revision')::bigint <> $expected_revision THEN
            RAISE EXCEPTION 'pair-decision publication failed: %', result;
        END IF;
    END \$\$;" >"$work_dir/pair_decision_refresh.log" 2>&1 &
pair_decision_refresh=$!
pair_decision_paused=false
for _ in $(seq 1 600); do
    if [[ $(docker exec "$container" psql -X -At -U postgres -d foundation \
        -c "SELECT EXISTS (SELECT FROM pg_catalog.pg_locks WHERE locktype = 'advisory' AND classid = 718110::oid AND objid = 110972::oid AND objsubid = 2 AND granted)") == t ]]; then
        pair_decision_paused=true
        break
    fi
    sleep 0.1
done
if [[ $pair_decision_paused != true ]]; then
    kill "$pair_decision_refresh" 2>/dev/null || true
    wait "$pair_decision_refresh" || true
    cat "$work_dir/pair_decision_refresh.log"
    echo 'FAIL: pair-decision refresh did not reach publication' >&2
    exit 1
fi
docker exec -e PGAPPNAME=mdm_pair_decision_write "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$
        DECLARE left_id uuid; right_id uuid; result record;
        BEGIN
            SELECT source_record_id INTO STRICT left_id FROM public.e2e_source_records(ARRAY[4]::bigint[]);
            SELECT source_record_id INTO STRICT right_id FROM public.e2e_source_records(ARRAY[6]::bigint[]);
            SELECT * INTO STRICT result FROM mdm_steward.decide(
                'customer', left_id, right_id, 'MATCH', 0, 'pair decision publication race');
            IF result.decision_version <> 1 OR result.decision_epoch <> 5 THEN
                RAISE EXCEPTION 'pair decision was not serialized after publication: %', result;
            END IF;
        END \$\$;" >"$work_dir/pair_decision_write.log" 2>&1 &
pair_decision_write=$!
pair_decision_blocked=false
for _ in $(seq 1 600); do
    if [[ $(docker exec "$container" psql -X -At -U postgres -d foundation \
        -c "SELECT EXISTS (SELECT FROM pg_catalog.pg_stat_activity a, pg_catalog.pg_stat_activity b WHERE a.application_name = 'mdm_pair_decision_refresh' AND b.application_name = 'mdm_pair_decision_write' AND a.pid = ANY(pg_catalog.pg_blocking_pids(b.pid)))") == t ]]; then
        pair_decision_blocked=true
        break
    fi
    sleep 0.1
done
if ! wait "$pair_decision_refresh"; then cat "$work_dir/pair_decision_refresh.log"; exit 1; fi
if ! wait "$pair_decision_write"; then cat "$work_dir/pair_decision_write.log"; exit 1; fi
if [[ $pair_decision_blocked != true ]]; then
    echo 'FAIL: pair decision did not contend with publication on the entity lock' >&2
    exit 1
fi
current_revision=$expected_revision
expected_revision=$((current_revision + 1))
if ! docker exec -e PGAPPNAME=mdm_pair_decision_apply "$container" psql -X -v ON_ERROR_STOP=1 -v expected_revision="$expected_revision" -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$ DECLARE result jsonb; BEGIN
        result := mdm.refresh('customer', 'ALLOW');
        IF result->>'changed' <> 'true'
           OR NOT (
               (result->>'resolver_strategy' = 'affected' AND result->'resolver_fallback_reason' = 'null'::jsonb)
               OR (result->>'resolver_strategy' = 'full'
                   AND result->>'resolver_fallback_reason' = 'delta_full_invalidation'))
           OR (result->>'publication_revision')::bigint <> $expected_revision THEN
            RAISE EXCEPTION 'pair decision was not applied by affected resolution: %', result;
        END IF;
    END \$\$;" >"$work_dir/pair_decision_apply.log" 2>&1; then
    cat "$work_dir/pair_decision_apply.log"
    exit 1
fi
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "DO \$\$
        DECLARE state jsonb;
        BEGIN
            SELECT pg_catalog.jsonb_build_object(
                'row_count', (SELECT count(*) FROM mdm_internal.steward_decisions d
                    JOIN mdm_internal.entities e USING (entity_id)
                    WHERE e.entity_name = 'customer' AND d.reason = 'pair decision publication race'),
                'decision', d.decision, 'decision_version', d.decision_version,
                'decision_epoch', d.decision_epoch, 'base_publication_revision', d.base_publication_revision,
                'reason', d.reason, 'created_by_name', d.created_by_name,
                'created_as_role_name', d.created_as_role_name, 'is_current', d.is_current,
                'operation_kind', o.operation_kind, 'status', o.status, 'result_code', o.result_code,
                'actor_name', o.actor_name, 'actor_role_name', o.actor_role_name,
                'decision_id_matches', o.outcome->>'decision_id' = d.decision_id::text,
                'outcome_epoch', (o.outcome->>'decision_epoch')::bigint,
                'merged_output', (SELECT count(DISTINCT m.mdm_id) = 1
                    FROM mdm_out.customer_members m
                    JOIN public.e2e_source_records(ARRAY[4, 6]::bigint[]) r USING (source_record_id)
                    WHERE m.active))
              INTO state
              FROM mdm_internal.steward_decisions d
              JOIN mdm_internal.entities e USING (entity_id)
              JOIN mdm_internal.operations o USING (operation_id)
             WHERE e.entity_name = 'customer' AND d.reason = 'pair decision publication race';
            IF state IS DISTINCT FROM pg_catalog.jsonb_build_object(
                'row_count', 1, 'decision', 'MATCH', 'decision_version', 1,
                'decision_epoch', 5, 'base_publication_revision', $current_revision,
                'reason', 'pair decision publication race', 'created_by_name', 'mdm_test_login',
                'created_as_role_name', 'mdm_administrator', 'is_current', true,
                'operation_kind', 'steward_decide', 'status', 'succeeded', 'result_code', 'MDM_OK',
                'actor_name', 'mdm_test_login', 'actor_role_name', 'mdm_administrator',
                'decision_id_matches', true, 'outcome_epoch', 5, 'merged_output', true) THEN
                RAISE EXCEPTION 'pair-decision publication race left unexpected durable audit: %', state;
            END IF;
        END \$\$;"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "DROP TRIGGER delay_directive_race_output ON mdm_out.customer;
        DROP FUNCTION public.delay_release_output();
        DROP FUNCTION public.e2e_source_records(bigint[]);"

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
original_operations=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "SELECT count(*) FROM mdm_internal.operations")
original_policy_digest=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "SELECT md5(COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) - 'last_observed_at' ORDER BY c.case_key) FROM mdm_steward.policy_cases_v1 c WHERE c.entity_name = 'policy_qualification'), '[]'::jsonb)::text)")
original_policy_max=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "SELECT max(case_key) FROM mdm_steward.policy_cases_v1 WHERE entity_name = 'policy_qualification'")
restore_issue_key=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "SELECT pg_catalog.encode(issue_key, 'hex') FROM mdm_steward.policy_cases_v1 WHERE entity_name = 'policy_qualification' AND status = 'resolved' ORDER BY case_key DESC LIMIT 1")
artifact_query="SELECT md5(string_agg(encode(artifact_bytes, 'hex'), ',' ORDER BY definition_version)) FROM mdm_internal.definition_artifacts"
original_artifacts=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "$artifact_query")
source_identity_query="SELECT md5(COALESCE((SELECT string_agg(source_identity_id::text || ':' || encode(identity_digest, 'hex') || ':' || key_contract::text, '|' ORDER BY source_identity_id) FROM mdm_internal.source_identities), '') || '/' || COALESCE((SELECT string_agg(source_identity_id::text || ':' || encode(source_record_key, 'hex') || ':' || source_record_id::text || ':' || active::text, '|' ORDER BY source_identity_id, source_record_key) FROM mdm_internal.source_records), ''))"
original_source_identities=$(docker exec "$container" psql -X -At -U postgres -d foundation -c "$source_identity_query")
docker exec "$container" pg_dump -Fc -U postgres foundation >"$dump_file"
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "CREATE FUNCTION public.e2e_customer_publication_state() RETURNS jsonb
        LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog AS \$e2e\$
        SELECT pg_catalog.jsonb_build_object(
            'publication_revision', (SELECT publication_revision FROM mdm_internal.entities WHERE entity_name = 'customer'),
            'publication_count', (SELECT count(*) FROM mdm_internal.publications p JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer'),
            'members', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(m) ORDER BY m.source_record_id) FROM mdm_out.customer_members m), '[]'::jsonb),
            'entities', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(o) ORDER BY o.mdm_id) FROM mdm_out.customer o), '[]'::jsonb),
            'reviews', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.review_id) FROM mdm_out.customer_review r), '[]'::jsonb)
        )
        \$e2e\$;
        GRANT EXECUTE ON FUNCTION public.e2e_customer_publication_state() TO mdm_administrator"
docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation <<'SQL'
SET ROLE mdm_administrator;
DO $$
DECLARE
    failed boolean := false;
    create_result record;
    result jsonb;
    retry_result jsonb;
    before_state jsonb;
    failed_state jsonb;
    retry_state jsonb;
    final_state jsonb;
BEGIN
    before_state := public.e2e_customer_publication_state();
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
    failed_state := public.e2e_customer_publication_state();
    IF failed_state->'publication_revision' IS DISTINCT FROM before_state->'publication_revision'
       OR failed_state->'publication_count' IS DISTINCT FROM before_state->'publication_count'
       OR failed_state->'members' IS DISTINCT FROM before_state->'members'
       OR failed_state->'entities' IS DISTINCT FROM before_state->'entities'
       OR failed_state->'reviews' IS DISTINCT FROM before_state->'reviews' THEN
        RAISE EXCEPTION 'resolver-limit failure changed the published state: before %, after %',
            before_state, failed_state;
    END IF;

    SELECT * INTO STRICT create_result
    FROM mdm.create(jsonb_set(mdm.describe('customer', 'definition'),
        '{limits,max_active_records}', '100'), 5);
    IF create_result.desired_version <> 6 THEN
        RAISE EXCEPTION 'resolver-limit retry did not create version 6';
    END IF;
    result := mdm.refresh('customer', 'ALLOW');
    retry_state := public.e2e_customer_publication_state();
    IF (result->>'publication_revision')::bigint <> (retry_state->>'publication_revision')::bigint THEN
        RAISE EXCEPTION 'resolver-limit retry returned a stale publication revision: %, state %',
            result, retry_state;
    END IF;

    retry_result := mdm.refresh('customer', 'ALLOW');
    final_state := public.e2e_customer_publication_state();
    IF retry_result->>'changed' <> 'false'
       OR (retry_result->>'publication_revision')::bigint <> (retry_state->>'publication_revision')::bigint
       OR final_state->'publication_revision' IS DISTINCT FROM retry_state->'publication_revision'
       OR final_state->'publication_count' IS DISTINCT FROM retry_state->'publication_count'
       OR final_state->'members' IS DISTINCT FROM retry_state->'members'
       OR final_state->'entities' IS DISTINCT FROM retry_state->'entities'
       OR final_state->'reviews' IS DISTINCT FROM retry_state->'reviews' THEN
        RAISE EXCEPTION 'resolver-limit retry was not stable: result %, before %, after %',
            retry_result, retry_state, final_state;
    END IF;
END
$$;
RESET ROLE;
SQL
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c "CREATE FUNCTION public.e2e_graph_member_row_counts() RETURNS jsonb
        LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS \$e2e\$
        DECLARE member record; row_count bigint; counts jsonb := '{}'::jsonb;
        BEGIN
            FOR member IN
                SELECT gm.logical_id, gm.relation_oid
                FROM mdm_internal.graph_members gm
                JOIN mdm_internal.graph_bindings b USING (graph_binding_id)
                JOIN mdm_internal.entities e USING (entity_id)
                WHERE e.entity_name = 'customer' AND b.definition_version = e.desired_version
                ORDER BY gm.topological_ordinal
            LOOP
                EXECUTE pg_catalog.format('SELECT count(*) FROM %s', member.relation_oid::regclass)
                    INTO row_count;
                counts := counts || pg_catalog.jsonb_build_object(member.logical_id, row_count);
            END LOOP;
            RETURN counts;
        END
        \$e2e\$;
        GRANT EXECUTE ON FUNCTION public.e2e_graph_member_row_counts() TO mdm_administrator"
physical_revision=$(docker exec "$container" psql -X -At -U postgres -d foundation \
    -c "SELECT publication_revision FROM mdm_internal.entities WHERE entity_name = 'customer'")
physical_state=$(docker exec "$container" psql -X -At -U postgres -d foundation \
    -c "SELECT md5(public.e2e_customer_publication_state()::text)")
physical_pgdata=$(docker exec "$container" psql -X -At -U postgres -d foundation -c 'SHOW data_directory')
mkdir -p "$physical_data"
docker exec -u postgres "$container" mkdir -p /tmp/pg-mdm-physical-backup
docker exec -u postgres -e PGPASSWORD=postgres "$container" \
    pg_basebackup -h 127.0.0.1 -U postgres -D /tmp/pg-mdm-physical-backup -Fp -Xs >/dev/null
docker cp "$container:/tmp/pg-mdm-physical-backup/." "$physical_data/" >/dev/null
docker run --detach --user root --name "$physical_container" -e POSTGRES_PASSWORD=postgres -e PGDATA="$physical_pgdata" \
    -v "$physical_data:$physical_pgdata" "$image" >/dev/null
for _ in $(seq 1 60); do
    if docker exec "$physical_container" pg_isready -U postgres >/dev/null 2>&1; then
        break
    fi
    sleep 1
done
docker exec "$physical_container" pg_isready -U postgres >/dev/null
physical_refresh=$(docker exec "$physical_container" psql -X -qAt -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; SELECT (result->>'changed') || '|' || (result->>'publication_revision') FROM (SELECT mdm.refresh('customer', 'ALLOW') AS result) refresh")
if [[ $physical_refresh != "false|$physical_revision" ]]; then
    echo "FAIL: intact physical recovery refresh returned '$physical_refresh', expected 'false|$physical_revision'" >&2
    exit 1
fi
physical_recovered_state=$(docker exec "$physical_container" psql -X -At -U postgres -d foundation \
    -c "SELECT md5(public.e2e_customer_publication_state()::text)")
if [[ $physical_recovered_state != "$physical_state" ]]; then
    echo "FAIL: intact physical recovery changed publication state: $physical_recovered_state != $physical_state" >&2
    exit 1
fi
physical_graph_populated=$(docker exec "$physical_container" psql -X -At -U postgres -d foundation \
    -c "SELECT COALESCE(pg_catalog.bool_or(row_count::bigint > 0), false) FROM pg_catalog.jsonb_each_text(public.e2e_graph_member_row_counts()) AS member(logical_id, row_count)")
if [[ $physical_graph_populated != t ]]; then
    echo "FAIL: intact physical recovery graph members were empty: $physical_graph_populated" >&2
    exit 1
fi
docker exec "$physical_container" psql -X -v ON_ERROR_STOP=1 -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; DO \$\$
        DECLARE result record;
        BEGIN
            SELECT * INTO STRICT result FROM mdm.create(jsonb_set(
                mdm.describe('customer', 'definition'), '{limits,max_active_records}', '101'), 6);
            IF NOT result.changed OR result.desired_version <> 7 THEN
                RAISE EXCEPTION 'physical recovery fixture did not create pending graph version: %', result;
            END IF;
        END \$\$;"
missing_graph_empty=$(docker exec "$physical_container" psql -X -At -U postgres -d foundation \
    -c "SELECT count(*) > 0 AND pg_catalog.bool_and(row_count::bigint = 0) FROM pg_catalog.jsonb_each_text(public.e2e_graph_member_row_counts()) AS member(logical_id, row_count)")
if [[ $missing_graph_empty != t ]]; then
    echo "FAIL: pending physical graph members were not empty: $missing_graph_empty" >&2
    exit 1
fi
mkdir -p "$missing_graph_data"
docker exec -u postgres "$physical_container" mkdir -p /tmp/pg-mdm-missing-graph-backup
docker exec -u postgres -e PGPASSWORD=postgres "$physical_container" \
    pg_basebackup -h 127.0.0.1 -U postgres -D /tmp/pg-mdm-missing-graph-backup -Fp -Xs >/dev/null
docker cp "$physical_container:/tmp/pg-mdm-missing-graph-backup/." "$missing_graph_data/" >/dev/null
docker rm -fv "$physical_container" >/dev/null
docker run --detach --user root --name "$physical_container" -e POSTGRES_PASSWORD=postgres -e PGDATA="$physical_pgdata" \
    -v "$missing_graph_data:$physical_pgdata" "$image" >/dev/null
for _ in $(seq 1 60); do
    if docker exec "$physical_container" pg_isready -U postgres >/dev/null 2>&1; then
        break
    fi
    sleep 1
done
docker exec "$physical_container" pg_isready -U postgres >/dev/null
missing_graph_state=$(docker exec "$physical_container" psql -X -At -U postgres -d foundation \
    -c "SELECT md5(public.e2e_customer_publication_state()::text)")
if [[ $missing_graph_state != "$physical_state" ]]; then
    echo "FAIL: pending physical graph recovery changed publication state: $missing_graph_state != $physical_state" >&2
    exit 1
fi
physical_rebuild_refresh=$(docker exec "$physical_container" psql -X -qAt -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; SELECT (result->>'changed') || '|' || (result->>'publication_revision') FROM (SELECT mdm.refresh('customer', 'ALLOW') AS result) refresh")
expected_rebuilt_revision=$((physical_revision + 1))
if [[ $physical_rebuild_refresh != "true|$expected_rebuilt_revision" ]]; then
    echo "FAIL: pending physical graph rebuild returned '$physical_rebuild_refresh', expected 'true|$expected_rebuilt_revision'" >&2
    exit 1
fi
rebuilt_graph_populated=$(docker exec "$physical_container" psql -X -At -U postgres -d foundation \
    -c "SELECT COALESCE(pg_catalog.bool_or(row_count::bigint > 0), false) FROM pg_catalog.jsonb_each_text(public.e2e_graph_member_row_counts()) AS member(logical_id, row_count)")
if [[ $rebuilt_graph_populated != t ]]; then
    echo "FAIL: physical recovery did not populate desired graph members: $rebuilt_graph_populated" >&2
    exit 1
fi
rebuilt_publication_state=$(docker exec "$physical_container" psql -X -At -U postgres -d foundation \
    -c "SELECT md5(public.e2e_customer_publication_state()::text)")
stable_rebuild_refresh=$(docker exec "$physical_container" psql -X -qAt -U mdm_test_login -d foundation \
    -c "SET ROLE mdm_administrator; SELECT (result->>'changed') || '|' || (result->>'publication_revision') FROM (SELECT mdm.refresh('customer', 'ALLOW') AS result) refresh")
if [[ $stable_rebuild_refresh != "false|$expected_rebuilt_revision" ]]; then
    echo "FAIL: recovered graph retry returned '$stable_rebuild_refresh', expected 'false|$expected_rebuilt_revision'" >&2
    exit 1
fi
stable_rebuild_state=$(docker exec "$physical_container" psql -X -At -U postgres -d foundation \
    -c "SELECT md5(public.e2e_customer_publication_state()::text)")
if [[ $stable_rebuild_state != "$rebuilt_publication_state" ]]; then
    echo "FAIL: retry changed recovered publication state: $stable_rebuild_state != $rebuilt_publication_state" >&2
    exit 1
fi
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -c 'REVOKE ALL ON SCHEMA pgtrickle FROM mdm_administrator CASCADE;
        REVOKE ALL ON FUNCTION pgtrickle.encode_row_id_v2(text, anyelement) FROM mdm_administrator CASCADE;
        REVOKE ALL ON SCHEMA mdm_admin, mdm_steward FROM mdm_administrator CASCADE;
        SET ROLE mdm_helper_owner;
        REVOKE ALL ON SCHEMA mdm_admin, mdm_steward FROM mdm_administrator CASCADE;
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
    "pg_restore -l /tmp/foundation.dump | grep -v -E 'pgtrickle_changes|TABLE DATA pgtrickle |TRIGGER public .* pg_trickle_cdc_' > /tmp/foundation.list"
docker exec "$container" pg_restore -v -U postgres -d restored \
    --use-list=/tmp/foundation.list /tmp/foundation.dump >/dev/null
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d restored \
    -v helper_owner=mdm_helper_owner -f /sql/configure_helper.sql >/dev/null
restored_policy_digest=$(docker exec "$container" psql -X -At -U postgres -d restored -c "SELECT md5(COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) - 'last_observed_at' ORDER BY c.case_key) FROM mdm_steward.policy_cases_v1 c WHERE c.entity_name = 'policy_qualification'), '[]'::jsonb)::text)")
if [[ "$original_policy_digest" != "$restored_policy_digest" ]]; then
    echo "FAIL: restored policy case digest changed: $restored_policy_digest != $original_policy_digest" >&2
    exit 1
fi
restored_artifacts=$(docker exec "$container" psql -X -At -U postgres -d restored -c "$artifact_query")
if [[ "$original_artifacts" != "$restored_artifacts" ]]; then
    echo 'FAIL: definition artifacts changed during logical restore' >&2
    exit 1
fi
restored_source_identities=$(docker exec "$container" psql -X -At -U postgres -d restored -c "$source_identity_query")
if [[ "$original_source_identities" != "$restored_source_identities" ]]; then
    echo 'FAIL: source identities or records changed during logical restore' >&2
    exit 1
fi
docker exec "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d restored \
    -v original_source_oid="$original_source_oid" -v original_role_oid="$original_role_oid" \
    -v original_operations="$original_operations" -v original_policy_digest="$original_policy_digest" \
    -v original_policy_max="$original_policy_max" -v restore_issue_key="$restore_issue_key" \
    -f /tests/restore.sql

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
# The clone adds one isolation-only row on top of the restored records.
if [[ "$clone_sources" != "$((restored_sources + 1))" ]]; then
    echo "FAIL: clone isolation has $clone_sources active source records, expected $((restored_sources + 1))" >&2
    exit 1
fi

docker exec -i "$container" psql -X -qAt -v ON_ERROR_STOP=1 -U postgres -d foundation \
    -f /tests/operating_envelope.sql > "$work_dir/database-envelope.json"
python3 "$repo_root/scripts/record_e2e_evidence.py" \
    "$container" "$image" "$source_revision" \
    "$work_dir/database-envelope.json" "$qualification_json" "$e2e_log"

echo 'PASS: installation, Graph V1 admission, authorization, definition history, concurrency, and restore/rebind'
echo 'PASS: resolver-limit rollback and retry, backup/restore, and clone isolation'
echo 'PASS: physical backup recovery with populated and pending graph state'
echo 'PASS: candidate AUTO/FULL exact-row comparisons, FULL source oracle, and reported node strategies'
echo 'PASS: source writes after a returned boundary remain pending for the next refresh'
echo 'PASS: Delta V1 consumer registration, resnapshot, acknowledgement, and observability'
echo 'PASS: compiler v9 to v10 canary adoption preserves the complete publication'
echo 'PASS: affected-resolution qualification matches full rebuilds at 128 and 2,048 records'
echo 'PASS: decision and golden-override intervals use affected resolution or the invalidation fallback'
