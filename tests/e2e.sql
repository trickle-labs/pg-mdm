\set ON_ERROR_STOP on

CREATE DATABASE foundation;
\connect foundation postgres
CREATE EXTENSION pg_trickle VERSION '0.105.1';
CREATE EXTENSION pg_mdm VERSION '0.8.0';

DO $$
DECLARE
    schema_count integer;
BEGIN
    SELECT count(*) INTO schema_count
    FROM pg_catalog.pg_namespace
    WHERE nspname IN ('mdm', 'mdm_out', 'mdm_steward', 'mdm_admin', 'mdm_internal', 'mdm_graph');
    IF schema_count <> 6 THEN
        RAISE EXCEPTION 'expected six MDM schemas, found %', schema_count;
    END IF;
    IF pg_catalog.to_regclass('mdm_internal.operations') IS NULL THEN
        RAISE EXCEPTION 'operations table is missing';
    END IF;
    IF pg_catalog.to_regclass('mdm_internal.source_records') IS NULL THEN
        RAISE EXCEPTION 'source_records table is missing';
    END IF;
    IF pg_catalog.to_regtype('mdm_internal.normalized_value') IS NULL THEN
        RAISE EXCEPTION 'normalized_value type is missing';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_extension
        WHERE extname = 'pg_mdm' AND extversion = '0.8.0'
    ) THEN
        RAISE EXCEPTION 'pg_mdm 0.8.0 is not installed';
    END IF;
END
$$;
ALTER EXTENSION pg_mdm UPDATE TO '0.9.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.10.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.11.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.12.0';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extversion = '0.12.0')
       OR pg_catalog.to_regclass('mdm_internal.graph_bindings') IS NULL
       OR pg_catalog.to_regclass('mdm_internal.graph_members') IS NULL
       OR pg_catalog.to_regclass('mdm_internal.graph_bindings_entity_definition_generation') IS NULL
       OR pg_catalog.to_regclass('mdm_internal.graph_members_binding_ordinal') IS NULL
       OR pg_catalog.to_regclass('mdm_internal.operations_entity_started') IS NULL THEN
        RAISE EXCEPTION '0.8.0 to 0.12.0 upgrade did not complete';
    END IF;
END
$$;

\connect postgres postgres
CREATE DATABASE upgrade;
\connect upgrade postgres
CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm VERSION '0.1.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.2.0';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extversion = '0.2.0')
       OR pg_catalog.to_regclass('mdm_internal.entities') IS NULL
       OR pg_catalog.to_regclass('mdm_internal.definition_artifacts') IS NULL THEN
        RAISE EXCEPTION '0.1.0 to 0.2.0 upgrade did not install definition catalog';
    END IF;
END
$$;
ALTER EXTENSION pg_mdm UPDATE TO '0.3.0';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extversion = '0.3.0')
       OR pg_catalog.to_regclass('mdm_internal.source_records') IS NULL
       OR pg_catalog.to_regtype('mdm_internal.normalized_value') IS NULL THEN
        RAISE EXCEPTION '0.2.0 to 0.3.0 upgrade did not install v0.3 catalog';
    END IF;
END
$$;
ALTER EXTENSION pg_mdm UPDATE TO '0.4.0';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extversion = '0.4.0') THEN
        RAISE EXCEPTION '0.3.0 to 0.4.0 upgrade did not update extension version';
    END IF;
END
$$;
ALTER EXTENSION pg_mdm UPDATE TO '0.5.0';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extversion = '0.5.0')
       OR pg_catalog.to_regclass('mdm_internal.steward_decisions') IS NULL
       OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_attribute WHERE attrelid = 'mdm_internal.entities'::pg_catalog.regclass AND attname = 'decision_epoch') THEN
        RAISE EXCEPTION '0.4.0 to 0.5.0 upgrade did not install steward decisions';
    END IF;
END
$$;
ALTER EXTENSION pg_mdm UPDATE TO '0.6.0';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extversion = '0.6.0') THEN
        RAISE EXCEPTION '0.5.0 to 0.6.0 upgrade did not update extension version';
    END IF;
END
$$;
ALTER EXTENSION pg_mdm UPDATE TO '0.7.0';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extversion = '0.7.0')
       OR pg_catalog.to_regclass('mdm_internal.publications') IS NULL
       OR pg_catalog.to_regclass('mdm_internal.identity_registry') IS NULL THEN
        RAISE EXCEPTION '0.6.0 to 0.7.0 upgrade did not install v0.7 catalog';
    END IF;
END
$$;
ALTER EXTENSION pg_mdm UPDATE TO '0.8.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.9.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.10.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.12.0';

\connect postgres postgres
CREATE DATABASE upgrade_direct;
\connect upgrade_direct postgres
CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm VERSION '0.2.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.3.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.4.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.5.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.6.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.7.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.8.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.9.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.10.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.12.0';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extversion = '0.12.0')
       OR pg_catalog.to_regclass('mdm_internal.source_records') IS NULL
       OR pg_catalog.to_regclass('mdm_internal.publications') IS NULL
       OR pg_catalog.to_regclass('mdm_internal.graph_bindings') IS NULL
       OR pg_catalog.to_regclass('mdm_internal.operations_entity_started') IS NULL THEN
        RAISE EXCEPTION 'direct 0.2.0 to 0.12.0 upgrade did not complete';
    END IF;
END
$$;

\connect postgres postgres
CREATE DATABASE capability_errors;
\connect capability_errors postgres
CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm;
DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_internal.integration_capabilities()
        WHERE major_version = 1 AND minor_version = 0 AND enabled) <> 2 THEN
        RAISE EXCEPTION 'Graph V1 integration capabilities are not enabled';
    END IF;
END
$$;

CREATE FUNCTION public.assert_adapter_error(expected_code text)
RETURNS void
LANGUAGE plpgsql
AS $$
BEGIN
    BEGIN
        PERFORM mdm_internal.integration_capabilities();
        RAISE EXCEPTION 'adapter accepted invalid capability rows';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, expected_code) = 0 THEN
            RAISE;
        END IF;
    END;
END
$$;

CREATE OR REPLACE FUNCTION pgtrickle.integration_capabilities()
RETURNS TABLE (capability text, major_version smallint, minor_version smallint, enabled boolean, details jsonb)
LANGUAGE sql
AS $$ SELECT 'output_delta_consumer', 1::smallint, 0::smallint, false, '{}'::jsonb $$;
SELECT public.assert_adapter_error('MDM_PGT_CAPABILITY_MISSING');

CREATE OR REPLACE FUNCTION pgtrickle.integration_capabilities()
RETURNS TABLE (capability text, major_version smallint, minor_version smallint, enabled boolean, details jsonb)
LANGUAGE sql
AS $$
    VALUES
        ('external_graph_refresh', 1::smallint, 0::smallint, false, '{}'::jsonb),
        ('external_graph_refresh', 1::smallint, 0::smallint, false, '{}'::jsonb)
$$;
SELECT public.assert_adapter_error('MDM_PGT_CAPABILITY_INVALID');

CREATE OR REPLACE FUNCTION pgtrickle.integration_capabilities()
RETURNS TABLE (capability text, major_version smallint, minor_version smallint, enabled boolean, details jsonb)
LANGUAGE sql
AS $$ SELECT 'external_graph_refresh', 1::smallint, 0::smallint, false, '[]'::jsonb $$;
SELECT public.assert_adapter_error('MDM_PGT_CAPABILITY_INVALID');

CREATE OR REPLACE FUNCTION pgtrickle.integration_capabilities()
RETURNS TABLE (capability text, major_version smallint, minor_version smallint, enabled boolean, details jsonb)
LANGUAGE sql
AS $$ SELECT 'external_graph_refresh', 2::smallint, 0::smallint, true, '{}'::jsonb $$;
SELECT public.assert_adapter_error('MDM_PGT_CAPABILITY_VERSION');

CREATE OR REPLACE FUNCTION pgtrickle.integration_capabilities()
RETURNS TABLE (capability text, major_version smallint, minor_version smallint, enabled boolean, details jsonb)
LANGUAGE sql
AS $$ SELECT 'external_graph_refresh', 1::smallint, 0::smallint, false, '{}'::jsonb $$;
DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_internal.integration_capabilities()) <> 1 THEN
        RAISE EXCEPTION 'missing Delta V1 row was not accepted';
    END IF;
END
$$;

CREATE OR REPLACE FUNCTION pgtrickle.integration_capabilities()
RETURNS TABLE (capability text, major_version smallint, minor_version smallint, enabled boolean, details jsonb)
LANGUAGE sql
AS $$
    VALUES
        ('external_graph_refresh', 1::smallint, 0::smallint, false, '{}'::jsonb),
        ('future_capability', 9::smallint, 0::smallint, true, '{}'::jsonb)
$$;
DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_internal.integration_capabilities()) <> 1 THEN
        RAISE EXCEPTION 'unknown capability was not ignored';
    END IF;
END
$$;

\connect foundation postgres
DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_internal.integration_capabilities()
        WHERE major_version = 1 AND minor_version = 0 AND enabled) <> 2 THEN
        RAISE EXCEPTION 'baseline capabilities do not match';
    END IF;
    PERFORM mdm_internal.require_graph_v1();
END
$$;

CREATE ROLE mdm_helper_owner NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE mdm_administrator NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE mdm_configurator NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE mdm_steward_role NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE mdm_output_reader NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE mdm_explanation_reader NOLOGIN NOSUPERUSER NOBYPASSRLS;
CREATE ROLE mdm_bypass NOLOGIN NOSUPERUSER BYPASSRLS;
CREATE ROLE mdm_test_login LOGIN NOSUPERUSER NOBYPASSRLS;
GRANT mdm_administrator, mdm_configurator, mdm_steward_role, mdm_output_reader,
      mdm_explanation_reader, mdm_bypass TO mdm_test_login WITH SET TRUE, INHERIT FALSE;
GRANT USAGE ON SCHEMA mdm_admin TO mdm_administrator;
GRANT USAGE ON SCHEMA mdm TO mdm_administrator;
GRANT EXECUTE ON FUNCTION mdm_admin.verify_installation() TO mdm_administrator;
GRANT EXECUTE ON FUNCTION mdm_admin.drop_entity(text, text) TO mdm_administrator;

\connect postgres postgres
CREATE DATABASE graph_conformance;
\connect graph_conformance postgres
CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm;
CREATE TABLE public.mdm_graph_source (
    id integer PRIMARY KEY,
    owner_name text NOT NULL,
    value text NOT NULL
);
INSERT INTO public.mdm_graph_source VALUES
    (1, 'mdm_administrator', 'visible'),
    (2, 'another_role', 'hidden');
ALTER TABLE public.mdm_graph_source ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.mdm_graph_source FORCE ROW LEVEL SECURITY;
ALTER TABLE public.mdm_graph_source OWNER TO mdm_administrator;
CREATE POLICY mdm_graph_owner_rows ON public.mdm_graph_source
    TO mdm_administrator USING (owner_name = current_user);
CREATE TABLE public.mdm_graph_publication (
    graph_refresh_id bigint NOT NULL,
    source_boundary_digest bytea NOT NULL
);
GRANT USAGE ON SCHEMA pgtrickle TO mdm_administrator;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA pgtrickle TO mdm_administrator;
GRANT CREATE ON SCHEMA public TO mdm_administrator;
GRANT SELECT, MAINTAIN ON public.mdm_graph_source TO mdm_administrator;
GRANT SELECT, INSERT ON public.mdm_graph_publication TO mdm_administrator;
CREATE TABLE public.mdm_graph_diff_source (
    id integer PRIMARY KEY,
    owner_name text NOT NULL,
    value text NOT NULL
);
INSERT INTO public.mdm_graph_diff_source VALUES
    (1, 'mdm_administrator', 'visible');
GRANT SELECT, INSERT, UPDATE, DELETE, MAINTAIN ON public.mdm_graph_diff_source TO mdm_administrator;
CREATE TABLE public.mdm_candidate_source (
    source_record_id uuid NOT NULL,
    field_name text NOT NULL,
    state text NOT NULL,
    canonical_bytes bytea,
    source_sort_key bytea NOT NULL,
    PRIMARY KEY (source_record_id, field_name)
);
GRANT SELECT, INSERT, UPDATE, DELETE, MAINTAIN ON public.mdm_candidate_source TO mdm_administrator;

SET SESSION AUTHORIZATION mdm_test_login;
SET ROLE mdm_administrator;
SELECT pgtrickle.create_stream_table(
    name => 'public.mdm_graph_probe',
    query => 'SELECT id, owner_name, value FROM public.mdm_graph_source',
    schedule => '1h',
    refresh_mode => 'AUTO',
    initialize => false,
    orchestration_mode => 'EXTERNAL'
);
SELECT pgtrickle.create_stream_table(
    name => 'public.mdm_graph_probe_reference',
    query => 'SELECT id, owner_name, value FROM public.mdm_graph_source',
    schedule => '1h',
    refresh_mode => 'FULL',
    initialize => false,
    orchestration_mode => 'EXTERNAL'
);
SELECT pgtrickle.create_stream_table(
    name => 'public.mdm_graph_diff_probe',
    query => 'SELECT id, owner_name, value FROM public.mdm_graph_diff_source',
    schedule => '1h',
    refresh_mode => 'AUTO',
    initialize => false,
    orchestration_mode => 'EXTERNAL'
);
SELECT pgtrickle.create_stream_table(
    name => 'public.mdm_graph_diff_probe_reference',
    query => 'SELECT id, owner_name, value FROM public.mdm_graph_diff_source',
    schedule => '1h',
    refresh_mode => 'FULL',
    initialize => false,
    orchestration_mode => 'EXTERNAL'
);
SELECT pgtrickle.create_stream_table(
    name => 'public.mdm_candidate_blocks_auto',
    query => $query$SELECT 'email'::text AS channel_id, canonical_bytes AS block_key, source_record_id, source_sort_key
        FROM public.mdm_candidate_source
        WHERE field_name = 'email' AND state = 'value' AND canonical_bytes IS NOT NULL$query$,
    schedule => '1h', refresh_mode => 'AUTO', initialize => false,
    orchestration_mode => 'EXTERNAL'
);
SELECT pgtrickle.create_stream_table(
    name => 'public.mdm_candidate_blocks_full',
    query => $query$SELECT 'email'::text AS channel_id, canonical_bytes AS block_key, source_record_id, source_sort_key
        FROM public.mdm_candidate_source
        WHERE field_name = 'email' AND state = 'value' AND canonical_bytes IS NOT NULL$query$,
    schedule => '1h', refresh_mode => 'FULL', initialize => false,
    orchestration_mode => 'EXTERNAL'
);
SELECT pgtrickle.create_stream_table(
    name => 'public.mdm_candidate_pairs_auto',
    query => $query$WITH blocks AS (
            SELECT 'email'::text AS channel_id, canonical_bytes AS block_key, source_record_id, source_sort_key
            FROM public.mdm_candidate_source
            WHERE field_name = 'email' AND state = 'value' AND canonical_bytes IS NOT NULL
        ), stats AS (
            SELECT channel_id, block_key, count(*)::bigint AS block_records
            FROM blocks GROUP BY channel_id, block_key
        )
        SELECT l.source_record_id AS left_source_record_id,
               r.source_record_id AS right_source_record_id,
               l.source_sort_key AS left_sort_key,
               r.source_sort_key AS right_sort_key
        FROM blocks l
        JOIN stats s ON s.channel_id = l.channel_id AND s.block_key = l.block_key
        JOIN blocks r ON r.channel_id = l.channel_id AND r.block_key = l.block_key
            AND l.source_sort_key < r.source_sort_key
        WHERE s.block_records <= 100$query$,
    schedule => '1h', refresh_mode => 'AUTO', initialize => false,
    orchestration_mode => 'EXTERNAL'
);
SELECT pgtrickle.create_stream_table(
    name => 'public.mdm_candidate_pairs_full',
    query => $query$WITH blocks AS (
            SELECT 'email'::text AS channel_id, canonical_bytes AS block_key, source_record_id, source_sort_key
            FROM public.mdm_candidate_source
            WHERE field_name = 'email' AND state = 'value' AND canonical_bytes IS NOT NULL
        ), stats AS (
            SELECT channel_id, block_key, count(*)::bigint AS block_records
            FROM blocks GROUP BY channel_id, block_key
        )
        SELECT l.source_record_id AS left_source_record_id,
               r.source_record_id AS right_source_record_id,
               l.source_sort_key AS left_sort_key,
               r.source_sort_key AS right_sort_key
        FROM blocks l
        JOIN stats s ON s.channel_id = l.channel_id AND s.block_key = l.block_key
        JOIN blocks r ON r.channel_id = l.channel_id AND r.block_key = l.block_key
            AND l.source_sort_key < r.source_sort_key
        WHERE s.block_records <= 100$query$,
    schedule => '1h', refresh_mode => 'FULL', initialize => false,
    orchestration_mode => 'EXTERNAL'
);
CREATE FUNCTION public.refresh_mdm_graph(roots regclass[])
RETURNS jsonb
LANGUAGE plpgsql
AS $$
DECLARE
    expected_digest bytea;
    refreshed record;
BEGIN
    SELECT graph_digest INTO STRICT expected_digest
    FROM pgtrickle.graph_contract(roots);
    SELECT * INTO STRICT refreshed
    FROM pgtrickle.refresh_graph_strict(roots, expected_digest, 'ALLOW');
    RETURN pg_catalog.to_jsonb(refreshed);
END
$$;
-- AUTO is diagnostic here: report divergence and keep the production graph on FULL until every history agrees.
CREATE FUNCTION public.check_mdm_candidate_probes(stage text)
RETURNS jsonb
LANGUAGE plpgsql
AS $$
DECLARE
    roots regclass[] := ARRAY[
        'public.mdm_candidate_blocks_auto'::regclass,
        'public.mdm_candidate_blocks_full'::regclass,
        'public.mdm_candidate_pairs_auto'::regclass,
        'public.mdm_candidate_pairs_full'::regclass];
    refreshed jsonb;
    block_rows_equal boolean;
    pair_rows_equal boolean;
    full_rows_valid boolean;
BEGIN
    refreshed := public.refresh_mdm_graph(roots);
    SELECT NOT EXISTS (
               SELECT channel_id, block_key, source_record_id, source_sort_key
               FROM public.mdm_candidate_blocks_auto
               EXCEPT ALL
               SELECT channel_id, block_key, source_record_id, source_sort_key
               FROM public.mdm_candidate_blocks_full)
       AND NOT EXISTS (
               SELECT channel_id, block_key, source_record_id, source_sort_key
               FROM public.mdm_candidate_blocks_full
               EXCEPT ALL
               SELECT channel_id, block_key, source_record_id, source_sort_key
               FROM public.mdm_candidate_blocks_auto)
      INTO block_rows_equal;
    SELECT NOT EXISTS (
               SELECT left_source_record_id, right_source_record_id, left_sort_key, right_sort_key
               FROM public.mdm_candidate_pairs_auto
               EXCEPT ALL
               SELECT left_source_record_id, right_source_record_id, left_sort_key, right_sort_key
               FROM public.mdm_candidate_pairs_full)
       AND NOT EXISTS (
               SELECT left_source_record_id, right_source_record_id, left_sort_key, right_sort_key
               FROM public.mdm_candidate_pairs_full
               EXCEPT ALL
               SELECT left_source_record_id, right_source_record_id, left_sort_key, right_sort_key
               FROM public.mdm_candidate_pairs_auto)
      INTO pair_rows_equal;
    WITH blocks AS (
             SELECT canonical_bytes AS block_key, source_record_id, source_sort_key
             FROM public.mdm_candidate_source
             WHERE field_name = 'email' AND state = 'value' AND canonical_bytes IS NOT NULL
         ), stats AS (
             SELECT block_key, count(*)::bigint AS block_records
             FROM blocks GROUP BY block_key
         )
    SELECT NOT EXISTS (
               SELECT channel_id, block_key, source_record_id, source_sort_key
               FROM public.mdm_candidate_blocks_full
               EXCEPT ALL
               SELECT 'email'::text, canonical_bytes, source_record_id, source_sort_key
               FROM public.mdm_candidate_source
               WHERE field_name = 'email' AND state = 'value' AND canonical_bytes IS NOT NULL)
       AND NOT EXISTS (
               SELECT 'email'::text, canonical_bytes, source_record_id, source_sort_key
               FROM public.mdm_candidate_source
               WHERE field_name = 'email' AND state = 'value' AND canonical_bytes IS NOT NULL
               EXCEPT ALL
               SELECT channel_id, block_key, source_record_id, source_sort_key
               FROM public.mdm_candidate_blocks_full)
       AND NOT EXISTS (
               SELECT left_source_record_id, right_source_record_id, left_sort_key, right_sort_key
               FROM public.mdm_candidate_pairs_full
               EXCEPT ALL
               SELECT l.source_record_id, r.source_record_id, l.source_sort_key, r.source_sort_key
               FROM blocks l
               JOIN stats s USING (block_key)
               JOIN blocks r USING (block_key)
               WHERE l.source_sort_key < r.source_sort_key AND s.block_records <= 100)
       AND NOT EXISTS (
               SELECT l.source_record_id, r.source_record_id, l.source_sort_key, r.source_sort_key
               FROM blocks l
               JOIN stats s USING (block_key)
               JOIN blocks r USING (block_key)
               WHERE l.source_sort_key < r.source_sort_key AND s.block_records <= 100
               EXCEPT ALL
               SELECT left_source_record_id, right_source_record_id, left_sort_key, right_sort_key
               FROM public.mdm_candidate_pairs_full)
      INTO full_rows_valid;
    IF NOT full_rows_valid
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_candidate_blocks_auto') IS NULL
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_candidate_blocks_full') IS DISTINCT FROM 'FULL'
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_candidate_pairs_auto') IS NULL
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_candidate_pairs_full') IS DISTINCT FROM 'FULL' THEN
        RAISE EXCEPTION 'candidate FULL oracle or reported strategy failed at %: %', stage, refreshed;
    END IF;
    RAISE NOTICE 'candidate probes at %: blocks AUTO=%, FULL=%, equal=%; pairs AUTO=%, FULL=%, equal=%',
        stage,
        public.mdm_graph_action(refreshed->'node_results', 'public.mdm_candidate_blocks_auto'),
        public.mdm_graph_action(refreshed->'node_results', 'public.mdm_candidate_blocks_full'),
        block_rows_equal,
        public.mdm_graph_action(refreshed->'node_results', 'public.mdm_candidate_pairs_auto'),
        public.mdm_graph_action(refreshed->'node_results', 'public.mdm_candidate_pairs_full'),
        pair_rows_equal;
    RETURN refreshed || jsonb_build_object(
        'candidate_probe_rows_equal', jsonb_build_object(
            'blocks', block_rows_equal,
            'pairs', pair_rows_equal),
        'candidate_full_matches_source_oracle', full_rows_valid);
END
$$;
CREATE FUNCTION public.mdm_graph_action(results jsonb, node_identity text)
RETURNS text
LANGUAGE sql
IMMUTABLE
STRICT
AS $$
    SELECT value->>'action'
    FROM pg_catalog.jsonb_each(results)
    WHERE value->>'identity' = node_identity
$$;
DO $$
DECLARE stream_contract record;
DECLARE graph_contract record;
BEGIN
    SELECT * INTO STRICT stream_contract
      FROM pgtrickle.stream_table_contract('public.mdm_graph_probe'::regclass);
    IF stream_contract.contract_version <> 1
       OR octet_length(stream_contract.contract_digest) <> 32
       OR stream_contract.contract->>'orchestration_mode' <> 'EXTERNAL'
       OR stream_contract.contract #>> '{relation,owner}' <> 'mdm_administrator' THEN
        RAISE EXCEPTION 'unexpected stream contract: %', stream_contract.contract;
    END IF;
    SELECT * INTO STRICT graph_contract
      FROM pgtrickle.graph_contract(ARRAY['public.mdm_graph_probe'::regclass]);
    IF graph_contract.contract_version <> 1
       OR octet_length(graph_contract.graph_digest) <> 32
       OR NOT graph_contract.contract->'members' @> '[{"orchestration_mode":"EXTERNAL"}]'::jsonb THEN
        RAISE EXCEPTION 'unexpected graph contract: %', graph_contract.contract;
    END IF;
    SELECT * INTO STRICT stream_contract
      FROM pgtrickle.stream_table_contract('public.mdm_graph_probe_reference'::regclass);
    IF stream_contract.contract->>'refresh_mode' <> 'FULL' THEN
        RAISE EXCEPTION 'full reference graph is not pinned to FULL: %', stream_contract.contract;
    END IF;
END
$$;

BEGIN;
DO $$
DECLARE expected_digest bytea;
DECLARE refreshed record;
BEGIN
    SELECT graph_digest INTO STRICT expected_digest
      FROM pgtrickle.graph_contract(ARRAY['public.mdm_graph_probe'::regclass]);
    SELECT * INTO STRICT refreshed FROM pgtrickle.refresh_graph_strict(
        ARRAY['public.mdm_graph_probe'::regclass], expected_digest, 'ALLOW');
    IF refreshed.contract_version <> 1
       OR refreshed.source_boundary->>'completeness' <> 'PROVEN'
       OR octet_length(refreshed.source_boundary_digest) <> 32
       OR (SELECT count(*) FROM public.mdm_graph_probe) <> 1 THEN
        RAISE EXCEPTION 'unexpected graph refresh: %', row_to_json(refreshed);
    END IF;
    INSERT INTO public.mdm_graph_publication
    VALUES (refreshed.graph_refresh_id, refreshed.source_boundary_digest);
END
$$;
ROLLBACK;
DO $$
BEGIN
    IF (SELECT count(*) FROM public.mdm_graph_probe) <> 0
       OR (SELECT count(*) FROM public.mdm_graph_publication) <> 0 THEN
        RAISE EXCEPTION 'graph refresh and publication did not roll back together';
    END IF;
END
$$;

BEGIN;
DO $$
DECLARE expected_digest bytea;
DECLARE refreshed record;
BEGIN
    SELECT graph_digest INTO STRICT expected_digest
      FROM pgtrickle.graph_contract(ARRAY[
          'public.mdm_graph_probe'::regclass,
          'public.mdm_graph_probe_reference'::regclass]);
    SELECT * INTO STRICT refreshed FROM pgtrickle.refresh_graph_strict(
        ARRAY[
            'public.mdm_graph_probe'::regclass,
            'public.mdm_graph_probe_reference'::regclass], expected_digest, 'ALLOW');
    INSERT INTO public.mdm_graph_publication
    VALUES (refreshed.graph_refresh_id, refreshed.source_boundary_digest);
END
$$;
COMMIT;
DO $$
BEGIN
    IF (SELECT count(*) FROM public.mdm_graph_probe) <> 1
       OR (SELECT owner_name FROM public.mdm_graph_probe) <> 'mdm_administrator'
       OR (SELECT count(*) FROM public.mdm_graph_publication) <> 1 THEN
        RAISE EXCEPTION 'Graph V1 commit or owner-scoped RLS failed';
    END IF;
END
$$;
DO $$
DECLARE
    refreshed jsonb;
BEGIN
    INSERT INTO public.mdm_graph_diff_source
    SELECT id, 'mdm_administrator', 'baseline'
    FROM generate_series(3, 102) AS ids(id);
    refreshed := public.refresh_mdm_graph(ARRAY[
        'public.mdm_graph_diff_probe'::regclass,
        'public.mdm_graph_diff_probe_reference'::regclass]);
    IF (SELECT count(*) FROM public.mdm_graph_diff_probe) <> 101
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe EXCEPT SELECT * FROM public.mdm_graph_diff_probe_reference)
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe_reference EXCEPT SELECT * FROM public.mdm_graph_diff_probe) THEN
        RAISE EXCEPTION 'AUTO and FULL baseline results disagree: %', refreshed;
    END IF;
END
$$;
DO $$
DECLARE
    refreshed jsonb;
BEGIN
    INSERT INTO public.mdm_graph_diff_source VALUES (103, 'mdm_administrator', 'inserted');
    refreshed := public.refresh_mdm_graph(ARRAY[
        'public.mdm_graph_diff_probe'::regclass,
        'public.mdm_graph_diff_probe_reference'::regclass]);
    IF refreshed->'source_boundary'->>'completeness' <> 'PROVEN'
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_graph_diff_probe') IS NULL
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_graph_diff_probe') = 'FULL'
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_graph_diff_probe_reference') IS DISTINCT FROM 'FULL'
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe EXCEPT SELECT * FROM public.mdm_graph_diff_probe_reference)
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe_reference EXCEPT SELECT * FROM public.mdm_graph_diff_probe) THEN
        RAISE EXCEPTION 'AUTO and FULL disagree after insert: %', refreshed;
    END IF;

    UPDATE public.mdm_graph_diff_source SET value = 'updated' WHERE id = 103;
    refreshed := public.refresh_mdm_graph(ARRAY[
        'public.mdm_graph_diff_probe'::regclass,
        'public.mdm_graph_diff_probe_reference'::regclass]);
    IF refreshed->'source_boundary'->>'completeness' <> 'PROVEN'
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_graph_diff_probe') IS NULL
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_graph_diff_probe') = 'FULL'
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_graph_diff_probe_reference') IS DISTINCT FROM 'FULL'
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe EXCEPT SELECT * FROM public.mdm_graph_diff_probe_reference)
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe_reference EXCEPT SELECT * FROM public.mdm_graph_diff_probe) THEN
        RAISE EXCEPTION 'AUTO and FULL disagree after update: %', refreshed;
    END IF;

    DELETE FROM public.mdm_graph_diff_source WHERE id = 103;
    refreshed := public.refresh_mdm_graph(ARRAY[
        'public.mdm_graph_diff_probe'::regclass,
        'public.mdm_graph_diff_probe_reference'::regclass]);
    IF refreshed->'source_boundary'->>'completeness' <> 'PROVEN'
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_graph_diff_probe') IS NULL
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_graph_diff_probe') = 'FULL'
       OR public.mdm_graph_action(refreshed->'node_results', 'public.mdm_graph_diff_probe_reference') IS DISTINCT FROM 'FULL'
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe EXCEPT SELECT * FROM public.mdm_graph_diff_probe_reference)
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe_reference EXCEPT SELECT * FROM public.mdm_graph_diff_probe) THEN
        RAISE EXCEPTION 'AUTO and FULL disagree after delete: %', refreshed;
    END IF;

    refreshed := public.refresh_mdm_graph(ARRAY[
        'public.mdm_graph_diff_probe'::regclass,
        'public.mdm_graph_diff_probe_reference'::regclass]);
    IF refreshed->'source_boundary'->>'completeness' <> 'PROVEN'
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe EXCEPT SELECT * FROM public.mdm_graph_diff_probe_reference)
       OR EXISTS (SELECT * FROM public.mdm_graph_diff_probe_reference EXCEPT SELECT * FROM public.mdm_graph_diff_probe) THEN
        RAISE EXCEPTION 'AUTO and FULL disagree on no-op refresh: %', refreshed;
    END IF;
END
$$;
DO $$
DECLARE
    refreshed jsonb;
BEGIN
    INSERT INTO public.mdm_candidate_source VALUES
        ('00000000-0000-0000-0000-000000000001', 'email', 'value', '\x01'::bytea, '\x01'::bytea),
        ('00000000-0000-0000-0000-000000000002', 'email', 'value', '\x01'::bytea, '\x02'::bytea),
        ('00000000-0000-0000-0000-000000000003', 'email', 'value', '\x01'::bytea, '\x03'::bytea),
        ('00000000-0000-0000-0000-000000000004', 'email', 'value', '\x01'::bytea, '\x04'::bytea),
        ('00000000-0000-0000-0000-000000000005', 'phone', 'value', '\x01'::bytea, '\x05'::bytea),
        ('00000000-0000-0000-0000-000000000006', 'email', 'missing', '\x01'::bytea, '\x06'::bytea);
    refreshed := public.check_mdm_candidate_probes('multi-row insert');
    IF (SELECT count(*) FROM public.mdm_candidate_blocks_full) <> 4
       OR (SELECT count(*) FROM public.mdm_candidate_pairs_full) <> 6 THEN
        RAISE EXCEPTION 'candidate FULL result after multi-row insert is invalid: %', refreshed;
    END IF;
END
$$;

DO $$
DECLARE refreshed jsonb;
BEGIN
    UPDATE public.mdm_candidate_source SET canonical_bytes = '\x02'::bytea
    WHERE source_record_id = '00000000-0000-0000-0000-000000000004' AND field_name = 'email';
    refreshed := public.check_mdm_candidate_probes('update');
    IF (SELECT count(*) FROM public.mdm_candidate_pairs_full) <> 3 THEN
        RAISE EXCEPTION 'candidate FULL result after update is invalid: %', refreshed;
    END IF;
END
$$;

DO $$
DECLARE refreshed jsonb;
BEGIN
    DELETE FROM public.mdm_candidate_source
    WHERE source_record_id = '00000000-0000-0000-0000-000000000003';
    refreshed := public.check_mdm_candidate_probes('delete');
    IF (SELECT count(*) FROM public.mdm_candidate_pairs_full) <> 1 THEN
        RAISE EXCEPTION 'candidate FULL result after delete is invalid: %', refreshed;
    END IF;
END
$$;

BEGIN;
UPDATE public.mdm_candidate_source SET canonical_bytes = '\x01'::bytea
WHERE source_record_id = '00000000-0000-0000-0000-000000000004' AND field_name = 'email';
DO $$
DECLARE refreshed jsonb;
BEGIN
    refreshed := public.check_mdm_candidate_probes('rolled-back mutation');
END
$$;
ROLLBACK;

DO $$
DECLARE refreshed jsonb;
BEGIN
    refreshed := public.check_mdm_candidate_probes('rollback recovery');
    IF (SELECT count(*) FROM public.mdm_candidate_pairs_full) <> 1 THEN
        RAISE EXCEPTION 'candidate FULL result after rollback recovery is invalid: %', refreshed;
    END IF;
END
$$;

DO $$
DECLARE refreshed jsonb;
BEGIN
    UPDATE public.mdm_candidate_source SET canonical_bytes = '\x01'::bytea
    WHERE source_record_id = '00000000-0000-0000-0000-000000000004' AND field_name = 'email';
    refreshed := public.check_mdm_candidate_probes('retry');
    IF (SELECT count(*) FROM public.mdm_candidate_pairs_full) <> 3 THEN
        RAISE EXCEPTION 'candidate FULL result after retry is invalid: %', refreshed;
    END IF;
END
$$;
DROP FUNCTION public.mdm_graph_action(jsonb, text);
DROP FUNCTION public.check_mdm_candidate_probes(text);
DROP FUNCTION public.refresh_mdm_graph(regclass[]);
RESET ROLE;
RESET SESSION AUTHORIZATION;

\connect foundation mdm_test_login

SET ROLE mdm_administrator;
DO $$
DECLARE
    caller name := current_user;
    authenticated name := session_user;
    caller_path text := pg_catalog.current_setting('search_path');
BEGIN
    BEGIN
        PERFORM mdm_admin.verify_installation();
        RAISE EXCEPTION 'unsafe owner was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_HELPER_OWNER_UNSAFE') = 0 THEN
            RAISE;
        END IF;
    END;
    IF current_user <> caller OR session_user <> authenticated
       OR pg_catalog.current_setting('search_path') <> caller_path THEN
        RAISE EXCEPTION 'helper error changed caller context';
    END IF;
END
$$;

\connect foundation postgres
\set helper_owner mdm_helper_owner
\ir /sql/configure_helper.sql

DO $$
DECLARE helper record;
BEGIN
    FOR helper IN
        SELECT p.oid, p.proconfig, p.proowner, p.proacl
        FROM pg_catalog.pg_proc p
        JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
        WHERE n.nspname IN ('mdm', 'mdm_admin', 'mdm_internal') AND p.prosecdef
    LOOP
        IF helper.proconfig IS DISTINCT FROM ARRAY['search_path=pg_catalog, mdm_internal, pg_temp']
           OR helper.proowner <> 'mdm_helper_owner'::regrole
           OR EXISTS (
               SELECT FROM pg_catalog.aclexplode(COALESCE(helper.proacl, pg_catalog.acldefault('f', helper.proowner)))
               WHERE grantee = 0 AND privilege_type = 'EXECUTE'
           ) THEN
            RAISE EXCEPTION 'unsafe helper configuration: %', helper.oid::regprocedure;
        END IF;
    END LOOP;
    IF EXISTS (
        SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
        WHERE n.nspname = 'mdm' AND p.prosecdef
    ) THEN
        RAISE EXCEPTION 'public source actions must be SECURITY INVOKER';
    END IF;
    IF has_schema_privilege('mdm_administrator', 'mdm_internal', 'USAGE')
       OR EXISTS (
           SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
           WHERE n.nspname = 'mdm_internal'
             AND has_function_privilege('mdm_administrator', p.oid, 'EXECUTE')
       ) THEN
        RAISE EXCEPTION 'application role can access a private helper';
    END IF;
END
$$;

SELECT NOT has_schema_privilege('mdm_configurator', 'mdm_internal', 'USAGE') AS internal_schema_private,
       NOT has_table_privilege('mdm_configurator', 'mdm_internal.operations', 'SELECT') AS operations_private,
       NOT has_function_privilege('mdm_configurator', 'mdm_admin.verify_installation()', 'EXECUTE') AS helper_private
\gset
\if :internal_schema_private
\else
\quit 1
\endif
\if :operations_private
\else
\quit 1
\endif
\if :helper_private
\else
\quit 1
\endif

CREATE SCHEMA attacker AUTHORIZATION mdm_test_login;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET search_path = attacker, public;
BEGIN;
SELECT mdm_admin.verify_installation();
ROLLBACK;
RESET search_path;
RESET ROLE;

\connect foundation postgres
GRANT USAGE ON SCHEMA public TO mdm_administrator;
CREATE TABLE public.crm_customer (
    id bigint PRIMARY KEY,
    display_name text NOT NULL,
    email_address text,
    updated_at timestamptz NOT NULL
);
GRANT SELECT, MAINTAIN ON public.crm_customer TO mdm_administrator;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
WITH proposed AS (
    SELECT mdm.entity(
        name => 'customer',
        sources => ARRAY[
            mdm.source(
                name => 'crm',
                relation => 'public.crm_customer'::regclass,
                source_id => ARRAY['id'],
                mode => 'tracked',
                fields => jsonb_build_object(
                    'name', 'display_name',
                    'email', 'email_address'
                ),
                row_changed_at => 'updated_at'
            )
        ],
        fields => ARRAY[
            mdm.field(name => 'name', type => 'text', cleaner => 'company_name'),
            mdm.field(name => 'email', type => 'text', cleaner => 'email')
        ],
        matches => ARRAY[
            mdm.match(
                name => 'same_email',
                fields => ARRAY['email'],
                comparison => 'exact',
                strength => 'identity',
                evidence_group => 'email',
                candidate => jsonb_build_object('kind', 'exact', 'field', 'email')
            )
        ],
        golden_values => ARRAY[
            mdm.golden_value(field => 'name', policy => 'prefer_source', sources => ARRAY['crm'])
        ]
    ) AS definition
)
SELECT desired_version = 1 AND changed AND octet_length(definition_digest) = 32 AND octet_length(artifact_digest) = 32 AS v02_create_ok
FROM mdm.create((SELECT definition FROM proposed), NULL, 'initial customer definition')
\gset
\if :v02_create_ok
\else
\quit 1
\endif

DO $$
DECLARE
    summary jsonb;
BEGIN
    summary := mdm.describe('customer', 'summary');
    IF summary->>'active_version' IS NOT NULL
       OR summary->>'graph_state' <> 'ready'
       OR summary->>'graph_generation' <> '1'
       OR (summary->>'member_count')::integer <= 0
       OR length(summary->>'graph_binding_digest') <> 64
       OR length(summary->>'graph_digest') <> 64
       OR summary #>> '{candidate_plan,channels,0,channel_id}' <> 'same_email'
       OR summary #>> '{candidate_plan,channels,0,fields,0}' <> 'email'
       OR summary #>> '{candidate_plan,max_block_records}' IS NULL
       OR summary #>> '{candidate_plan,max_candidate_pairs}' IS NULL
       OR summary #>> '{candidate_plan,warning_block_records}' IS NULL
       OR summary #>> '{candidate_semantics,candidate,absolute_ceilings,max_block_records}' IS NULL
       OR summary #>> '{candidate_semantics,candidate,absolute_ceilings,max_candidate_pairs}' IS NULL
       OR summary #>> '{evidence_semantics,absolute_max_comparator_work}' IS NULL
       OR summary #>> '{candidate_semantics,decisions,absolute_max_decision_closure}' IS NULL
       OR summary #>> '{candidate_semantics,clustering,absolute_ceilings,max_active_records}' IS NULL
       OR summary->'resolver_limits' IS NULL
       OR summary::text LIKE '%mdm_graph.%'
       OR summary->'graph' ? 'contract'
       OR summary->'graph' ? 'members' THEN
        RAISE EXCEPTION 'create did not expose the complete bounded plan and dormant graph: %', summary;
    END IF;
END
$$;

\connect foundation postgres
DO $$
DECLARE
    member record;
    contract jsonb;
    row_count bigint;
    member_count integer := 0;
BEGIN
    FOR member IN
        SELECT gm.relation_oid, gm.relation_name
        FROM mdm_internal.graph_members gm
        JOIN mdm_internal.graph_bindings gb USING (graph_binding_id)
        JOIN mdm_internal.entities e USING (entity_id)
        WHERE e.entity_name = 'customer' AND gb.definition_version = 1
        ORDER BY gm.topological_ordinal
    LOOP
        member_count := member_count + 1;
        SELECT c.contract INTO STRICT contract
        FROM pgtrickle.stream_table_contract(member.relation_oid::regclass) c;
        IF contract->>'orchestration_mode' <> 'EXTERNAL'
           OR contract #>> '{relation,owner}' <> 'mdm_administrator' THEN
            RAISE EXCEPTION 'graph member contract is not externally owned: %', contract;
        END IF;
        EXECUTE pg_catalog.format('SELECT pg_catalog.count(*) FROM %s', member.relation_oid::regclass)
            INTO row_count;
        IF row_count <> 0 THEN
            RAISE EXCEPTION 'graph member was initialized before refresh: % has % rows',
                member.relation_name, row_count;
        END IF;
    END LOOP;
    IF member_count = 0 THEN
        RAISE EXCEPTION 'create did not install graph members';
    END IF;
END
$$;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SELECT summary->>'graph_binding_digest' AS binding_digest,
       summary->>'graph_digest' AS graph_digest,
       summary->>'graph_generation' AS graph_generation,
       summary->>'member_count' AS member_count,
       summary->'publication'->>'publication_revision' AS publication_revision
FROM (SELECT mdm.describe('customer', 'summary') AS summary) before_create
\gset v08_
WITH proposed AS (
    SELECT mdm.entity(
        name => 'customer',
        sources => ARRAY[mdm.source(
            name => 'crm', relation => 'public.crm_customer'::regclass,
            source_id => ARRAY['id'], mode => 'tracked',
            fields => jsonb_build_object('name', 'display_name', 'email', 'email_address'),
            row_changed_at => 'updated_at')],
        fields => ARRAY[
            mdm.field(name => 'name', type => 'text', cleaner => 'company_name'),
            mdm.field(name => 'email', type => 'text', cleaner => 'email')],
        matches => ARRAY[mdm.match(
            name => 'same_email', fields => ARRAY['email'], comparison => 'exact',
            strength => 'identity', evidence_group => 'email',
            candidate => jsonb_build_object('kind', 'exact', 'field', 'email'))],
        golden_values => ARRAY[mdm.golden_value(
            field => 'name', policy => 'prefer_source', sources => ARRAY['crm'])]
    ) AS definition
)
SELECT NOT changed AND desired_version = 1
   AND mdm.describe('customer', 'summary')->>'graph_state' = 'ready'
   AND mdm.describe('customer', 'summary')->>'active_version' IS NULL
   AND mdm.describe('customer', 'summary')->>'graph_binding_digest' IS NOT DISTINCT FROM :'v08_binding_digest'
   AND mdm.describe('customer', 'summary')->>'graph_digest' IS NOT DISTINCT FROM :'v08_graph_digest'
   AND mdm.describe('customer', 'summary')->>'graph_generation' IS NOT DISTINCT FROM :'v08_graph_generation'
   AND mdm.describe('customer', 'summary')->>'member_count' IS NOT DISTINCT FROM :'v08_member_count'
   AND mdm.describe('customer', 'summary')->'publication'->>'publication_revision' = '0'
   AND :'v08_publication_revision' = '0' AS v02_noop_ok
FROM mdm.create((SELECT definition FROM proposed), 1, NULL)
\gset
\if :v02_noop_ok
\else
\quit 1
\endif


DO $$
DECLARE
    described jsonb;
    summary jsonb;
    result record;
BEGIN
    described := mdm.describe('customer', 'definition');
    summary := mdm.describe('customer', 'summary');
    IF described->>'name' IS DISTINCT FROM 'customer' THEN
        RAISE EXCEPTION 'definition description did not round trip';
    END IF;
    SELECT * INTO STRICT result FROM mdm.create(described, 1, 'definition round trip');
    IF result.changed
       OR encode(result.definition_digest, 'hex') IS DISTINCT FROM summary->>'definition_digest'
       OR encode(result.artifact_digest, 'hex') IS DISTINCT FROM summary->>'artifact_digest' THEN
        RAISE EXCEPTION 'definition round trip changed a semantic or artifact digest';
    END IF;
    SELECT * INTO STRICT result FROM mdm.create(
        jsonb_set(described, '{limits,max_candidate_pairs}', '100'), 1, 'definition B');
    IF result.desired_version <> 2 OR NOT result.changed THEN
        RAISE EXCEPTION 'A to B did not create version 2';
    END IF;
    SELECT * INTO STRICT result FROM mdm.create(described, 2, 'return to definition A');
    IF result.desired_version <> 3 OR NOT result.changed THEN
        RAISE EXCEPTION 'B to A did not create version 3';
    END IF;
    BEGIN
        PERFORM mdm.create(described, 2);
        RAISE EXCEPTION 'stale expected_version was accepted for a no-op';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_VERSION_CONFLICT') = 0 THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM mdm.create(jsonb_set(described, '{execution_role}', '"mdm_configurator"'), 3);
        RAISE EXCEPTION 'unselected execution role was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_UNAUTHORIZED') = 0 THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM mdm_internal.describe_entity(NULL);
        RAISE EXCEPTION 'application called a private helper';
    EXCEPTION WHEN insufficient_privilege THEN NULL;
    END;
END
$$;
RESET ROLE;

\connect foundation postgres
DO $$
DECLARE
    definition_count bigint;
    artifact_count bigint;
BEGIN
    SELECT count(*) INTO definition_count FROM mdm_internal.definitions d
    JOIN mdm_internal.entities e ON e.entity_id = d.entity_id
    WHERE e.entity_name = 'customer';
    SELECT count(*) INTO artifact_count FROM mdm_internal.definition_artifacts a
    JOIN mdm_internal.entities e ON e.entity_id = a.entity_id
    WHERE e.entity_name = 'customer';
    IF to_regclass('mdm_out.customer') IS NOT NULL THEN
        RAISE EXCEPTION 'v0.2 created a public output table';
    END IF;
    IF definition_count <> 3 OR artifact_count <> 3 THEN
        RAISE EXCEPTION 'unexpected definition history: % / %', definition_count, artifact_count;
    END IF;
    IF (SELECT count(DISTINCT definition_digest) FROM mdm_internal.definitions) <> 2 THEN
        RAISE EXCEPTION 'A to B to A did not preserve definition identity';
    END IF;
    IF (SELECT count(*) FROM mdm_internal.operations) <> 5
       OR (SELECT count(*) FROM mdm_internal.operations
           WHERE operation_kind = 'create' AND status = 'succeeded'
             AND actor_name = 'mdm_test_login' AND actor_role_name = 'mdm_administrator') <> 5 THEN
        RAISE EXCEPTION 'unexpected operation count after v0.2 create';
    END IF;
END
$$;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE
    summary jsonb;
BEGIN
    summary := mdm.describe('customer', 'summary');
    IF summary->>'source_key_encoding' IS DISTINCT FROM '2'
       OR summary->>'row_identity_version' IS DISTINCT FROM '2'
       OR summary->>'clustering_policy' IS DISTINCT FROM '1'
       OR (summary->'resolver_limits' ? 'max_active_records') IS NOT TRUE
       OR (summary->'clustering_admission' ? 'established_established') IS NOT TRUE
       OR jsonb_array_length(summary->'cleaners') <> 2
       OR summary->'cleaner_versions'->>'text' IS DISTINCT FROM '1'
       OR summary->'cleaner_versions'->>'date' IS DISTINCT FROM '1'
       OR summary->'cleaner_versions'->>'email' IS DISTINCT FROM '1' THEN
        RAISE EXCEPTION 'describe summary does not have expected v0.4 metadata: %', summary;
    END IF;
END
$$;
RESET ROLE;

\connect foundation postgres
INSERT INTO public.crm_customer VALUES
    (9001, 'Steward One', 'steward-one@example.test', statement_timestamp()),
    (9002, 'Steward Two', 'steward-two@example.test', statement_timestamp()),
    (9003, 'Steward Three', 'steward-three@example.test', statement_timestamp());
DO $$
DECLARE
    norm mdm_internal.normalized_value;
BEGIN
    norm := mdm_internal.normalize_text('  Foo  Bar  ', 'text', 1, 'present', '{}'::jsonb);
    IF norm.state <> 'value' OR norm.normalized <> 'foo bar' OR norm.canonical_bytes <> '\x0101666f6f20626172'::bytea THEN
        RAISE EXCEPTION 'normalize_text failed: %', norm;
    END IF;

    norm := mdm_internal.normalize_date('2024-02-29'::date, 'date', 1, 'present', '{}'::jsonb);
    IF norm.state <> 'value' OR norm.normalized <> '2024-02-29' OR norm.canonical_bytes <> '\x0102323032342d30322d3239'::bytea THEN
        RAISE EXCEPTION 'normalize_date failed: %', norm;
    END IF;

END
$$;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE
    first_refresh jsonb;
    second_refresh jsonb;
    rebuilt jsonb;
BEGIN
    first_refresh := mdm.refresh('customer', 'ALLOW');
    IF first_refresh->>'changed' <> 'true'
       OR (first_refresh->>'publication_revision')::bigint <> 1
       OR (first_refresh->>'graph_refresh_id')::bigint <= 0
       OR first_refresh->'source_boundary'->>'completeness' <> 'PROVEN'
       OR length(first_refresh->>'source_boundary_digest') <> 64
       OR jsonb_typeof(first_refresh->'stage_timings_ms') <> 'object'
       OR first_refresh->'stage_timings_ms'->>'graph_refresh' IS NULL
       OR first_refresh->'stage_timings_ms'->>'mdm_resolution' IS NULL
       OR first_refresh->'stage_timings_ms'->>'publication' IS NULL
       OR first_refresh->'stage_timings_ms'->>'elapsed_before_operation_completion' IS NULL
       OR (first_refresh->'stage_timings_ms'->>'graph_refresh')::bigint < 0
       OR (first_refresh->'stage_timings_ms'->>'mdm_resolution')::bigint < 0
       OR (first_refresh->'stage_timings_ms'->>'publication')::bigint < 0
       OR (first_refresh->'stage_timings_ms'->>'elapsed_before_operation_completion')::bigint < 0
       OR first_refresh->>'component_checks' IS NULL
       OR (first_refresh->>'component_checks')::bigint < 0 THEN
        RAISE EXCEPTION 'initial v0.9 refresh is invalid: %', first_refresh;
    END IF;
    second_refresh := mdm.refresh('customer', 'ALLOW');
    IF second_refresh->>'changed' <> 'false'
       OR (second_refresh->>'publication_revision')::bigint <> 1 THEN
        RAISE EXCEPTION 'refresh no-op is invalid: %', second_refresh;
    END IF;
    rebuilt := mdm_admin.rebuild('customer', 'ALLOW');
    IF rebuilt->>'entity_name' <> 'customer'
       OR (rebuilt->>'publication_revision')::bigint <> 1 THEN
        RAISE EXCEPTION 'administrative rebuild is invalid: %', rebuilt;
    END IF;
END
$$;
RESET ROLE;


\connect foundation postgres
DO $$
DECLARE entity_columns text[]; member_columns text[]; review_columns text[];
BEGIN
    SELECT pg_catalog.array_agg(a.attname::text || ':' || pg_catalog.format_type(a.atttypid, a.atttypmod) ORDER BY a.attnum)
      INTO entity_columns
      FROM pg_catalog.pg_attribute a
     WHERE a.attrelid = 'mdm_out.customer'::pg_catalog.regclass
       AND a.attnum > 0 AND NOT a.attisdropped;
    SELECT pg_catalog.array_agg(a.attname::text || ':' || pg_catalog.format_type(a.atttypid, a.atttypmod) ORDER BY a.attnum)
      INTO member_columns
      FROM pg_catalog.pg_attribute a
     WHERE a.attrelid = 'mdm_out.customer_members'::pg_catalog.regclass
       AND a.attnum > 0 AND NOT a.attisdropped;
    SELECT pg_catalog.array_agg(a.attname::text || ':' || pg_catalog.format_type(a.atttypid, a.atttypmod) ORDER BY a.attnum)
      INTO review_columns
      FROM pg_catalog.pg_attribute a
     WHERE a.attrelid = 'mdm_out.customer_review'::pg_catalog.regclass
       AND a.attnum > 0 AND NOT a.attisdropped;
    IF entity_columns IS DISTINCT FROM ARRAY['mdm_id:uuid', 'name:text', 'member_count:bigint', 'has_review:boolean', 'last_change_revision:bigint']
       OR member_columns IS DISTINCT FROM ARRAY['source_record_id:uuid', 'source_name:name', 'source_id:jsonb', 'mdm_id:uuid', 'active:boolean', 'first_membership_revision:bigint', 'last_membership_revision:bigint', 'membership_reason:text', 'last_change_revision:bigint']
       OR review_columns IS DISTINCT FROM ARRAY['review_id:uuid', 'issue_key:bytea', 'occurrence:integer', 'status:text', 'severity:text', 'reason_code:text', 'subjects:jsonb', 'masked_summary:jsonb', 'opened_revision:bigint', 'resolved_revision:bigint', 'last_change_revision:bigint', 'concurrency_version:bigint'] THEN
        RAISE EXCEPTION 'public output table names, column order, or PostgreSQL types changed: %, %, %',
            entity_columns, member_columns, review_columns;
    END IF;
END
$$;

CREATE TABLE public.e2e_pg_trickle_upgrade_snapshot AS
SELECT e.publication_revision,
       (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(b) ORDER BY b.graph_generation)
        FROM mdm_internal.graph_bindings b WHERE b.entity_id = e.entity_id) AS bindings,
       (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(m) ORDER BY m.logical_id)
        FROM mdm_internal.graph_members m JOIN mdm_internal.graph_bindings b USING (graph_binding_id)
        WHERE b.entity_id = e.entity_id) AS graph_members,
       (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(p) ORDER BY p.publication_revision)
        FROM mdm_internal.publications p WHERE p.entity_id = e.entity_id) AS publications,
       (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(i) ORDER BY i.mdm_id)
        FROM mdm_internal.identity_registry i WHERE i.entity_id = e.entity_id) AS identities,
       (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) ORDER BY c.mdm_id)
        FROM mdm_out.customer c) AS outputs,
       pg_catalog.has_table_privilege('mdm_output_reader', 'mdm_out.customer', 'SELECT') AS output_reader_grant,
       pg_catalog.to_regclass('mdm_out.customer')::text AS consumer_relation
FROM mdm_internal.entities e
WHERE e.entity_name = 'customer';
DO $$
BEGIN
    IF (SELECT extversion FROM pg_catalog.pg_extension WHERE extname = 'pg_trickle') <> '0.105.1' THEN
        RAISE EXCEPTION 'populated upgrade fixture did not start on pg_trickle 0.105.1';
    END IF;
END
$$;
ALTER EXTENSION pg_trickle UPDATE TO '0.105.2';
DO $$
DECLARE
    snapshot record;
    current_state record;
BEGIN
    SELECT * INTO STRICT snapshot FROM public.e2e_pg_trickle_upgrade_snapshot;
    SELECT e.publication_revision,
           (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(b) ORDER BY b.graph_generation)
            FROM mdm_internal.graph_bindings b WHERE b.entity_id = e.entity_id) AS bindings,
           (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(m) ORDER BY m.logical_id)
            FROM mdm_internal.graph_members m JOIN mdm_internal.graph_bindings b USING (graph_binding_id)
            WHERE b.entity_id = e.entity_id) AS graph_members,
           (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(p) ORDER BY p.publication_revision)
            FROM mdm_internal.publications p WHERE p.entity_id = e.entity_id) AS publications,
           (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(i) ORDER BY i.mdm_id)
            FROM mdm_internal.identity_registry i WHERE i.entity_id = e.entity_id) AS identities,
           (SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) ORDER BY c.mdm_id)
            FROM mdm_out.customer c) AS outputs,
           pg_catalog.has_table_privilege('mdm_output_reader', 'mdm_out.customer', 'SELECT') AS output_reader_grant,
           pg_catalog.to_regclass('mdm_out.customer')::text AS consumer_relation
    INTO STRICT current_state
    FROM mdm_internal.entities e
    WHERE e.entity_name = 'customer';
    IF (SELECT extversion FROM pg_catalog.pg_extension WHERE extname = 'pg_trickle') <> '0.105.2'
       OR snapshot.publication_revision IS DISTINCT FROM current_state.publication_revision
       OR snapshot.bindings IS DISTINCT FROM current_state.bindings
       OR snapshot.graph_members IS DISTINCT FROM current_state.graph_members
       OR snapshot.publications IS DISTINCT FROM current_state.publications
       OR snapshot.identities IS DISTINCT FROM current_state.identities
       OR snapshot.outputs IS DISTINCT FROM current_state.outputs
       OR snapshot.output_reader_grant IS DISTINCT FROM current_state.output_reader_grant
       OR snapshot.consumer_relation IS DISTINCT FROM current_state.consumer_relation THEN
        RAISE EXCEPTION 'pg_trickle upgrade changed populated MDM state: before %, after %', snapshot, current_state;
    END IF;
END
$$;
DROP TABLE public.e2e_pg_trickle_upgrade_snapshot;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'false' OR (result->>'publication_revision')::bigint <> 1 THEN
        RAISE EXCEPTION 'post-upgrade populated graph refresh is invalid: %', result;
    END IF;
END
$$;
RESET ROLE;


\connect foundation postgres
GRANT USAGE ON SCHEMA mdm_steward TO mdm_administrator;
GRANT EXECUTE ON FUNCTION mdm_steward.decide(text, uuid, uuid, text, bigint, text) TO mdm_administrator;
CREATE FUNCTION public.e2e_source_records(ids bigint[])
RETURNS TABLE(source_record_id uuid, source_record_key bytea)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, mdm_internal, public
AS $$
    SELECT r.source_record_id, r.source_record_key
    FROM mdm_internal.source_records r
    JOIN mdm_internal.source_identities s
      ON s.source_identity_id = r.source_identity_id AND s.entity_id = r.entity_id
    JOIN mdm_internal.entities e ON e.entity_id = r.entity_id
    JOIN public.crm_customer c
      ON r.source_record_key = pgtrickle.encode_row_id_v2(
          'SCAN_KEY', ROW(e.entity_id, s.source_identity_id, c.id))
    WHERE e.entity_name = 'customer' AND c.id = ANY (ids)
$$;
REVOKE ALL ON FUNCTION public.e2e_source_records(bigint[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.e2e_source_records(bigint[]) TO mdm_administrator;
CREATE FUNCTION public.e2e_active_mdm_ids(ids bigint[])
RETURNS uuid[]
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, mdm_internal, public
AS $$
    SELECT COALESCE(pg_catalog.array_agg(DISTINCT m.mdm_id ORDER BY m.mdm_id), ARRAY[]::uuid[])
    FROM mdm_internal.memberships m
    JOIN public.e2e_source_records(ids) r USING (source_record_id)
    WHERE m.active
$$;
REVOKE ALL ON FUNCTION public.e2e_active_mdm_ids(bigint[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.e2e_active_mdm_ids(bigint[]) TO mdm_administrator;
CREATE FUNCTION public.e2e_preview_state()
RETURNS jsonb
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, mdm_internal, mdm_out
AS $$
    SELECT pg_catalog.jsonb_build_object(
        'publication_revision', e.publication_revision,
        'publication_count', (SELECT count(*) FROM mdm_internal.publications p WHERE p.entity_id = e.entity_id),
        'observation_count', (SELECT count(*) FROM mdm_internal.publication_observations o WHERE o.entity_id = e.entity_id),
        'operation_count', (SELECT count(*) FROM mdm_internal.operations o WHERE o.entity_name = e.entity_name),
        'source_record_count', (SELECT count(*) FROM mdm_internal.source_records r WHERE r.entity_id = e.entity_id),
        'identity_count', (SELECT count(*) FROM mdm_internal.identity_registry i WHERE i.entity_id = e.entity_id),
        'output_count', (SELECT count(*) FROM mdm_out.customer)
    )
    FROM mdm_internal.entities e
    WHERE e.entity_name = 'customer'
$$;
REVOKE ALL ON FUNCTION public.e2e_preview_state() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.e2e_preview_state() TO mdm_administrator;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE
    before_state jsonb;
    after_state jsonb;
    validation jsonb;
    sampled jsonb;
    sampled_repeat jsonb;
    scoped jsonb;
    source_ids jsonb;
    duplicate_id text;
BEGIN
    before_state := public.e2e_preview_state();
    validation := mdm.preview('customer', 'validation');
    sampled := mdm.preview('customer', 'sampled', '{"sample_size":2}'::jsonb);
    sampled_repeat := mdm.preview('customer', 'sampled', '{"sample_size":2}'::jsonb);
    SELECT pg_catalog.jsonb_agg(source_record_id::text ORDER BY source_record_id), min(source_record_id::text)
    INTO source_ids, duplicate_id
    FROM public.e2e_source_records(ARRAY[9001, 9002]::bigint[]);
    scoped := mdm.preview('customer', 'scoped', pg_catalog.jsonb_build_object('source_record_ids', source_ids));
    after_state := public.e2e_preview_state();
    IF validation->>'evidence_level' <> 'validation' OR validation->>'exact' <> 'false'
       OR validation->>'data_read' <> 'false'
       OR sampled->>'evidence_level' <> 'sampled' OR sampled->>'exact' <> 'false'
       OR sampled->>'sample_size' <> '2' OR sampled->>'record_count' <> '2'
       OR sampled IS DISTINCT FROM sampled_repeat
       OR scoped->>'evidence_level' <> 'exact_subjects' OR scoped->>'exact' <> 'true'
       OR scoped->>'limitation' NOT LIKE '%Omitted records can change the full-entity result%'
       OR before_state IS DISTINCT FROM after_state THEN
        RAISE EXCEPTION 'preview contract or read-only behavior failed: %, %, %, state %, %', validation, sampled, scoped, before_state, after_state;
    END IF;
    BEGIN
        PERFORM mdm.preview('customer', 'scoped', pg_catalog.jsonb_build_object('source_record_ids', pg_catalog.jsonb_build_array(duplicate_id, duplicate_id)));
        RAISE EXCEPTION 'duplicate preview subjects were accepted';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_DEFINITION_INVALID') = 0 THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM mdm.preview('customer', 'scoped', pg_catalog.jsonb_build_object('source_record_ids', source_ids, 'unknown', true));
        RAISE EXCEPTION 'unknown preview option was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_DEFINITION_INVALID') = 0 THEN RAISE; END IF;
    END;
END
$$;
RESET ROLE;
\connect foundation postgres
INSERT INTO mdm_internal.definition_artifacts (
    entity_id, definition_version, compiler_version, artifact_format_version,
    artifact_bytes, artifact_digest, created_by_name
)
SELECT entity_id, desired_version, 2147483647, 1,
       pg_catalog.decode('ff', 'hex'),
       pg_catalog.decode(pg_catalog.repeat('ff', 32), 'hex'),
       current_user::text
FROM mdm_internal.entities
WHERE entity_name = 'customer';
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
BEGIN
    BEGIN
        PERFORM mdm.preview('customer', 'validation');
        RAISE EXCEPTION 'preview accepted a graph binding for a different artifact';
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM <> 'MDM_GRAPH_BINDING: graph binding is invalid: no graph binding exists for the desired definition' THEN
            RAISE;
        END IF;
    END;
END
$$;
RESET ROLE;
\connect foundation postgres
DELETE FROM mdm_internal.definition_artifacts
WHERE artifact_digest = pg_catalog.decode(pg_catalog.repeat('ff', 32), 'hex');
CREATE TABLE public.e2e_customer_state_snapshot (
    snapshot_id boolean PRIMARY KEY DEFAULT true CHECK (snapshot_id),
    state jsonb NOT NULL
);
REVOKE ALL ON public.e2e_customer_state_snapshot FROM PUBLIC, mdm_administrator;
CREATE FUNCTION public.e2e_customer_state()
RETURNS jsonb
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, mdm_out
AS $$
DECLARE
    current_internal jsonb;
    previous_internal jsonb;
    changed_categories jsonb := '[]'::jsonb;
BEGIN
    SELECT pg_catalog.jsonb_build_object(
        'registry', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(i) ORDER BY i.mdm_id) FROM mdm_internal.identity_registry i JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer'), '[]'::jsonb),
        'memberships', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(m) ORDER BY m.source_record_id) FROM mdm_internal.memberships m JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer'), '[]'::jsonb),
        'aliases', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(a) ORDER BY a.alias_mdm_id) FROM mdm_internal.identity_aliases a JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer'), '[]'::jsonb),
        'splits', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(s) ORDER BY s.parent_mdm_id, s.publication_revision, s.child_mdm_id) FROM mdm_internal.identity_splits s JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer'), '[]'::jsonb),
        'golden', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(g) - 'publication_revision' ORDER BY g.mdm_id, g.field_name) FROM mdm_internal.golden_provenance g JOIN mdm_internal.entities e ON e.entity_id = g.entity_id AND e.publication_revision = g.publication_revision WHERE e.entity_name = 'customer'), '[]'::jsonb),
        'reviews', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.review_id) FROM mdm_internal.reviews r JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer'), '[]'::jsonb),
        'facts', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(f) ORDER BY f.publication_revision, f.fact_number) FROM mdm_internal.resolution_facts f JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer'), '[]'::jsonb)
    ) INTO current_internal;
    SELECT state INTO previous_internal FROM public.e2e_customer_state_snapshot WHERE snapshot_id;
    IF FOUND THEN
        SELECT COALESCE(pg_catalog.jsonb_agg(key ORDER BY key), '[]'::jsonb)
        INTO changed_categories
        FROM pg_catalog.jsonb_object_keys(current_internal) AS categories(key)
        WHERE current_internal->key IS DISTINCT FROM previous_internal->key;
        UPDATE public.e2e_customer_state_snapshot SET state = current_internal WHERE snapshot_id;
    ELSE
        INSERT INTO public.e2e_customer_state_snapshot(state) VALUES (current_internal);
    END IF;
    RETURN pg_catalog.jsonb_build_object(
        'publication_revision', (SELECT publication_revision FROM mdm_internal.entities WHERE entity_name = 'customer'),
        'publication_count', (SELECT count(*) FROM mdm_internal.publications p JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer'),
        'observation_count', (SELECT count(*) FROM mdm_internal.publication_observations o JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer'),
        'members', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(m) ORDER BY m.source_record_id) FROM mdm_out.customer_members m), '[]'::jsonb),
        'entities', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(e) ORDER BY e.mdm_id) FROM mdm_out.customer e), '[]'::jsonb),
        'reviews', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(r) ORDER BY r.review_id) FROM mdm_out.customer_review r), '[]'::jsonb),
        'internal_changed', changed_categories
    );
END
$$;
REVOKE ALL ON FUNCTION public.e2e_customer_state() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.e2e_customer_state() TO mdm_administrator;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE
    left_id uuid;
    right_id uuid;
    third_id uuid;
    result record;
    before_preview jsonb;
    after_preview jsonb;
    scoped_preview jsonb;
BEGIN
    SELECT source_record_id INTO STRICT left_id FROM public.e2e_source_records(ARRAY[9001]::bigint[]);
    SELECT source_record_id INTO STRICT right_id FROM public.e2e_source_records(ARRAY[9002]::bigint[]);
    SELECT source_record_id INTO STRICT third_id FROM public.e2e_source_records(ARRAY[9003]::bigint[]);
    SELECT * INTO STRICT result
    FROM mdm_steward.decide('customer', left_id, right_id, 'MATCH', 0, 'confirmed by steward');
    IF result.decision_version <> 1 OR result.decision_epoch <> 1 THEN
        RAISE EXCEPTION 'initial steward decision is invalid: %', result;
    END IF;
    SELECT * INTO STRICT result
    FROM mdm_steward.decide('customer', right_id, third_id, 'NOT_MATCH', 0, 'contradictory source identity');
    IF result.decision_version <> 1 OR result.decision_epoch <> 2 THEN
        RAISE EXCEPTION 'second steward decision is invalid: %', result;
    END IF;
    before_preview := public.e2e_preview_state();
    scoped_preview := mdm.preview(
        'customer', 'scoped',
        pg_catalog.jsonb_build_object('source_record_ids', pg_catalog.jsonb_build_array(left_id::text)));
    after_preview := public.e2e_preview_state();
    IF pg_catalog.jsonb_array_length(scoped_preview->'materialized_source_record_ids') <> 3
       OR before_preview IS DISTINCT FROM after_preview THEN
        RAISE EXCEPTION 'scoped preview did not expand manual-decision neighbors read-only: %, %, %',
            scoped_preview, before_preview, after_preview;
    END IF;
    BEGIN
        PERFORM mdm_steward.decide('customer', left_id, third_id, 'MATCH', 0, 'must remain prohibited');
        RAISE EXCEPTION 'contradictory MATCH was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_DECISION_CONTRADICTION') = 0 THEN RAISE; END IF;
    END;
    SELECT * INTO STRICT result
    FROM mdm_steward.decide('customer', left_id, right_id, 'NOT_MATCH', 1, 'contradictory source identity');
    IF result.decision_version <> 2 OR result.decision_epoch <> 3 THEN
        RAISE EXCEPTION 'replacement steward decision is invalid: %', result;
    END IF;
    BEGIN
        PERFORM mdm_steward.decide('customer', left_id, right_id, 'MATCH', 1, 'stale replay');
        RAISE EXCEPTION 'stale steward decision was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_DECISION_VERSION_CONFLICT') = 0 THEN RAISE; END IF;
    END;
    BEGIN
        INSERT INTO mdm_internal.steward_decisions (entity_id, left_source_record_id, right_source_record_id, decision, decision_version, reason, created_by_name, created_as_role_name, operation_id, base_publication_revision, decision_epoch)
        SELECT e.entity_id, left_id, right_id, 'MATCH', 99, 'bypass', 'attacker', 'attacker', operation_id, 0, 99
        FROM mdm_internal.entities e
        CROSS JOIN LATERAL (SELECT operation_id FROM mdm_internal.operations LIMIT 1) o
        WHERE e.entity_name = 'customer';
        RAISE EXCEPTION 'direct steward table DML was accepted';
    EXCEPTION WHEN insufficient_privilege THEN NULL;
    END;
END
$$;
RESET ROLE;

\connect foundation postgres
INSERT INTO public.crm_customer VALUES
    (1, 'Acme One', 'same@example.test', statement_timestamp()),
    (2, 'Acme Two', 'same@example.test', statement_timestamp());
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'true'
       OR (result->>'publication_revision')::bigint <> 2
       OR result->'source_boundary'->>'completeness' <> 'PROVEN' THEN
        RAISE EXCEPTION 'insert refresh did not publish a proven boundary: %', result;
    END IF;
END
$$;
RESET ROLE;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE
    mdm_ids uuid[];
    source_record_id uuid;
    expected_source_ids jsonb;
    before_state jsonb;
    after_state jsonb;
    scoped jsonb;
BEGIN
    SELECT r.source_record_id INTO STRICT source_record_id
    FROM public.e2e_source_records(ARRAY[1]::bigint[]) AS r;
    SELECT pg_catalog.jsonb_agg(r.source_record_id::text ORDER BY r.source_record_id)
    INTO expected_source_ids
    FROM public.e2e_source_records(ARRAY[1, 2]::bigint[]) AS r;
    before_state := public.e2e_preview_state();
    scoped := mdm.preview(
        'customer', 'scoped',
        pg_catalog.jsonb_build_object('source_record_ids', pg_catalog.jsonb_build_array(source_record_id::text))
    );
    after_state := public.e2e_preview_state();
    IF scoped->>'record_count' <> '2'
       OR scoped->'materialized_source_record_ids' IS DISTINCT FROM expected_source_ids
       OR before_state IS DISTINCT FROM after_state THEN
        RAISE EXCEPTION 'scoped preview did not expand candidate-pair neighbors read-only: %, %, %',
            scoped, before_state, after_state;
    END IF;
    mdm_ids := public.e2e_active_mdm_ids(ARRAY[1, 2]::bigint[]);
    before_state := public.e2e_preview_state();
    scoped := mdm.preview('customer', 'scoped', pg_catalog.jsonb_build_object('mdm_ids', pg_catalog.to_jsonb(mdm_ids)));
    after_state := public.e2e_preview_state();
    IF cardinality(mdm_ids) NOT BETWEEN 1 AND 2
       OR scoped->>'evidence_level' <> 'exact_subjects'
       OR scoped->>'record_count' <> '2'
       OR pg_catalog.jsonb_array_length(scoped->'materialized_source_record_ids') <> 2
       OR before_state IS DISTINCT FROM after_state THEN
        RAISE EXCEPTION 'mdm_id preview did not expand active members read-only: %, %, %', scoped, before_state, after_state;
    END IF;
END
$$;
RESET ROLE;
\connect foundation postgres
DO $$
DECLARE
    binding_id uuid;
    pair_relation text;
    evidence_relation text;
    normalized_relation text;
    pair_rows jsonb;
    evidence_rows jsonb;
    expected_pair_rows jsonb;
    expected_evidence_rows jsonb;
    normalized_rows jsonb;
    member_rows jsonb;
BEGIN
    SELECT gb.graph_binding_id INTO STRICT binding_id
    FROM mdm_internal.graph_bindings gb
    JOIN mdm_internal.entities e ON e.entity_id = gb.entity_id
    WHERE e.entity_name = 'customer'
    ORDER BY gb.graph_generation DESC
    LIMIT 1;
    SELECT relation_name INTO STRICT pair_relation
    FROM mdm_internal.graph_members
    WHERE graph_binding_id = binding_id AND logical_id = 'pairs/customer';
    SELECT relation_name INTO STRICT evidence_relation
    FROM mdm_internal.graph_members
    WHERE graph_binding_id = binding_id AND logical_id = 'evidence/customer';
    SELECT relation_name INTO STRICT normalized_relation
    FROM mdm_internal.graph_members
    WHERE graph_binding_id = binding_id AND logical_id = 'normalized/email';
    EXECUTE pg_catalog.format(
        'SELECT COALESCE(pg_catalog.jsonb_agg(pg_catalog.to_jsonb(t) ORDER BY t.left_sort_key, t.right_sort_key), ''[]''::jsonb) FROM %s t',
        pair_relation
    ) INTO pair_rows;
    EXECUTE pg_catalog.format(
        'SELECT COALESCE(pg_catalog.jsonb_agg(pg_catalog.to_jsonb(t) ORDER BY t.left_sort_key, t.right_sort_key, t.rule), ''[]''::jsonb) FROM %s t',
        evidence_relation
    ) INTO evidence_rows;
    EXECUTE pg_catalog.format(
        'SELECT COALESCE(pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
            ''left_source_record_id'', l.source_record_id,
            ''right_source_record_id'', r.source_record_id,
            ''left_sort_key'', l.source_sort_key,
            ''right_sort_key'', r.source_sort_key
        ) ORDER BY l.source_sort_key, r.source_sort_key), ''[]''::jsonb)
         FROM %s l JOIN %s r
           ON l.field_name = ''email'' AND r.field_name = ''email''
          AND l.state = ''value'' AND r.state = ''value''
          AND l.canonical_bytes IS NOT NULL AND l.canonical_bytes = r.canonical_bytes
          AND l.source_sort_key < r.source_sort_key',
        normalized_relation, normalized_relation
    ) INTO expected_pair_rows;
    expected_evidence_rows := pg_catalog.jsonb_build_array(pg_catalog.jsonb_build_object(
        'left_source_record_id', expected_pair_rows->0->'left_source_record_id',
        'right_source_record_id', expected_pair_rows->0->'right_source_record_id',
        'left_sort_key', expected_pair_rows->0->'left_sort_key',
        'right_sort_key', expected_pair_rows->0->'right_sort_key',
        'rule', 'same_email', 'evidence_group', 'email', 'class', 'agree',
        'score', NULL, 'comparator', 'exact_v1', 'comparator_version', 1,
        'left_value_digest', pg_catalog.decode('60ad19203e7805d9f109a44e991b02a1362e03115f39f995d9f40f56ee175c8c', 'hex'),
        'right_value_digest', pg_catalog.decode('60ad19203e7805d9f109a44e991b02a1362e03115f39f995d9f40f56ee175c8c', 'hex')
    ));
    EXECUTE pg_catalog.format(
        'SELECT COALESCE(pg_catalog.jsonb_agg(pg_catalog.to_jsonb(t)), ''[]''::jsonb) FROM %s t WHERE t.field_name = ''email'' AND t.source_record_id IN (SELECT source_record_id FROM public.e2e_source_records(ARRAY[1, 2]::bigint[]))',
        normalized_relation
    ) INTO normalized_rows;
    SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
        'source_id', source_id, 'source_record_id', source_record_id,
        'mdm_id', mdm_id, 'active', active
    ) ORDER BY source_id) INTO member_rows
    FROM mdm_out.customer_members
    WHERE source_name = 'crm'
      AND source_record_id IN (
          SELECT source_record_id FROM public.e2e_source_records(ARRAY[1, 2]::bigint[])
      );
    IF pg_catalog.jsonb_array_length(expected_pair_rows) <> 1
       OR pair_rows IS DISTINCT FROM expected_pair_rows
       OR evidence_rows IS DISTINCT FROM expected_evidence_rows
       OR (SELECT count(*) FROM mdm_internal.resolution_facts f
           JOIN mdm_internal.entities e USING (entity_id)
           WHERE e.entity_name = 'customer' AND f.publication_revision = 2
             AND f.subject_kind = 'pair'
             AND f.subject_key = pg_catalog.convert_to(
                 (expected_pair_rows->0->>'left_source_record_id') || ':' ||
                 (expected_pair_rows->0->>'right_source_record_id'), 'UTF8')
             AND f.fact_kind = 'accepted'
             AND f.fact = '{"reason_code":"AUTOMATIC_IDENTITY","evidence_groups":["email"]}'::jsonb) <> 1
       OR (SELECT count(*) FROM mdm_out.customer_members
        WHERE source_name = 'crm' AND active
          AND source_record_id IN (SELECT source_record_id FROM public.e2e_source_records(ARRAY[1, 2]::bigint[]))) <> 2
       OR (SELECT count(DISTINCT mdm_id) FROM mdm_out.customer_members
        WHERE source_name = 'crm' AND active
          AND source_record_id IN (SELECT source_record_id FROM public.e2e_source_records(ARRAY[1, 2]::bigint[]))) <> 1 THEN
        RAISE EXCEPTION 'insert terminal output or production reader differs from the independent email expectation: pairs %, expected %, evidence %, expected %, members %, normalized %',
            pair_rows, expected_pair_rows, evidence_rows, expected_evidence_rows, member_rows, normalized_rows;
    END IF;
END
$$;

UPDATE public.crm_customer
SET email_address = 'two@example.test', updated_at = statement_timestamp()
WHERE id = 2;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'true'
       OR (result->>'publication_revision')::bigint <> 3 THEN
        RAISE EXCEPTION 'update refresh did not publish the split: %', result;
    END IF;
END
$$;
RESET ROLE;
\connect foundation postgres
DO $$
BEGIN
    IF (SELECT count(DISTINCT mdm_id) FROM mdm_out.customer_members
        WHERE source_name = 'crm' AND active
          AND source_record_id IN (SELECT source_record_id FROM public.e2e_source_records(ARRAY[1, 2]::bigint[]))) <> 2 THEN
        RAISE EXCEPTION 'update reference result did not split the records';
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM mdm_out.customer_members m
        JOIN mdm_internal.entities e ON e.entity_name = 'customer'
        WHERE m.source_name = 'crm'
          AND m.last_change_revision < e.publication_revision
    ) THEN
        RAISE EXCEPTION 'public row last-change revisions were not retained independently of the current entity revision';
    END IF;
END
$$;

CREATE TABLE public.release_output_checkpoint AS
SELECT e.publication_revision,
       (SELECT count(*) FROM mdm_internal.publications p WHERE p.entity_id = e.entity_id) AS publication_count,
       (SELECT COALESCE(jsonb_agg(to_jsonb(o) ORDER BY o.mdm_id), '[]'::jsonb) FROM mdm_out.customer o) AS entities,
       (SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.source_record_id), '[]'::jsonb) FROM mdm_out.customer_members m) AS members,
       (SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY r.review_id), '[]'::jsonb) FROM mdm_out.customer_review r) AS reviews
FROM mdm_internal.entities e WHERE e.entity_name = 'customer';
CREATE FUNCTION public.fail_release_output()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'injected publication failure';
END
$$;
CREATE TRIGGER fail_release_output
BEFORE INSERT OR UPDATE ON mdm_out.customer
FOR EACH ROW EXECUTE FUNCTION public.fail_release_output();
UPDATE public.crm_customer
SET display_name = 'Acme Updated', updated_at = statement_timestamp()
WHERE id = 2;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE before_state jsonb; after_state jsonb;
BEGIN
    before_state := public.e2e_customer_state();
    BEGIN
        PERFORM mdm.refresh('customer', 'ALLOW');
        RAISE EXCEPTION 'refresh unexpectedly passed the injected publication failure';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'injected publication failure') = 0 THEN RAISE; END IF;
    END;
    after_state := public.e2e_customer_state();
    IF after_state->'internal_changed' <> '[]'::jsonb
       OR after_state->'entities' IS DISTINCT FROM before_state->'entities'
       OR after_state->'members' IS DISTINCT FROM before_state->'members'
       OR after_state->'reviews' IS DISTINCT FROM before_state->'reviews'
       OR after_state->'publication_revision' IS DISTINCT FROM before_state->'publication_revision'
       OR after_state->'publication_count' IS DISTINCT FROM before_state->'publication_count'
       OR after_state->'observation_count' IS DISTINCT FROM before_state->'observation_count' THEN
        RAISE EXCEPTION 'failed publication changed internal or public semantic state: before %, after %', before_state, after_state;
    END IF;
END
$$;
RESET ROLE;
\connect foundation postgres
DO $$
DECLARE checkpoint record;
BEGIN
    SELECT * INTO STRICT checkpoint FROM public.release_output_checkpoint;
    IF (SELECT publication_revision FROM mdm_internal.entities WHERE entity_name = 'customer') <> checkpoint.publication_revision
       OR (SELECT count(*) FROM mdm_internal.publications p JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'customer') <> checkpoint.publication_count
       OR (SELECT COALESCE(jsonb_agg(to_jsonb(o) ORDER BY o.mdm_id), '[]'::jsonb) FROM mdm_out.customer o) IS DISTINCT FROM checkpoint.entities
       OR (SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.source_record_id), '[]'::jsonb) FROM mdm_out.customer_members m) IS DISTINCT FROM checkpoint.members
       OR (SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY r.review_id), '[]'::jsonb) FROM mdm_out.customer_review r) IS DISTINCT FROM checkpoint.reviews THEN
        RAISE EXCEPTION 'failed refresh changed publication state';
    END IF;
END
$$;
DROP TRIGGER fail_release_output ON mdm_out.customer;
DROP FUNCTION public.fail_release_output();
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'true'
       OR (result->>'publication_revision')::bigint <> 4
       OR result->'source_boundary'->>'completeness' <> 'PROVEN'
       OR length(result->>'source_boundary_digest') <> 64 THEN
        RAISE EXCEPTION 'retry skipped a source update after rollback: %', result;
    END IF;
END
$$;
RESET ROLE;
\connect foundation postgres
DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_out.customer WHERE name = 'Acme Updated') <> 1 THEN
        RAISE EXCEPTION 'retry did not publish the updated golden value';
    END IF;
END
$$;
DROP TABLE public.release_output_checkpoint;

DELETE FROM public.crm_customer WHERE id = 1;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb; before_state jsonb; output_state jsonb;
BEGIN
    result := mdm.refresh('customer', 'ALLOW');
    IF result->>'changed' <> 'true'
       OR (result->>'publication_revision')::bigint <> 5 THEN
        RAISE EXCEPTION 'delete refresh did not publish: %', result;
    END IF;
    before_state := public.e2e_customer_state();
    result := mdm.refresh('customer', 'ALLOW');
    output_state := public.e2e_customer_state();
    IF result->>'changed' <> 'false'
       OR (result->>'publication_revision')::bigint <> 5
       OR output_state->'internal_changed' <> '[]'::jsonb
       OR output_state->'entities' IS DISTINCT FROM before_state->'entities'
       OR output_state->'members' IS DISTINCT FROM before_state->'members'
       OR output_state->'reviews' IS DISTINCT FROM before_state->'reviews'
       OR output_state->'publication_count' IS DISTINCT FROM before_state->'publication_count'
       OR (output_state->>'observation_count')::bigint <> (before_state->>'observation_count')::bigint + 1 THEN
        RAISE EXCEPTION 'no-op refresh changed the publication: changed %, revision %, internal categories %',
            result->>'changed', result->>'publication_revision', output_state->'internal_changed';
    END IF;
END
$$;
RESET ROLE;
\connect foundation postgres
DROP FUNCTION public.e2e_customer_state();
DROP TABLE public.e2e_customer_state_snapshot;
DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_out.customer_members
        WHERE source_name = 'crm' AND active
          AND source_record_id IN (SELECT source_record_id FROM public.e2e_source_records(ARRAY[2]::bigint[]))) <> 1
       OR (SELECT count(*) FROM mdm_out.customer WHERE name = 'Acme Updated') <> 1 THEN
        RAISE EXCEPTION 'delete reference result is incomplete';
    END IF;
END
$$;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SELECT mdm_admin.verify_installation();
RESET ROLE;

\connect foundation postgres
DO $$
DECLARE
    operation mdm_internal.operations%ROWTYPE;
BEGIN
    SELECT * INTO STRICT operation FROM mdm_internal.operations WHERE operation_kind = 'foundation_check';
    IF operation.operation_kind <> 'foundation_check'
       OR operation.status <> 'succeeded'
       OR operation.result_code <> 'MDM_OK'
       OR operation.actor_name <> 'mdm_test_login'
       OR operation.actor_role_name <> 'mdm_administrator'
       OR operation.completed_at IS NULL
       OR operation.outcome #>> '{external_graph_refresh,enabled}' <> 'true'
       OR operation.outcome #>> '{output_delta_consumer,enabled}' <> 'true' THEN
        RAISE EXCEPTION 'operation record is invalid: %', row_to_json(operation);
    END IF;
    IF pg_catalog.to_regprocedure('mdm_admin.verify_installation(text)') IS NOT NULL THEN
        RAISE EXCEPTION 'forgeable helper overload exists';
    END IF;
END
$$;

DO $$
BEGIN
    BEGIN
        PERFORM mdm_admin.verify_installation();
        RAISE EXCEPTION 'superuser call was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_UNAUTHORIZED') = 0 THEN
            RAISE;
        END IF;
    END;
END
$$;

GRANT USAGE ON SCHEMA mdm_admin TO mdm_bypass;
GRANT EXECUTE ON FUNCTION mdm_admin.verify_installation() TO mdm_bypass;
\connect foundation mdm_test_login
SET ROLE mdm_bypass;
DO $$
BEGIN
    BEGIN
        PERFORM mdm_admin.verify_installation();
        RAISE EXCEPTION 'BYPASSRLS call was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_UNAUTHORIZED') = 0 THEN
            RAISE;
        END IF;
    END;
END
$$;
RESET ROLE;

\connect foundation postgres
DO $$
DECLARE
    denied_role name;
BEGIN
    FOREACH denied_role IN ARRAY ARRAY[
        'mdm_configurator'::name,
        'mdm_steward_role'::name,
        'mdm_output_reader'::name,
        'mdm_explanation_reader'::name
    ] LOOP
        IF pg_catalog.has_schema_privilege(denied_role, 'mdm_admin', 'USAGE')
           OR pg_catalog.has_function_privilege(
               denied_role,
               'mdm_admin.verify_installation()',
               'EXECUTE'
           ) THEN
            RAISE EXCEPTION '% has an action grant', denied_role;
        END IF;
    END LOOP;
END
$$;

-- Inherited ownership bypasses ordinary RLS, even when the owner OID differs.
CREATE ROLE mdm_source_owner NOLOGIN NOSUPERUSER NOBYPASSRLS;
GRANT mdm_source_owner TO mdm_administrator WITH INHERIT TRUE, SET FALSE;
CREATE TABLE public.rls_customer (LIKE public.crm_customer INCLUDING ALL);
INSERT INTO public.rls_customer VALUES (1, 'hidden', 'hidden@example.test', now());
ALTER TABLE public.rls_customer OWNER TO mdm_source_owner;
ALTER TABLE public.rls_customer ENABLE ROW LEVEL SECURITY;
CREATE POLICY deny_all ON public.rls_customer USING (false);

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE proposed jsonb;
BEGIN
    IF (SELECT count(*) FROM public.rls_customer) <> 1 THEN
        RAISE EXCEPTION 'inherited-owner fixture did not bypass RLS';
    END IF;
    proposed := jsonb_set(jsonb_set(mdm.describe('customer', 'definition'),
        '{name}', '"rls_customer"'), '{sources,0,relation}', '"public.rls_customer"');
    BEGIN
        PERFORM mdm.create(proposed);
        RAISE EXCEPTION 'inherited owner bypass was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_SOURCE_INVALID') = 0 THEN RAISE; END IF;
    END;
END
$$;

\connect foundation postgres
ALTER TABLE public.rls_customer FORCE ROW LEVEL SECURITY;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
BEGIN;
DO $$
DECLARE proposed jsonb;
BEGIN
    IF (SELECT count(*) FROM public.rls_customer) <> 0 THEN
        RAISE EXCEPTION 'FORCE RLS did not apply to selected execution role';
    END IF;
    proposed := jsonb_set(jsonb_set(mdm.describe('customer', 'definition'),
        '{name}', '"rls_customer"'), '{sources,0,relation}', '"public.rls_customer"');
    PERFORM mdm.create(proposed);
END
$$;
ROLLBACK;

\connect foundation postgres
DROP TABLE public.rls_customer;
REVOKE mdm_source_owner FROM mdm_administrator;
DROP ROLE mdm_source_owner;

CREATE TABLE public.typed_customer (
    id bigint PRIMARY KEY, label varchar(80), amount numeric(12, 2), changed timestamp(3)
);
INSERT INTO public.typed_customer VALUES (1, 'typed', 1.00, statement_timestamp());
GRANT SELECT, MAINTAIN ON public.typed_customer TO mdm_administrator;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
BEGIN;
SELECT * FROM mdm.create(mdm.entity(
    name => 'typed_customer',
    sources => ARRAY[mdm.source(
        name => 'typed', relation => 'public.typed_customer'::regclass,
        source_id => ARRAY['id'], mode => 'tracked',
        fields => '{"label":"label","amount":"amount","changed":"changed"}'::jsonb,
        row_changed_at => 'changed')],
    fields => ARRAY[
        mdm.field(name => 'label', type => 'text', cleaner => 'none'),
        mdm.field(name => 'amount', type => 'numeric', cleaner => 'none'),
        mdm.field(name => 'changed', type => 'timestamp', cleaner => 'none')],
    matches => ARRAY[mdm.match(name => 'same_label', fields => ARRAY['label'],
        comparison => 'exact', strength => 'identity', evidence_group => 'label')],
    golden_values => ARRAY[]::jsonb[]));
SELECT mdm.refresh('typed_customer', 'ALLOW');
DO $$
DECLARE
    customer_record_id uuid;
BEGIN
    SELECT source_record_id INTO STRICT customer_record_id
    FROM public.e2e_source_records(ARRAY[9001]::bigint[]);
    BEGIN
        PERFORM mdm.preview(
            'typed_customer', 'scoped',
            pg_catalog.jsonb_build_object('source_record_ids', pg_catalog.jsonb_build_array(customer_record_id::text))
        );
        RAISE EXCEPTION 'scoped preview accepted a source record from another entity';
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM <> 'MDM_SOURCE_INVALID: invalid source contract: scoped preview cannot include a record from another entity' THEN
            RAISE;
        END IF;
    END;
END
$$;
ROLLBACK;
RESET ROLE;
\connect foundation postgres
DROP FUNCTION public.e2e_source_records(bigint[]);
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
BEGIN
    BEGIN
        PERFORM mdm.create(jsonb_set(mdm.describe('customer', 'definition'),
            '{sources,0,fields,email}', '{"value":"email_address","state":123}'), 3);
        RAISE EXCEPTION 'non-string state column was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_SOURCE_INVALID') = 0 THEN RAISE; END IF;
    END;
END
$$;

\connect foundation postgres
DROP TABLE public.typed_customer;
REVOKE SELECT, MAINTAIN ON public.crm_customer FROM mdm_administrator;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
BEGIN
    BEGIN
        PERFORM mdm.create(mdm.describe('customer', 'definition'), 3);
        RAISE EXCEPTION 'revoked source SELECT was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_SOURCE_INVALID') = 0 THEN RAISE; END IF;
    END;
END
$$;
\connect foundation postgres
GRANT SELECT, MAINTAIN ON public.crm_customer TO mdm_administrator;

-- Capability availability is operational state, outside the definition identity.
BEGIN;
CREATE OR REPLACE FUNCTION pgtrickle.integration_capabilities()
RETURNS TABLE (capability text, major_version smallint, minor_version smallint, enabled boolean, details jsonb)
LANGUAGE sql
AS $$
    VALUES ('external_graph_refresh', 1::smallint, 1::smallint, true, '{"changed":true}'::jsonb),
           ('output_delta_consumer', 1::smallint, 1::smallint, false, '{}'::jsonb)
$$;
SET SESSION AUTHORIZATION mdm_test_login;
SET ROLE mdm_administrator;
DO $$
DECLARE result record;
BEGIN
    SELECT * INTO STRICT result FROM mdm.create(mdm.describe('customer', 'definition'), 3);
    IF result.changed OR result.desired_version <> 3 THEN
        RAISE EXCEPTION 'capability change changed the definition identity';
    END IF;
END
$$;
RESET ROLE;
RESET SESSION AUTHORIZATION;
ROLLBACK;

-- Entity drop validates the complete binding before changing graph or MDM state.
BEGIN;
SET SESSION AUTHORIZATION mdm_test_login;
SET ROLE mdm_administrator;
DO $$
DECLARE proposed jsonb;
BEGIN
    proposed := jsonb_set(mdm.describe('customer', 'definition'), '{name}', '"drop_probe"');
    PERFORM mdm.create(proposed);
END
$$;
DO $$
BEGIN
    BEGIN
        PERFORM mdm_admin.drop_entity('drop_probe', 'wrong confirmation');
        RAISE EXCEPTION 'entity drop accepted the wrong confirmation';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_GRAPH_LIFECYCLE') = 0 THEN RAISE; END IF;
    END;
END
$$;
RESET ROLE;
RESET SESSION AUTHORIZATION;
CREATE TABLE public.e2e_drop_member_names AS
SELECT DISTINCT gm.relation_name
FROM mdm_internal.graph_members gm
JOIN mdm_internal.graph_bindings gb USING (graph_binding_id)
JOIN mdm_internal.entities e USING (entity_id)
WHERE e.entity_name = 'drop_probe';
WITH target AS (
    SELECT gm.graph_binding_id, gm.logical_id
    FROM mdm_internal.graph_members gm
    JOIN mdm_internal.graph_bindings gb USING (graph_binding_id)
    JOIN mdm_internal.entities e USING (entity_id)
    WHERE e.entity_name = 'drop_probe' AND gb.graph_generation = (
        SELECT max(b.graph_generation)
        FROM mdm_internal.graph_bindings b
        WHERE b.entity_id = e.entity_id
    )
    ORDER BY gm.topological_ordinal
    LIMIT 1
)
UPDATE mdm_internal.graph_members gm
SET relation_oid = (gm.relation_oid::bigint + 1000000)::oid
FROM target
WHERE gm.graph_binding_id = target.graph_binding_id
  AND gm.logical_id = target.logical_id;
SET SESSION AUTHORIZATION mdm_test_login;
SET ROLE mdm_administrator;
DO $$
BEGIN
    BEGIN
        PERFORM mdm_admin.drop_entity('drop_probe', 'drop_probe');
        RAISE EXCEPTION 'entity drop accepted a changed member OID';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_GRAPH_LIFECYCLE') = 0 THEN RAISE; END IF;
    END;
END
$$;
RESET ROLE;
RESET SESSION AUTHORIZATION;
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM mdm_internal.entities WHERE entity_name = 'drop_probe')
       OR EXISTS (
           SELECT FROM public.e2e_drop_member_names n
           WHERE pg_catalog.to_regclass(n.relation_name) IS NULL
       ) THEN
        RAISE EXCEPTION 'failed entity drop did not roll back every graph and output change';
    END IF;
END
$$;
UPDATE mdm_internal.graph_members gm
SET relation_oid = pg_catalog.to_regclass(gm.relation_name)::oid
WHERE pg_catalog.to_regclass(gm.relation_name) IS NOT NULL
  AND gm.relation_oid <> pg_catalog.to_regclass(gm.relation_name)::oid;
SET SESSION AUTHORIZATION mdm_test_login;
SET ROLE mdm_administrator;
SELECT mdm_admin.drop_entity('drop_probe', 'drop_probe');
RESET ROLE;
RESET SESSION AUTHORIZATION;
DO $$
BEGIN
    IF EXISTS (SELECT FROM mdm_internal.entities WHERE entity_name = 'drop_probe')
       OR EXISTS (
           SELECT FROM public.e2e_drop_member_names n
           WHERE pg_catalog.to_regclass(n.relation_name) IS NOT NULL
       ) THEN
        RAISE EXCEPTION 'confirmed entity drop left MDM state or graph members behind';
    END IF;
END
$$;
ROLLBACK;
DO $$
BEGIN
    IF EXISTS (SELECT FROM mdm_internal.entities WHERE entity_name = 'drop_probe')
       OR pg_catalog.to_regclass('mdm_out.customer') IS NULL THEN
        RAISE EXCEPTION 'lifecycle qualification did not roll back cleanly';
    END IF;
END
$$;

\connect postgres postgres
DROP DATABASE graph_conformance WITH (FORCE);
\connect foundation postgres
