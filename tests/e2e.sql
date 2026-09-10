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
    IF pg_catalog.to_regclass('mdm_internal.source_records') IS NULL THEN
        RAISE EXCEPTION 'source_records table is missing';
    END IF;
    IF pg_catalog.to_regtype('mdm_internal.normalized_value') IS NULL THEN
        RAISE EXCEPTION 'normalized_value type is missing';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_extension
        WHERE extname = 'pg_mdm' AND extversion = '0.6.0'
    ) THEN
        RAISE EXCEPTION 'pg_mdm 0.6.0 is not installed';
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

\connect postgres postgres
CREATE DATABASE upgrade_direct;
\connect upgrade_direct postgres
CREATE EXTENSION pg_trickle;
CREATE EXTENSION pg_mdm VERSION '0.2.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.3.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.4.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.5.0';
ALTER EXTENSION pg_mdm UPDATE TO '0.6.0';
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extversion = '0.6.0')
       OR pg_catalog.to_regclass('mdm_internal.source_records') IS NULL
       OR pg_catalog.to_regtype('mdm_internal.normalized_value') IS NULL THEN
        RAISE EXCEPTION 'direct 0.2.0 to 0.6.0 upgrade did not install v0.6 catalog';
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
GRANT USAGE ON SCHEMA mdm TO mdm_administrator;
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
CREATE TABLE public.crm_customer (
    id bigint PRIMARY KEY,
    display_name text NOT NULL,
    email_address text,
    updated_at timestamptz NOT NULL
);
GRANT SELECT ON public.crm_customer TO mdm_administrator;

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
                evidence_group => 'email'
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
            strength => 'identity', evidence_group => 'email')],
        golden_values => ARRAY[mdm.golden_value(
            field => 'name', policy => 'prefer_source', sources => ARRAY['crm'])]
    ) AS definition
)
SELECT NOT changed AND desired_version = 1 AS v02_noop_ok
FROM mdm.create((SELECT definition FROM proposed), 1, NULL)
\gset
\if :v02_noop_ok
\else
\quit 1
\endif

DO $$
DECLARE
    described jsonb;
    result record;
BEGIN
    described := mdm.describe('customer', 'definition');
    IF described->>'name' IS DISTINCT FROM 'customer' THEN
        RAISE EXCEPTION 'definition description did not round trip';
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
    IF (SELECT count(*) FROM mdm_internal.operations) <> 4
       OR (SELECT count(*) FROM mdm_internal.operations
           WHERE operation_kind = 'create' AND status = 'succeeded'
             AND actor_name = 'mdm_test_login' AND actor_role_name = 'mdm_administrator') <> 4 THEN
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
DO $$
DECLARE
    norm mdm_internal.normalized_value;
    e_id uuid;
    s_id uuid;
BEGIN
    norm := mdm_internal.normalize_text('  Foo  Bar  ', 'text', 1, 'present', '{}'::jsonb);
    IF norm.state <> 'value' OR norm.normalized <> 'foo bar' OR norm.canonical_bytes <> '\x0101666f6f20626172'::bytea THEN
        RAISE EXCEPTION 'normalize_text failed: %', norm;
    END IF;

    norm := mdm_internal.normalize_date('2024-02-29'::date, 'date', 1, 'present', '{}'::jsonb);
    IF norm.state <> 'value' OR norm.normalized <> '2024-02-29' OR norm.canonical_bytes <> '\x0102323032342d30322d3239'::bytea THEN
        RAISE EXCEPTION 'normalize_date failed: %', norm;
    END IF;

    SELECT entity_id INTO STRICT e_id FROM mdm_internal.entities WHERE entity_name = 'customer';
    SELECT source_identity_id INTO STRICT s_id FROM mdm_internal.source_identities WHERE entity_id = e_id AND source_name = 'crm';
    INSERT INTO mdm_internal.source_records (entity_id, source_identity_id, source_record_key, active)
    VALUES (e_id, s_id, '\x01020304'::bytea, true),
           (e_id, s_id, '\x01020305'::bytea, true),
           (e_id, s_id, '\x01020306'::bytea, true);
END
$$;

GRANT USAGE ON SCHEMA mdm_steward TO mdm_administrator;
GRANT EXECUTE ON FUNCTION mdm_steward.decide(text, uuid, uuid, text, bigint, text) TO mdm_administrator;
CREATE FUNCTION public.e2e_steward_ids()
RETURNS TABLE(source_record_id uuid, source_record_key bytea)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, mdm_internal
AS $$
    SELECT source_record_id, source_record_key
    FROM mdm_internal.source_records
    WHERE source_record_key IN ('\x01020304'::bytea, '\x01020305'::bytea, '\x01020306'::bytea)
$$;
REVOKE ALL ON FUNCTION public.e2e_steward_ids() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.e2e_steward_ids() TO mdm_administrator;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE
    left_id uuid;
    right_id uuid;
    third_id uuid;
    result record;
BEGIN
    SELECT source_record_id INTO STRICT left_id
    FROM public.e2e_steward_ids()
    WHERE source_record_key = '\x01020304'::bytea;
    SELECT source_record_id INTO STRICT right_id
    FROM public.e2e_steward_ids()
    WHERE source_record_key = '\x01020305'::bytea;
    SELECT source_record_id INTO STRICT third_id
    FROM public.e2e_steward_ids()
    WHERE source_record_key = '\x01020306'::bytea;
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
DROP FUNCTION public.e2e_steward_ids();

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
GRANT SELECT ON public.typed_customer TO mdm_administrator;
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
ROLLBACK;
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
REVOKE SELECT ON public.crm_customer FROM mdm_administrator;
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
GRANT SELECT ON public.crm_customer TO mdm_administrator;

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
