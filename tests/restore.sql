\set ON_ERROR_STOP on

SELECT 'public.crm_customer'::regclass::oid <> :original_source_oid
   AND 'mdm_administrator'::regrole::oid <> :original_role_oid AS new_bindings
\gset
\if :new_bindings
\else
\quit 1
\endif

DO $$
BEGIN
    IF (SELECT count(*) FROM mdm_internal.entities WHERE entity_name = 'customer' AND desired_version = 4) <> 1
       OR (SELECT count(*) FROM mdm_internal.definitions) <> 4
       OR (SELECT count(*) FROM mdm_internal.definition_artifacts) <> 4
       OR (SELECT count(*) FROM mdm_internal.source_identities) <> 1
       OR (SELECT count(*) FROM mdm_internal.output_names) <> 3
       OR (SELECT count(*) FROM mdm_internal.operations WHERE status = 'succeeded' AND actor_name = 'mdm_test_login') <> 6
       OR (SELECT count(*) FROM mdm_internal.source_records WHERE source_record_key = '\x01020304'::bytea) <> 1 THEN
        RAISE EXCEPTION 'durable catalog data did not survive restore';
    END IF;
    IF EXISTS (SELECT FROM mdm_internal.source_bindings)
       OR EXISTS (SELECT FROM mdm_internal.execution_role_bindings) THEN
        RAISE EXCEPTION 'database-local bindings were included in the dump';
    END IF;
END
$$;
GRANT EXECUTE ON FUNCTION mdm_admin.rebind(text) TO mdm_administrator;

\connect restored mdm_test_login
SET ROLE mdm_administrator;
DO $$
BEGIN
    BEGIN
        PERFORM mdm.describe('customer', 'definition');
        RAISE EXCEPTION 'missing role binding was accepted';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_UNAUTHORIZED') = 0 THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM mdm_admin.rebind('customer');
        RAISE EXCEPTION 'application session rebound restored definitions';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_UNAUTHORIZED') = 0 THEN RAISE; END IF;
    END;
END
$$;

\connect restored postgres
BEGIN;
ALTER TABLE public.crm_customer ALTER COLUMN id TYPE text;
SET ROLE mdm_administrator;
DO $$
BEGIN
    BEGIN
        PERFORM mdm_admin.rebind('customer');
        RAISE EXCEPTION 'changed source key contract was rebound';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'MDM_SOURCE_INVALID') = 0 THEN RAISE; END IF;
    END;
END
$$;
RESET ROLE;
ROLLBACK;

SET ROLE mdm_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm_admin.rebind('customer');
    IF result->>'entity_name' IS DISTINCT FROM 'customer'
       OR result->>'desired_version' IS DISTINCT FROM '4'
       OR result->>'rebound_sources' IS DISTINCT FROM '1' THEN
        RAISE EXCEPTION 'rebind result is invalid: %', result;
    END IF;
END
$$;
RESET ROLE;
DO $$
BEGIN
    IF (SELECT role_oid FROM mdm_internal.execution_role_bindings) IS DISTINCT FROM 'mdm_administrator'::regrole::oid
       OR (SELECT relation_oid FROM mdm_internal.source_bindings) IS DISTINCT FROM 'public.crm_customer'::regclass::oid THEN
        RAISE EXCEPTION 'rebind did not record current database-local OIDs';
    END IF;
END
$$;

\connect restored mdm_test_login
SET ROLE mdm_administrator;
DO $$
DECLARE result record;
BEGIN
    SELECT * INTO STRICT result FROM mdm.create(mdm.describe('customer', 'definition'), 4);
    IF result.changed OR result.desired_version <> 4 THEN
        RAISE EXCEPTION 'rebind changed the restored definition';
    END IF;
    PERFORM mdm_admin.verify_installation();
END
$$;
