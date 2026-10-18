\set ON_ERROR_STOP on
\pset tuples_only on
\pset format unaligned
\set QUIET on

CREATE TABLE public.incremental_qualification_source (
    id bigint PRIMARY KEY,
    display_name text NOT NULL,
    email_address text,
    updated_at timestamptz NOT NULL
);
INSERT INTO public.incremental_qualification_source
SELECT id,
       'Qualification record ' || id,
       'qualification-' || id || '@example.test',
       pg_catalog.statement_timestamp()
FROM pg_catalog.generate_series(1, 128) AS id;
GRANT SELECT, INSERT, UPDATE, DELETE, MAINTAIN
ON public.incremental_qualification_source TO mdm_administrator;

CREATE TABLE public.incremental_qualification_results (
    population integer NOT NULL,
    sample integer NOT NULL,
    measured boolean NOT NULL,
    elapsed_ms numeric NOT NULL,
    graph_refresh_ms bigint NOT NULL,
    mdm_resolution_ms bigint NOT NULL,
    publication_ms bigint NOT NULL,
    stage_timings_ms jsonb NOT NULL,
    delta_rows integer NOT NULL,
    PRIMARY KEY (population, sample)
);
GRANT INSERT, SELECT ON public.incremental_qualification_results TO mdm_administrator;
CREATE TABLE public.incremental_qualification_workloads (
    workload text PRIMARY KEY,
    population integer NOT NULL,
    changed_records integer NOT NULL,
    elapsed_ms numeric NOT NULL,
    result jsonb NOT NULL
);
GRANT INSERT, SELECT ON public.incremental_qualification_workloads TO mdm_administrator;
-- AUTO can choose FULL from observed timings; this fixture qualifies differential execution.
ALTER ROLE mdm_test_login SET pg_trickle.refresh_strategy = 'differential';
ALTER ROLE mdm_test_login SET session_preload_libraries = 'auto_explain';
ALTER ROLE mdm_test_login SET auto_explain.log_min_duration = '0';
ALTER ROLE mdm_test_login SET auto_explain.log_nested_statements = 'on';
ALTER ROLE mdm_test_login SET auto_explain.log_analyze = 'on';
ALTER ROLE mdm_test_login SET auto_explain.log_timing = 'off';

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
DO $block$
DECLARE
    created record;
    result jsonb;
    actual jsonb;
    expected jsonb;
BEGIN
    SELECT * INTO STRICT created
    FROM mdm.create(
        pg_catalog.jsonb_set(
            pg_catalog.jsonb_set(
                mdm.describe('customer', 'definition'),
                '{name}',
                pg_catalog.to_jsonb('incremental_qualification'::text)),
            '{sources,0,relation}',
            pg_catalog.to_jsonb('public.incremental_qualification_source'::text)));
    result := mdm.refresh('incremental_qualification', 'ALLOW');
    actual := pg_catalog.jsonb_build_object(
        'created', pg_catalog.jsonb_build_object(
            'changed', created.changed,
            'desired_version', created.desired_version),
        'refresh', pg_catalog.jsonb_build_object(
            'active_records', result->'active_records',
            'affected_components', result->'affected_components',
            'affected_records', result->'affected_records',
            'changed', result->'changed',
            'delta_lag', result->'delta_lag',
            'entity_name', result->'entity_name',
            'identities', result->'identities',
            'publication_revision', result->'publication_revision',
            'resolver_fallback_reason', result->'resolver_fallback_reason',
            'resolver_strategy', result->'resolver_strategy',
            'unexpected_full_fallbacks', result->'unexpected_full_fallbacks'));
    expected := '{
        "created":{"changed":true,"desired_version":1},
        "refresh":{
            "active_records":128,
            "affected_components":128,
            "affected_records":128,
            "changed":true,
            "delta_lag":0,
            "entity_name":"incremental_qualification",
            "identities":128,
            "publication_revision":1,
            "resolver_fallback_reason":"initial_population",
            "resolver_strategy":"full",
            "unexpected_full_fallbacks":[]
        }
    }'::jsonb;
    IF actual IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'incremental qualification bootstrap mismatch: expected %, actual %',
            expected, actual;
    END IF;
    RAISE NOTICE 'spi trace population=128 pid=% result=%', pg_catalog.pg_backend_pid(), result;
    UPDATE public.incremental_qualification_source SET display_name = 'Qualification trace 128' WHERE id = 1;
    result := mdm.refresh('incremental_qualification', 'ALLOW');
    IF pg_catalog.jsonb_build_object('strategy', result->'resolver_strategy',
        'affected', result->'affected_records', 'lag', result->'delta_lag')
        IS DISTINCT FROM '{"strategy":"affected","affected":1,"lag":0}'::jsonb THEN
        RAISE EXCEPTION 'isolated 128-record source trace failed: %', result;
    END IF;
    PERFORM mdm.refresh('incremental_qualification', 'ALLOW');
END
$block$;
RESET ROLE;

\connect foundation postgres
ALTER ROLE mdm_test_login RESET session_preload_libraries;
ALTER ROLE mdm_test_login RESET auto_explain.log_min_duration;
ALTER ROLE mdm_test_login RESET auto_explain.log_nested_statements;
ALTER ROLE mdm_test_login RESET auto_explain.log_analyze;
ALTER ROLE mdm_test_login RESET auto_explain.log_timing;
CREATE FUNCTION public.incremental_qualification_publication()
RETURNS jsonb
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog
AS $function$
SELECT pg_catalog.jsonb_build_object(
    'entities', COALESCE((
        SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY pg_catalog.to_jsonb(r)::text)
        FROM mdm_out.incremental_qualification AS r
    ), '[]'::jsonb),
    'members', COALESCE((
        SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY pg_catalog.to_jsonb(r)::text)
        FROM mdm_out.incremental_qualification_members AS r
    ), '[]'::jsonb),
    'reviews', COALESCE((
        SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY pg_catalog.to_jsonb(r)::text)
        FROM mdm_out.incremental_qualification_review AS r
    ), '[]'::jsonb),
    'durable', (
        SELECT pg_catalog.jsonb_build_object(
            'subject', (SELECT pg_catalog.jsonb_build_object('kind', 'source_record', 'id', r.source_record_id)
                        FROM mdm_internal.source_records r
                        JOIN mdm_internal.source_identities s
                          ON s.entity_id = r.entity_id AND s.source_identity_id = r.source_identity_id
                        WHERE r.entity_id = e.entity_id
                          AND r.source_record_key = pgtrickle.encode_row_id_v2(
                              'SCAN_KEY', ROW(e.entity_id, s.source_identity_id, 1::bigint))),
            'registry', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.mdm_id) FROM mdm_internal.identity_registry r WHERE r.entity_id = e.entity_id),
            'memberships', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.source_record_id) FROM mdm_internal.memberships r WHERE r.entity_id = e.entity_id),
            'aliases', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.alias_mdm_id) FROM mdm_internal.identity_aliases r WHERE r.entity_id = e.entity_id),
            'splits', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.parent_mdm_id, r.publication_revision, r.child_mdm_id) FROM mdm_internal.identity_splits r WHERE r.entity_id = e.entity_id),
            'golden', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.mdm_id, r.field_name) FROM mdm_internal.golden_provenance r WHERE r.entity_id = e.entity_id AND r.publication_revision = e.publication_revision),
            'reviews', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.review_id) FROM mdm_internal.reviews r WHERE r.entity_id = e.entity_id),
            'facts', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.fact_number) FROM mdm_internal.resolution_facts r WHERE r.entity_id = e.entity_id AND r.publication_revision = e.publication_revision),
            'result_digest', (SELECT pg_catalog.encode(p.result_digest, 'hex') FROM mdm_internal.publications p WHERE p.entity_id = e.entity_id AND p.publication_revision = e.publication_revision)
        )
        FROM mdm_internal.entities e WHERE e.entity_name = 'incremental_qualification'
    )
)
$function$;
REVOKE ALL ON FUNCTION public.incremental_qualification_publication() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.incremental_qualification_publication() TO mdm_administrator;

CREATE FUNCTION public.incremental_qualification_explanations(subject jsonb)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $function$
DECLARE
    historical jsonb;
BEGIN
    BEGIN
        historical := mdm.explain('incremental_qualification', subject, 1, 100);
    EXCEPTION WHEN SQLSTATE 'XX000' THEN
        IF SQLERRM <> 'MDM_EXPLANATION_NOT_RETAINED: explanation is not retained: 1' THEN
            RAISE;
        END IF;
        historical := pg_catalog.jsonb_build_object('error', SQLERRM);
    END;
    RETURN pg_catalog.jsonb_build_array(
        mdm.explain('incremental_qualification', subject, NULL, 100), historical);
END
$function$;

CREATE FUNCTION public.run_incremental_qualification_sample(
    sample_population integer,
    sample_number integer,
    is_measured boolean
)
RETURNS void
LANGUAGE plpgsql
SET search_path = pg_catalog, public
AS $function$
DECLARE
    started timestamptz;
    elapsed numeric;
    result jsonb;
    rebuilt jsonb;
    before_rebuild jsonb;
    after_rebuild jsonb;
    subject jsonb;
    before_explanations jsonb;
    after_explanations jsonb;
    actual jsonb;
    expected jsonb;
BEGIN
    UPDATE public.incremental_qualification_source
       SET display_name = 'Qualification ' || sample_population || '-' || sample_number,
           email_address = 'qualification-' || sample_population || '-' || sample_number || '@example.test',
           updated_at = pg_catalog.clock_timestamp()
     WHERE id = 1;
    started := pg_catalog.clock_timestamp();
    result := mdm.refresh('incremental_qualification', 'ALLOW');
    elapsed := extract(epoch FROM pg_catalog.clock_timestamp() - started) * 1000;
    actual := pg_catalog.jsonb_build_object(
        'affected_components', result->'affected_components',
        'affected_records', result->'affected_records',
        'changed', result->'changed',
        'delta_lag', result->'delta_lag',
        'delta_row_count', result->'delta_row_count',
        'entity_name', result->'entity_name',
        'resolver_fallback_reason', result->'resolver_fallback_reason',
        'resolver_strategy', result->'resolver_strategy',
        'unexpected_full_fallbacks', result->'unexpected_full_fallbacks');
    expected := '{
        "affected_components":1,
        "affected_records":1,
        "changed":true,
        "delta_lag":0,
        "delta_row_count":2,
        "entity_name":"incremental_qualification",
        "resolver_fallback_reason":null,
        "resolver_strategy":"affected",
        "unexpected_full_fallbacks":[]
    }'::jsonb;
    IF actual IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'incremental qualification sample mismatch: expected %, actual %',
            expected, actual;
    END IF;

    before_rebuild := public.incremental_qualification_publication();
    subject := before_rebuild #> '{durable,subject}';
    before_explanations := public.incremental_qualification_explanations(subject);
    rebuilt := mdm_admin.rebuild('incremental_qualification', 'ALLOW');
    after_rebuild := public.incremental_qualification_publication();
    after_explanations := public.incremental_qualification_explanations(subject);
    IF rebuilt->>'changed' IS DISTINCT FROM 'false'
       OR rebuilt->>'publication_revision' IS DISTINCT FROM result->>'publication_revision'
       OR after_rebuild IS DISTINCT FROM before_rebuild
       OR after_explanations IS DISTINCT FROM before_explanations THEN
        RAISE EXCEPTION 'full rebuild changed qualified publication: refresh %, rebuild %, before %, after %',
            result, rebuilt, before_rebuild, after_rebuild;
    END IF;

    INSERT INTO public.incremental_qualification_results (
        population, sample, measured, elapsed_ms, graph_refresh_ms,
        mdm_resolution_ms, publication_ms, stage_timings_ms, delta_rows)
    VALUES (
        sample_population,
        sample_number,
        is_measured,
        elapsed,
        (result #>> '{stage_timings_ms,graph_refresh}')::bigint,
        (result #>> '{stage_timings_ms,mdm_resolution}')::bigint,
        (result #>> '{stage_timings_ms,publication}')::bigint,
        result->'stage_timings_ms' || pg_catalog.jsonb_build_object(
            'full_evaluation_state_loading', rebuilt #> '{stage_timings_ms,evaluation_state_loading}'),
        (result->>'delta_row_count')::integer);
END
$function$;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SET mdm.trace_producer_artifact_sha256 = :'producer_artifact_sha256';
DO $block$
BEGIN
    FOR sample_number IN 1..35 LOOP
        PERFORM public.run_incremental_qualification_sample(128, sample_number, sample_number > 5);
    END LOOP;
END
$block$;
RESET ROLE;

\connect foundation postgres
ALTER ROLE mdm_test_login SET session_preload_libraries = 'auto_explain';
ALTER ROLE mdm_test_login SET auto_explain.log_min_duration = '0';
ALTER ROLE mdm_test_login SET auto_explain.log_nested_statements = 'on';
ALTER ROLE mdm_test_login SET auto_explain.log_analyze = 'on';
ALTER ROLE mdm_test_login SET auto_explain.log_timing = 'off';

\connect foundation mdm_test_login
SET ROLE mdm_administrator;

INSERT INTO public.incremental_qualification_source
SELECT id,
       'Qualification record ' || id,
       'qualification-' || id || '@example.test',
       pg_catalog.statement_timestamp()
FROM pg_catalog.generate_series(129, 2048) AS id;
DO $block$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('incremental_qualification', 'ALLOW');
    IF result->>'resolver_strategy' IS DISTINCT FROM 'affected'
       OR result->>'resolver_fallback_reason' IS NOT NULL
       OR (result->>'affected_records')::integer <> 1920
       OR (result->>'affected_components')::integer <> 1920
       OR result->>'delta_lag' IS DISTINCT FROM '0' THEN
        RAISE EXCEPTION 'qualification population growth mismatch: %', result;
    END IF;
    RAISE NOTICE 'spi trace population=2048 pid=% result=%', pg_catalog.pg_backend_pid(), result;
    UPDATE public.incremental_qualification_source SET display_name = 'Qualification trace 2048' WHERE id = 1;
    result := mdm.refresh('incremental_qualification', 'ALLOW');
    IF pg_catalog.jsonb_build_object('strategy', result->'resolver_strategy',
        'affected', result->'affected_records', 'lag', result->'delta_lag')
        IS DISTINCT FROM '{"strategy":"affected","affected":1,"lag":0}'::jsonb THEN
        RAISE EXCEPTION 'isolated 2048-record source trace failed: %', result;
    END IF;
    PERFORM mdm.refresh('incremental_qualification', 'ALLOW');
END
$block$;
RESET ROLE;

\connect foundation postgres
ALTER ROLE mdm_test_login RESET session_preload_libraries;
ALTER ROLE mdm_test_login RESET auto_explain.log_min_duration;
ALTER ROLE mdm_test_login RESET auto_explain.log_nested_statements;
ALTER ROLE mdm_test_login RESET auto_explain.log_analyze;
ALTER ROLE mdm_test_login RESET auto_explain.log_timing;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SET mdm.trace_producer_artifact_sha256 = :'producer_artifact_sha256';
DO $block$
BEGIN
    FOR sample_number IN 1..35 LOOP
        PERFORM public.run_incremental_qualification_sample(2048, sample_number, sample_number > 5);
    END LOOP;
END
$block$;
RESET ROLE;

\connect foundation postgres
DO $block$
DECLARE actual jsonb;
DECLARE expected jsonb := '{
    "equivalence_checks":70,
    "measured_samples":60,
    "populations":[128,2048],
    "warmup_samples":10
}'::jsonb;
BEGIN
    SELECT pg_catalog.jsonb_build_object(
        'equivalence_checks', count(*),
        'measured_samples', count(*) FILTER (WHERE measured),
        'populations', pg_catalog.jsonb_agg(DISTINCT population ORDER BY population),
        'warmup_samples', count(*) FILTER (WHERE NOT measured))
      INTO actual
      FROM public.incremental_qualification_results;
    IF actual IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'incremental qualification coverage mismatch: expected %, actual %',
            expected, actual;
    END IF;
END
$block$;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SET mdm.trace_producer_artifact_sha256 = :'producer_artifact_sha256';
DO $block$
DECLARE
    workload text;
    population integer;
    changed_records integer;
    started timestamptz;
    elapsed_ms numeric;
    result jsonb;
    actual jsonb;
    expected jsonb;
    before_rebuild jsonb;
    rebuilt jsonb;
BEGIN
    FOREACH workload IN ARRAY ARRAY['no_change', 'larger', 'dense'] LOOP
        IF workload = 'no_change' THEN
            population := 2048;
            changed_records := 0;
            expected := '{"changed":false,"resolver_strategy":"skipped","affected_records":0,"affected_components":0,"delta_row_count":0}'::jsonb;
        ELSIF workload = 'larger' THEN
            INSERT INTO public.incremental_qualification_source
            SELECT id, 'Qualification record ' || id, 'qualification-' || id || '@example.test',
                   pg_catalog.statement_timestamp()
            FROM pg_catalog.generate_series(2049, 8192) AS id;
            PERFORM mdm.refresh('incremental_qualification', 'ALLOW');
            UPDATE public.incremental_qualification_source
            SET display_name = 'Larger isolated update', email_address = 'larger-isolated@example.test',
                updated_at = pg_catalog.clock_timestamp() WHERE id = 1;
            population := 8192;
            changed_records := 1;
            expected := '{"changed":true,"resolver_strategy":"affected","affected_records":1,"affected_components":1,"delta_row_count":2}'::jsonb;
        ELSE
            UPDATE public.incremental_qualification_source
            SET email_address = 'dense@example.test', updated_at = pg_catalog.clock_timestamp()
            WHERE id BETWEEN 1 AND 32;
            population := 8192;
            changed_records := 32;
            -- Thirty-two equal emails independently imply 496 pairs and 64 golden-row changes.
            expected := '{"changed":true,"resolver_strategy":"affected","affected_records":32,"affected_components":1,"delta_row_count":560}'::jsonb;
        END IF;
        started := pg_catalog.clock_timestamp();
        result := mdm.refresh('incremental_qualification', 'ALLOW');
        elapsed_ms := extract(epoch FROM pg_catalog.clock_timestamp() - started) * 1000;
        SELECT pg_catalog.jsonb_object_agg(key, result->key) INTO actual
        FROM pg_catalog.jsonb_object_keys(expected) AS keys(key);
        IF actual IS DISTINCT FROM expected
           OR result->>'active_records' IS DISTINCT FROM population::text
           OR result->>'resolver_fallback_reason' IS NOT NULL
           OR result->>'delta_lag' IS DISTINCT FROM '0' THEN
            RAISE EXCEPTION 'additional workload % mismatch: expected %, actual %', workload, expected, result;
        END IF;
        before_rebuild := public.incremental_qualification_publication();
        rebuilt := mdm_admin.rebuild('incremental_qualification', 'ALLOW');
        IF rebuilt->>'changed' IS DISTINCT FROM 'false'
           OR public.incremental_qualification_publication() IS DISTINCT FROM before_rebuild THEN
            RAISE EXCEPTION 'additional workload % differs from full rebuild: %', workload, rebuilt;
        END IF;
        INSERT INTO public.incremental_qualification_workloads
        VALUES (workload, population, changed_records, elapsed_ms, result);
    END LOOP;
END
$block$;
RESET ROLE;
\connect foundation postgres

SELECT pg_catalog.jsonb_build_object(
    'schema_version', 1,
    'pg_trickle_version', (
        SELECT extversion FROM pg_catalog.pg_extension WHERE extname = 'pg_trickle'),
    'equivalence_checks', (SELECT count(*) FROM public.incremental_qualification_results),
    'samples', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(s) ORDER BY s.population, s.sample)
                FROM public.incremental_qualification_results s),
    'populations', (
        SELECT pg_catalog.jsonb_agg(summary ORDER BY population)
        FROM (
            SELECT population,
                   pg_catalog.jsonb_build_object(
                       'population', population,
                       'measured_samples', count(*) FILTER (WHERE measured),
                       'warmup_samples', count(*) FILTER (WHERE NOT measured),
                       'elapsed_ms', pg_catalog.jsonb_build_object(
                           'p50', pg_catalog.round(
                               (pg_catalog.percentile_cont(0.50) WITHIN GROUP (ORDER BY elapsed_ms)
                               FILTER (WHERE measured))::numeric, 3),
                           'p95', pg_catalog.round(
                               (pg_catalog.percentile_cont(0.95) WITHIN GROUP (ORDER BY elapsed_ms)
                               FILTER (WHERE measured))::numeric, 3),
                           'max', pg_catalog.round(max(elapsed_ms) FILTER (WHERE measured), 3)),
                       'graph_refresh_ms', pg_catalog.jsonb_build_object(
                           'p95', pg_catalog.percentile_cont(0.95) WITHIN GROUP (ORDER BY graph_refresh_ms)
                               FILTER (WHERE measured)),
                       'mdm_resolution_ms', pg_catalog.jsonb_build_object(
                           'p95', pg_catalog.percentile_cont(0.95) WITHIN GROUP (ORDER BY mdm_resolution_ms)
                               FILTER (WHERE measured)),
                       'publication_ms', pg_catalog.jsonb_build_object(
                           'p95', pg_catalog.percentile_cont(0.95) WITHIN GROUP (ORDER BY publication_ms)
                               FILTER (WHERE measured)),
                       'source_stage_timings_ms', (
                           SELECT pg_catalog.jsonb_object_agg(stage, timings)
                           FROM (
                               SELECT key AS stage, pg_catalog.jsonb_build_object(
                                   'p50', pg_catalog.percentile_cont(0.50) WITHIN GROUP (ORDER BY value::numeric),
                                   'p95', pg_catalog.percentile_cont(0.95) WITHIN GROUP (ORDER BY value::numeric),
                                   'max', max(value::numeric)) AS timings
                               FROM public.incremental_qualification_results samples
                               CROSS JOIN LATERAL pg_catalog.jsonb_each_text(samples.stage_timings_ms)
                               WHERE samples.population = r.population AND samples.measured
                                 AND key IN ('source_access', 'source_capture', 'source_scan',
                                     'source_sync', 'graph_helper_sync', 'end_to_end')
                               GROUP BY key
                           ) stages),
                       'delta_rows', pg_catalog.jsonb_build_object(
                           'min', min(delta_rows) FILTER (WHERE measured),
                           'max', max(delta_rows) FILTER (WHERE measured))) AS summary
              FROM public.incremental_qualification_results r
             GROUP BY population
        ) AS grouped),
    'additional_workloads', (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(w) ORDER BY w.workload) FROM public.incremental_qualification_workloads w),
    'scope', 'synthetic exact-email graph; p95 is observational and has no timing gate'
);

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $block$
BEGIN
    PERFORM mdm_admin.drop_entity('incremental_qualification', 'incremental_qualification');
END
$block$;
RESET ROLE;
\connect foundation postgres
ALTER ROLE mdm_test_login RESET pg_trickle.refresh_strategy;
DROP FUNCTION public.run_incremental_qualification_sample(integer, integer, boolean);
DROP FUNCTION public.incremental_qualification_explanations(jsonb);
DROP FUNCTION public.incremental_qualification_publication();
DROP TABLE public.incremental_qualification_results;
DROP TABLE public.incremental_qualification_workloads;
DROP TABLE public.incremental_qualification_source;
