\set ON_ERROR_STOP on
\connect foundation postgres
CREATE TABLE public.:"source_table" (id bigint PRIMARY KEY, display_name text, email_address text, updated_at timestamptz NOT NULL);
INSERT INTO public.:"source_table" SELECT id, 'Record ' || id, id || '@publication.test', '2026-10-18 00:00:00+00'
FROM generate_series(1, :population) id;
GRANT SELECT, MAINTAIN ON public.:"source_table" TO mdm_administrator;
CREATE OR REPLACE FUNCTION public.publication_probe_refresh(entity_name text, population integer)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE result jsonb; actual jsonb;
BEGIN
    result := mdm.refresh(entity_name, 'ALLOW');
    SELECT jsonb_object_agg(key, value) INTO actual FROM jsonb_each(result)
    WHERE key IN ('changed', 'publication_revision', 'active_records', 'identities', 'open_reviews', 'resolver_strategy', 'delta_lag');
    IF actual IS DISTINCT FROM jsonb_build_object('changed', true, 'publication_revision', 1,
        'active_records', population, 'identities', population, 'open_reviews', 0, 'resolver_strategy', 'full', 'delta_lag', 0) THEN
        RAISE EXCEPTION 'publication probe refresh mismatch: %', actual;
    END IF;
    RAISE NOTICE 'PG_MDM_PUBLICATION_PROBE population=% pid=% result=%', population, pg_backend_pid(), actual;
END
$$;
REVOKE ALL ON FUNCTION public.publication_probe_refresh(text, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.publication_probe_refresh(text, integer) TO mdm_administrator;
ALTER ROLE mdm_test_login SET session_preload_libraries = 'auto_explain';
ALTER ROLE mdm_test_login SET auto_explain.log_min_duration = '0';
ALTER ROLE mdm_test_login SET auto_explain.log_nested_statements = 'on';
ALTER ROLE mdm_test_login SET auto_explain.log_analyze = 'on';
ALTER ROLE mdm_test_login SET auto_explain.log_timing = 'off';
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET jit = off;
SELECT * FROM mdm.create(mdm.entity(
    name => :'entity_name', sources => ARRAY[mdm.source(name => 'crm',
        relation => :'source_relation'::regclass, source_id => ARRAY['id'], mode => 'tracked',
        fields => '{"name":"display_name","email":"email_address"}'::jsonb, row_changed_at => 'updated_at')],
    fields => ARRAY[mdm.field(name => 'name', type => 'text', cleaner => 'company_name'),
        mdm.field(name => 'email', type => 'text', cleaner => 'email')],
    matches => ARRAY[mdm.match(name => 'email', fields => ARRAY['email'], comparison => 'exact', strength => 'identity',
        evidence_group => 'email', candidate => '{"kind":"exact","field":"email"}'::jsonb)],
    golden_values => ARRAY[mdm.golden_value(field => 'name', policy => 'first_non_null')]
));
SELECT public.publication_probe_refresh(:'entity_name', :population);
\connect foundation postgres
CREATE OR REPLACE FUNCTION public.assert_publication_probe(entity_name text, population integer)
RETURNS jsonb LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE entity uuid; source_identity uuid; output text; expected jsonb; actual jsonb; entities jsonb; members jsonb;
BEGIN
    SELECT entity_id INTO STRICT entity FROM mdm_internal.entities e WHERE e.entity_name = assert_publication_probe.entity_name;
    SELECT source_identity_id INTO STRICT source_identity FROM mdm_internal.source_identities WHERE entity_id = entity;
    SELECT output_name::text INTO STRICT output FROM mdm_internal.output_names WHERE entity_id = entity AND output_kind = 'entity';
    WITH rows AS (
        SELECT i, r.source_record_id, m.mdm_id, pgtrickle.encode_row_id_v2('SCAN_KEY', ROW(entity, source_identity, i)) AS key
        FROM generate_series(1::bigint, population) i
        LEFT JOIN mdm_internal.source_records r ON r.entity_id = entity
            AND r.source_record_key = pgtrickle.encode_row_id_v2('SCAN_KEY', ROW(entity, source_identity, i))
        LEFT JOIN mdm_internal.memberships m ON m.entity_id = entity AND m.source_record_id = r.source_record_id
    ), memberships AS (
        SELECT source_record_id, mdm_id, i, key, jsonb_build_object('entity_id', entity, 'source_record_id', source_record_id,
            'source_name', 'crm', 'source_id', jsonb_build_object('source_record_key', encode(key, 'hex')),
            'mdm_id', mdm_id, 'active', true, 'first_membership_revision', 1, 'last_membership_revision', 1,
            'membership_reason', 'new', 'last_change_revision', 1) AS row FROM rows
    ) SELECT jsonb_build_object(
        'registry', jsonb_agg(jsonb_build_object('entity_id', entity, 'mdm_id', mdm_id, 'created_revision', 1,
            'retired_revision', NULL, 'status', 'active') ORDER BY mdm_id),
        'memberships', jsonb_agg(row ORDER BY source_record_id),
        'members', jsonb_agg(row - 'entity_id' ORDER BY source_record_id),
        'entities', jsonb_agg(jsonb_build_object('mdm_id', mdm_id, 'name', 'Record ' || i,
            'member_count', 1, 'has_review', false, 'last_change_revision', 1) ORDER BY mdm_id),
        'golden', jsonb_agg(jsonb_build_object('entity_id', entity, 'publication_revision', 1, 'mdm_id', mdm_id,
            'field_name', 'name', 'value', 'Record ' || i, 'normalized_value', 'record ' || i,
            'status', 'selected', 'winning_source_record_id', source_record_id, 'policy', 'first_non_null',
            'policy_version', 1, 'tie_break', 'source_priority,source_sort_key',
            'contributors', jsonb_build_array(source_record_id), 'definition_version', 1) ORDER BY mdm_id)
    ) INTO expected FROM memberships;
    EXECUTE format('SELECT jsonb_agg(to_jsonb(t) ORDER BY mdm_id) FROM mdm_out.%I t', output) INTO entities;
    EXECUTE format('SELECT jsonb_agg(to_jsonb(t) ORDER BY source_record_id) FROM mdm_out.%I t', output || '_members') INTO members;
    actual := jsonb_build_object('entities', entities, 'members', members,
        'registry', (SELECT jsonb_agg(to_jsonb(t) ORDER BY mdm_id) FROM mdm_internal.identity_registry t WHERE entity_id = entity),
        'memberships', (SELECT jsonb_agg(to_jsonb(t) ORDER BY source_record_id) FROM mdm_internal.memberships t WHERE entity_id = entity),
        'golden', (SELECT jsonb_agg(to_jsonb(t) ORDER BY mdm_id) FROM mdm_internal.golden_provenance t WHERE entity_id = entity));
    IF actual IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'complete publication probe rows differ: actual %, expected %', actual, expected;
    END IF;
    RETURN actual;
END
$$;
SELECT public.assert_publication_probe(:'entity_name', :population);
\echo 'PASS: complete registry, memberships, golden provenance and public entity/member rows'
