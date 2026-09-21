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
    delta_rows integer NOT NULL,
    PRIMARY KEY (population, sample)
);
GRANT INSERT, SELECT ON public.incremental_qualification_results TO mdm_administrator;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
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
END
$block$;
RESET ROLE;

\connect foundation postgres
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
    ), '[]'::jsonb)
)
$function$;
REVOKE ALL ON FUNCTION public.incremental_qualification_publication() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.incremental_qualification_publication() TO mdm_administrator;

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
    actual jsonb;
    expected jsonb;
BEGIN
    UPDATE public.incremental_qualification_source
       SET display_name = 'Qualification ' || sample_population || '-' || sample_number,
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
        'entity_name', result->'entity_name',
        'resolver_fallback_reason', result->'resolver_fallback_reason',
        'resolver_strategy', result->'resolver_strategy',
        'unexpected_full_fallbacks', result->'unexpected_full_fallbacks');
    expected := '{
        "affected_components":128,
        "affected_records":128,
        "changed":true,
        "delta_lag":0,
        "entity_name":"incremental_qualification",
        "resolver_fallback_reason":"delta_full_invalidation",
        "resolver_strategy":"full",
        "unexpected_full_fallbacks":[]
    }'::jsonb;
    expected := pg_catalog.jsonb_set(expected, '{affected_components}', pg_catalog.to_jsonb(sample_population));
    expected := pg_catalog.jsonb_set(expected, '{affected_records}', pg_catalog.to_jsonb(sample_population));
    IF actual IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'incremental qualification sample mismatch: expected %, actual %',
            expected, actual;
    END IF;

    before_rebuild := public.incremental_qualification_publication();
    rebuilt := mdm_admin.rebuild('incremental_qualification', 'ALLOW');
    after_rebuild := public.incremental_qualification_publication();
    IF rebuilt->>'changed' IS DISTINCT FROM 'false'
       OR rebuilt->>'publication_revision' IS DISTINCT FROM result->>'publication_revision'
       OR after_rebuild IS DISTINCT FROM before_rebuild THEN
        RAISE EXCEPTION 'full rebuild changed qualified publication: refresh %, rebuild %, before %, after %',
            result, rebuilt, before_rebuild, after_rebuild;
    END IF;

    INSERT INTO public.incremental_qualification_results (
        population, sample, measured, elapsed_ms, graph_refresh_ms,
        mdm_resolution_ms, publication_ms, delta_rows)
    VALUES (
        sample_population,
        sample_number,
        is_measured,
        elapsed,
        (result #>> '{stage_timings_ms,graph_refresh}')::bigint,
        (result #>> '{stage_timings_ms,mdm_resolution}')::bigint,
        (result #>> '{stage_timings_ms,publication}')::bigint,
        (result->>'delta_row_count')::integer);
END
$function$;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $block$
BEGIN
    FOR sample_number IN 1..35 LOOP
        PERFORM public.run_incremental_qualification_sample(128, sample_number, sample_number > 5);
    END LOOP;
END
$block$;

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
    IF result->>'resolver_strategy' IS DISTINCT FROM 'full'
       OR result->>'resolver_fallback_reason' IS DISTINCT FROM 'delta_full_invalidation'
       OR (result->>'affected_records')::integer <> 2048
       OR (result->>'affected_components')::integer <> 2048
       OR result->>'delta_lag' IS DISTINCT FROM '0' THEN
        RAISE EXCEPTION 'qualification population growth mismatch: %', result;
    END IF;
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

SELECT pg_catalog.jsonb_build_object(
    'schema_version', 1,
    'pg_trickle_version', (
        SELECT extversion FROM pg_catalog.pg_extension WHERE extname = 'pg_trickle'),
    'equivalence_checks', (SELECT count(*) FROM public.incremental_qualification_results),
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
                       'delta_rows', pg_catalog.jsonb_build_object(
                           'min', min(delta_rows) FILTER (WHERE measured),
                           'max', max(delta_rows) FILTER (WHERE measured))) AS summary
              FROM public.incremental_qualification_results
             GROUP BY population
        ) AS grouped),
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
DROP FUNCTION public.run_incremental_qualification_sample(integer, integer, boolean);
DROP FUNCTION public.incremental_qualification_publication();
DROP TABLE public.incremental_qualification_results;
DROP TABLE public.incremental_qualification_source;
