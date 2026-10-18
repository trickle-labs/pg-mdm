\set ON_ERROR_STOP on
\connect foundation postgres

CREATE TABLE public.source_sync_crm (
    tenant bigint NOT NULL, id bigint NOT NULL, deleted boolean,
    display_name text NOT NULL, email_address text, updated_at timestamptz NOT NULL,
    PRIMARY KEY (tenant, id)
);
CREATE TABLE public.source_sync_erp (LIKE public.source_sync_crm INCLUDING ALL);
CREATE TABLE public.source_sync_expected (
    source_name text, tenant bigint, id bigint, active boolean NOT NULL,
    PRIMARY KEY (source_name, tenant, id)
);
CREATE TABLE public.source_sync_seen (source_record_key bytea PRIMARY KEY, original jsonb NOT NULL);
GRANT SELECT, INSERT, UPDATE, DELETE, MAINTAIN ON public.source_sync_crm,
    public.source_sync_erp, public.source_sync_expected TO mdm_administrator;
INSERT INTO public.source_sync_crm VALUES
    (1, 1, false, 'One', 'one@sync.test', statement_timestamp()),
    (1, 2, NULL, 'Two', 'two@sync.test', statement_timestamp());
INSERT INTO public.source_sync_erp VALUES
    (2, 1, false, 'Three', 'one@sync.test', statement_timestamp());
INSERT INTO public.source_sync_expected VALUES ('crm', 1, 1, true), ('crm', 1, 2, true), ('erp', 2, 1, true);

CREATE FUNCTION public.source_sync_state() RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    entity uuid; result jsonb := '{}'::jsonb; rows jsonb; member record; table_name text;
BEGIN
    SELECT entity_id INTO STRICT entity FROM mdm_internal.entities WHERE entity_name = 'source_sync';
    FOR table_name IN SELECT unnest(ARRAY[
        'source_records', 'identity_registry', 'memberships', 'identity_aliases', 'identity_splits',
        'golden_provenance', 'reviews', 'publications', 'publication_observations', 'resolution_facts'
    ]) LOOP
        EXECUTE format('SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), ''[]''::jsonb) FROM mdm_internal.%I t WHERE entity_id = $1', table_name)
        INTO rows USING entity;
        result := result || jsonb_build_object(table_name, rows);
    END LOOP;
    FOR member IN
        SELECT m.logical_id, m.relation_name FROM mdm_internal.graph_members m
        JOIN mdm_internal.graph_bindings b USING (graph_binding_id) WHERE b.entity_id = entity
    LOOP
        EXECUTE format('SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), ''[]''::jsonb) FROM %s t', member.relation_name)
        INTO rows;
        result := result || jsonb_build_object(member.logical_id, rows);
    END LOOP;
    RETURN result || jsonb_build_object(
        'entity_catalog', (SELECT to_jsonb(e) FROM mdm_internal.entities e WHERE e.entity_id = entity),
        'producer_streams', (SELECT jsonb_agg(to_jsonb(st) ORDER BY st.pgt_id)
            FROM pgtrickle.pgt_stream_tables st JOIN mdm_internal.graph_members m ON m.relation_oid = st.pgt_relid
            JOIN mdm_internal.graph_bindings b USING (graph_binding_id) WHERE b.entity_id = entity),
        'entities', (SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.mdm_id), '[]'::jsonb) FROM mdm_out.source_sync t),
        'members', (SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.source_record_id), '[]'::jsonb) FROM mdm_out.source_sync_members t),
        'output_reviews', (SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.review_id), '[]'::jsonb) FROM mdm_out.source_sync_review t),
        'consumers', (SELECT jsonb_agg(to_jsonb(s) ORDER BY c.logical_id)
            FROM pgtrickle.output_delta_consumer_status() s
            JOIN mdm_internal.graph_delta_consumers c USING (consumer_id)
            JOIN mdm_internal.graph_bindings b USING (graph_binding_id) WHERE b.entity_id = entity)
    );
END
$$;

CREATE FUNCTION public.assert_source_sync() RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE entity uuid; expected jsonb; actual jsonb; live jsonb; members jsonb;
BEGIN
    LOCK TABLE public.source_sync_crm, public.source_sync_erp IN SHARE MODE;
    SELECT entity_id INTO STRICT entity FROM mdm_internal.entities WHERE entity_name = 'source_sync';
    SELECT jsonb_agg(jsonb_build_object('key', encode(pgtrickle.encode_row_id_v2(
        'SCAN_KEY', ROW(entity, s.source_identity_id, x.tenant, x.id)), 'hex'), 'active', x.active)
        ORDER BY s.source_name, x.tenant, x.id)
    INTO expected FROM public.source_sync_expected x JOIN mdm_internal.source_identities s
        ON s.entity_id = entity AND s.source_name = x.source_name;
    SELECT jsonb_agg(jsonb_build_object('key', encode(r.source_record_key, 'hex'), 'active', r.active)
        ORDER BY s.source_name, x.tenant, x.id)
    INTO actual FROM mdm_internal.source_records r JOIN mdm_internal.source_identities s USING (source_identity_id)
    LEFT JOIN public.source_sync_expected x ON x.source_name = s.source_name
        AND r.source_record_key = pgtrickle.encode_row_id_v2('SCAN_KEY', ROW(entity, s.source_identity_id, x.tenant, x.id))
    WHERE r.entity_id = entity;
    IF actual IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'source registry differs from fixed lifecycle oracle: expected %, actual %', expected, actual;
    END IF;
    SELECT jsonb_agg(key ORDER BY key) INTO expected FROM jsonb_array_elements(actual) x
        CROSS JOIN LATERAL (SELECT x->>'key' AS key) k WHERE (x->>'active')::boolean;
    SELECT jsonb_agg(key ORDER BY key) INTO live FROM (
        SELECT encode(pgtrickle.encode_row_id_v2('SCAN_KEY', ROW(entity, s.source_identity_id, c.tenant, c.id)), 'hex') AS key
        FROM public.source_sync_crm c CROSS JOIN mdm_internal.source_identities s
        WHERE s.entity_id = entity AND s.source_name = 'crm' AND c.deleted IS NOT TRUE
        UNION ALL
        SELECT encode(pgtrickle.encode_row_id_v2('SCAN_KEY', ROW(entity, s.source_identity_id, c.tenant, c.id)), 'hex')
        FROM public.source_sync_erp c CROSS JOIN mdm_internal.source_identities s
        WHERE s.entity_id = entity AND s.source_name = 'erp'
    ) independently_scanned_source_keys;
    IF live IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'active registry differs from complete same-fence source keys: expected %, actual %', live, expected;
    END IF;
    SELECT jsonb_agg(jsonb_build_object('key', m.source_id->>'source_record_key', 'active', m.active)
        ORDER BY m.source_name, x.tenant, x.id)
    INTO members FROM mdm_out.source_sync_members m
    JOIN mdm_internal.source_identities s ON s.entity_id = entity AND s.source_name = m.source_name
    LEFT JOIN public.source_sync_expected x ON x.source_name = m.source_name
        AND m.source_id->>'source_record_key' = encode(pgtrickle.encode_row_id_v2('SCAN_KEY', ROW(entity, s.source_identity_id, x.tenant, x.id)), 'hex');
    IF members IS DISTINCT FROM actual THEN
        RAISE EXCEPTION 'member active states differ from registry: expected %, actual %', actual, members;
    END IF;
    SELECT jsonb_agg(component ORDER BY component::text) INTO expected FROM (
        SELECT jsonb_agg(key ORDER BY key) AS component FROM (
            SELECT encode(pgtrickle.encode_row_id_v2('SCAN_KEY', ROW(entity, s.source_identity_id, c.tenant, c.id)), 'hex') AS key, c.email_address
            FROM public.source_sync_crm c CROSS JOIN mdm_internal.source_identities s
            WHERE s.entity_id = entity AND s.source_name = 'crm' AND c.deleted IS NOT TRUE
            UNION ALL
            SELECT encode(pgtrickle.encode_row_id_v2('SCAN_KEY', ROW(entity, s.source_identity_id, c.tenant, c.id)), 'hex'), c.email_address
            FROM public.source_sync_erp c CROSS JOIN mdm_internal.source_identities s
            WHERE s.entity_id = entity AND s.source_name = 'erp'
        ) source_keys GROUP BY email_address
    ) independently_expected_exact_email_components;
    SELECT jsonb_agg(component ORDER BY component::text) INTO actual FROM (
        SELECT jsonb_agg(source_id->>'source_record_key' ORDER BY source_id->>'source_record_key') AS component
        FROM mdm_out.source_sync_members WHERE active GROUP BY mdm_id
    ) published_components;
    IF actual IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'source lifecycle changed complete expected matching: expected %, actual %', expected, actual;
    END IF;
    IF EXISTS (SELECT FROM mdm_internal.source_records r JOIN public.source_sync_seen seen USING (source_record_key)
        WHERE r.entity_id = entity AND to_jsonb(r) - 'active' - 'last_seen_at' IS DISTINCT FROM seen.original - 'active' - 'last_seen_at') THEN
        RAISE EXCEPTION 'source lifecycle changed a durable source identity';
    END IF;
    INSERT INTO public.source_sync_seen SELECT source_record_key, to_jsonb(r)
    FROM mdm_internal.source_records r WHERE r.entity_id = entity ON CONFLICT DO NOTHING;
END
$$;
REVOKE ALL ON FUNCTION public.source_sync_state(), public.assert_source_sync() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.source_sync_state(), public.assert_source_sync() TO mdm_administrator;

ALTER ROLE mdm_test_login SET pg_trickle.refresh_strategy = 'differential';
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SET mdm.trace_graph_deltas = 'on';
SELECT * FROM mdm.create(mdm.entity(
    name => 'source_sync', sources => ARRAY[
        mdm.source(name => 'crm', relation => 'public.source_sync_crm'::regclass,
            source_id => ARRAY['tenant','id'], mode => 'soft_delete',
            soft_delete_when => '{"column":"deleted","kind":"is_true"}'::jsonb,
            fields => '{"name":"display_name","email":"email_address"}'::jsonb, row_changed_at => 'updated_at'),
        mdm.source(name => 'erp', relation => 'public.source_sync_erp'::regclass,
            source_id => ARRAY['tenant','id'], mode => 'tracked',
            fields => '{"name":"display_name","email":"email_address"}'::jsonb, row_changed_at => 'updated_at')
    ], fields => ARRAY[
        mdm.field(name => 'name', type => 'text', cleaner => 'company_name'),
        mdm.field(name => 'email', type => 'text', cleaner => 'email')
    ], matches => ARRAY[
        mdm.match(name => 'same_email', fields => ARRAY['email'], comparison => 'exact',
            strength => 'identity', evidence_group => 'email', candidate => '{"kind":"exact","field":"email"}'::jsonb)
    ], golden_values => ARRAY[mdm.golden_value(field => 'name', policy => 'prefer_source', sources => ARRAY['crm','erp'])],
    limits => '{"max_active_records":4}'::jsonb
));
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();
DO $$
DECLARE before_state jsonb := public.source_sync_state(); result jsonb; after_state jsonb;
BEGIN
    result := mdm.refresh('source_sync');
    after_state := public.source_sync_state();
    IF result->>'changed' IS DISTINCT FROM 'false'
       OR (after_state->'source_records', after_state->'entities', after_state->'members', after_state->'output_reviews')
          IS DISTINCT FROM (before_state->'source_records', before_state->'entities', before_state->'members', before_state->'output_reviews') THEN
        RAISE EXCEPTION 'no-change refresh changed complete source or output rows';
    END IF;
END
$$;
SELECT public.assert_source_sync();

UPDATE public.source_sync_crm SET tenant = 9, id = 9, display_name = 'Renamed' WHERE tenant = 1 AND id = 1;
UPDATE public.source_sync_expected SET active = false WHERE source_name = 'crm' AND tenant = 1 AND id = 1;
INSERT INTO public.source_sync_expected VALUES ('crm', 9, 9, true);
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();

UPDATE public.source_sync_crm SET deleted = true WHERE tenant = 1 AND id = 2;
DELETE FROM public.source_sync_erp WHERE tenant = 2 AND id = 1;
INSERT INTO public.source_sync_crm VALUES (1, 3, false, 'Four', 'four@sync.test', statement_timestamp());
UPDATE public.source_sync_expected SET active = false WHERE (source_name, tenant, id) IN (('crm',1,2),('erp',2,1));
INSERT INTO public.source_sync_expected VALUES ('crm', 1, 3, true);
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();

UPDATE public.source_sync_crm SET deleted = false WHERE tenant = 1 AND id = 2;
INSERT INTO public.source_sync_erp VALUES (2, 1, false, 'Three', 'one@sync.test', statement_timestamp());
UPDATE public.source_sync_expected SET active = true WHERE (source_name, tenant, id) IN (('crm',1,2),('erp',2,1));
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();

-- Each source fits the limit; their combined active population does not.
INSERT INTO public.source_sync_erp VALUES (2, 2, false, 'Five', 'five@sync.test', statement_timestamp());
DO $$
DECLARE before_state jsonb := public.source_sync_state();
BEGIN
    BEGIN
        PERFORM mdm.refresh('source_sync');
        RAISE EXCEPTION 'entity-wide source limit accepted five active keys';
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM <> 'MDM_RESOLVER_LIMIT: resolver limit exceeded for max_active_records: 5 > 4' THEN RAISE; END IF;
    END;
    IF public.source_sync_state() IS DISTINCT FROM before_state THEN
        RAISE EXCEPTION 'source-limit failure changed capture, registration, graph, history, acknowledgement or output';
    END IF;
END
$$;
DELETE FROM public.source_sync_erp WHERE tenant = 2 AND id = 2;
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();

\connect foundation postgres
CREATE FUNCTION public.source_sync_fail() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'source-sync injected publication failure'; END
$$;
CREATE TRIGGER source_sync_fail BEFORE INSERT OR UPDATE ON mdm_out.source_sync
FOR EACH ROW EXECUTE FUNCTION public.source_sync_fail();
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
UPDATE public.source_sync_crm SET display_name = 'Changed Four' WHERE tenant = 1 AND id = 3;
DO $$
DECLARE before_state jsonb := public.source_sync_state();
BEGIN
    BEGIN
        PERFORM mdm.refresh('source_sync');
        RAISE EXCEPTION 'publication failure was not injected';
    EXCEPTION WHEN OTHERS THEN
        IF strpos(SQLERRM, 'source-sync injected publication failure') = 0 THEN RAISE; END IF;
    END;
    IF public.source_sync_state() IS DISTINCT FROM before_state THEN
        RAISE EXCEPTION 'publication failure changed capture, registration, graph, history, acknowledgement or output';
    END IF;
END
$$;
\connect foundation postgres
DROP TRIGGER source_sync_fail ON mdm_out.source_sync;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();
DO $$
DECLARE before_state jsonb := public.source_sync_state(); after_state jsonb;
BEGIN
    PERFORM mdm_admin.rebuild('source_sync');
    after_state := public.source_sync_state();
    IF (after_state->'entities', after_state->'members', after_state->'output_reviews', after_state->'source_records')
        IS DISTINCT FROM (before_state->'entities', before_state->'members', before_state->'output_reviews', before_state->'source_records') THEN
        RAISE EXCEPTION 'complete-scan rebuild differs from changed-key publication';
    END IF;
END
$$;

\connect foundation postgres
REVOKE SELECT ON public.source_sync_erp FROM mdm_administrator;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
DO $$
DECLARE before_state jsonb := public.source_sync_state();
BEGIN
    BEGIN
        PERFORM mdm.refresh('source_sync');
        RAISE EXCEPTION 'revoked source access was accepted on a no-change refresh';
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM <> 'MDM_SOURCE_INVALID: invalid source contract: source erp binding or execution-role privileges changed' THEN RAISE; END IF;
    END;
    IF public.source_sync_state() IS DISTINCT FROM before_state THEN
        RAISE EXCEPTION 'denied refresh changed capture or publication state';
    END IF;
END
$$;
\connect foundation postgres
GRANT SELECT ON public.source_sync_erp TO mdm_administrator;

-- Recovery of pg_mdm's source consumer must retain another consumer's pending work.
CREATE TABLE public.source_sync_consumers AS
SELECT c.logical_id, c.consumer_id, m.relation_name, m.contract_digest
FROM mdm_internal.graph_delta_consumers c JOIN mdm_internal.graph_members m USING (graph_binding_id, logical_id)
JOIN mdm_internal.graph_bindings b USING (graph_binding_id) JOIN mdm_internal.entities e USING (entity_id)
WHERE e.entity_name = 'source_sync' AND c.logical_id = 'records/crm';
GRANT SELECT ON public.source_sync_consumers TO mdm_administrator;
CREATE TABLE public.source_sync_sibling (consumer_id uuid);
GRANT SELECT, INSERT ON public.source_sync_sibling TO mdm_administrator;
BEGIN;
INSERT INTO public.source_sync_sibling SELECT registered.consumer_id
FROM public.source_sync_consumers c CROSS JOIN LATERAL pgtrickle.register_output_delta_consumer(
    c.relation_name::regclass, 'source-sync-sibling', c.contract_digest, 'RESNAPSHOT_REQUIRED') registered;
SELECT pgtrickle.ack_output_delta_resnapshot(c.consumer_id, snapshot.resnapshot_token)
FROM public.source_sync_sibling c CROSS JOIN LATERAL pgtrickle.begin_output_delta_resnapshot(c.consumer_id) snapshot;
COMMIT;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
UPDATE public.source_sync_crm SET tenant = 8, id = 8 WHERE tenant = 9 AND id = 9;
UPDATE public.source_sync_expected SET active = false WHERE source_name = 'crm' AND tenant = 9 AND id = 9;
INSERT INTO public.source_sync_expected VALUES ('crm', 8, 8, true);
\connect foundation postgres
SELECT pgtrickle.request_output_delta_resnapshot(consumer_id) FROM public.source_sync_consumers;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();
\connect foundation postgres
DO $$
DECLARE observed jsonb;
BEGIN
    SELECT jsonb_agg(jsonb_build_object('mode', b.mode, 'rows', b.row_count, 'inserted', b.rows_inserted, 'deleted', b.rows_deleted)
        ORDER BY b.batch_token) INTO observed
    FROM public.source_sync_sibling c CROSS JOIN LATERAL pgtrickle.output_delta_batches(c.consumer_id, NULL) b;
    IF observed IS DISTINCT FROM '[{"mode":"EXACT","rows":2,"inserted":1,"deleted":1},{"mode":"EXACT","rows":0,"inserted":0,"deleted":0}]'::jsonb THEN
        RAISE EXCEPTION 'source resnapshot lost a sibling consumer pending key range: %', observed;
    END IF;
END
$$;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();
RESET ROLE;
\connect foundation postgres
SELECT pgtrickle.qualify_output_delta_recovery(consumer_id, 'DELTA_GAP') FROM public.source_sync_consumers;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();
\connect foundation postgres
SELECT pgtrickle.qualify_output_delta_recovery(consumer_id, 'FULL_INVALIDATION') FROM public.source_sync_consumers;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SELECT mdm.refresh('source_sync');
SELECT public.assert_source_sync();
\connect foundation postgres
CREATE TABLE public.source_sync_partitioned (LIKE public.source_sync_crm INCLUDING ALL) PARTITION BY RANGE (tenant);
CREATE TABLE public.source_sync_partition PARTITION OF public.source_sync_partitioned DEFAULT;
INSERT INTO public.source_sync_partitioned VALUES (7, 7, false, 'Partitioned', 'partitioned@sync.test', statement_timestamp());
GRANT SELECT, INSERT, UPDATE, DELETE, MAINTAIN ON public.source_sync_partitioned TO mdm_administrator;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SELECT * FROM mdm.create(jsonb_set(jsonb_set(mdm.describe('source_sync', 'definition'),
    '{name}', '"source_sync_unsupported"'::jsonb), '{sources,0,relation}', '"public.source_sync_partitioned"'::jsonb));
SELECT mdm.refresh('source_sync_unsupported');
SELECT mdm.refresh('source_sync_unsupported');
UPDATE public.source_sync_partitioned SET id = 8 WHERE tenant = 7 AND id = 7;
SELECT mdm.refresh('source_sync_unsupported');
\connect foundation postgres
DO $$
DECLARE expected jsonb; actual jsonb; members jsonb;
BEGIN
    SELECT jsonb_agg(jsonb_build_object('key', encode(pgtrickle.encode_row_id_v2(
        'SCAN_KEY', ROW(e.entity_id, s.source_identity_id, x.tenant, x.id)), 'hex'), 'active', x.active)
        ORDER BY s.source_name, x.tenant, x.id) INTO expected
    FROM (VALUES ('crm',7::bigint,7::bigint,false),('crm',7,8,true),('erp',2,1,true)) x(source_name,tenant,id,active)
    JOIN mdm_internal.source_identities s ON s.source_name = x.source_name
    JOIN mdm_internal.entities e USING (entity_id) WHERE e.entity_name = 'source_sync_unsupported';
    SELECT jsonb_agg(jsonb_build_object('key', encode(r.source_record_key, 'hex'), 'active', r.active)
        ORDER BY s.source_name, CASE WHEN r.active THEN 8 ELSE 7 END)
    INTO actual FROM mdm_internal.source_records r JOIN mdm_internal.source_identities s USING (source_identity_id)
    JOIN mdm_internal.entities e ON e.entity_id = r.entity_id WHERE e.entity_name = 'source_sync_unsupported';
    SELECT jsonb_agg(jsonb_build_object('key', source_id->>'source_record_key', 'active', active)
        ORDER BY source_name, CASE WHEN active THEN 8 ELSE 7 END)
    INTO members FROM mdm_out.source_sync_unsupported_members;
    IF actual IS DISTINCT FROM expected OR members IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'unsupported-shape complete scan lost old/new keys: expected %, registry %, members %', expected, actual, members;
    END IF;
END
$$;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SELECT mdm_admin.drop_entity('source_sync_unsupported', 'source_sync_unsupported');
SELECT mdm_admin.drop_entity('source_sync', 'source_sync');
\connect foundation postgres
DROP FUNCTION public.source_sync_state(), public.assert_source_sync(), public.source_sync_fail();
DROP TABLE public.source_sync_crm, public.source_sync_erp, public.source_sync_expected,
    public.source_sync_seen, public.source_sync_consumers, public.source_sync_sibling,
    public.source_sync_partitioned;
ALTER ROLE mdm_test_login RESET pg_trickle.refresh_strategy;
SELECT 'PASS: fenced source-key lifecycle, complete-scan oracle, entity-wide limits, rollback/retry, denial and independent consumer recovery';
