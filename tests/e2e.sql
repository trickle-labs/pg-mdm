\set ON_ERROR_STOP on

CREATE DATABASE foundation;
\connect foundation postgres
CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm;

DO $$
DECLARE
    schema_count integer;
BEGIN
    SELECT count(*) INTO schema_count
    FROM pg_catalog.pg_namespace
    WHERE nspname IN ('mdm', 'mdm_out', 'mdm_steward', 'mdm_admin', 'mdm_internal');
    IF schema_count <> 5 THEN
        RAISE EXCEPTION 'expected five MDM schemas, found %', schema_count;
    END IF;
    IF pg_catalog.to_regclass('mdm_internal.operations') IS NULL THEN
        RAISE EXCEPTION 'operations table is missing';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_extension
        WHERE extname = 'pg_mdm' AND extversion = '0.1.0'
    ) THEN
        RAISE EXCEPTION 'pg_mdm 0.1.0 is not installed';
    END IF;
END
$$;

\connect postgres postgres
CREATE DATABASE capability_errors;
\connect capability_errors postgres
CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm;

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
        WHERE major_version = 1 AND minor_version = 0 AND NOT enabled) <> 2 THEN
        RAISE EXCEPTION 'baseline capabilities do not match';
    END IF;
    BEGIN
        PERFORM mdm_internal.require_graph_v1();
        RAISE EXCEPTION 'Graph V1 gate did not fail';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_PGT_CAPABILITY_DISABLED') = 0 THEN
            RAISE;
        END IF;
    END;
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
GRANT EXECUTE ON FUNCTION mdm_admin.verify_installation() TO mdm_administrator;

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
DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_internal.operations) <> 0 THEN
        RAISE EXCEPTION 'rolled-back operation survived';
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
    SELECT * INTO STRICT operation FROM mdm_internal.operations;
    IF operation.operation_kind <> 'foundation_check'
       OR operation.status <> 'succeeded'
       OR operation.result_code <> 'MDM_OK'
       OR operation.actor_name <> 'mdm_test_login'
       OR operation.actor_role_name <> 'mdm_administrator'
       OR operation.completed_at IS NULL
       OR operation.outcome #>> '{external_graph_refresh,enabled}' <> 'false'
       OR operation.outcome #>> '{output_delta_consumer,enabled}' <> 'false' THEN
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
