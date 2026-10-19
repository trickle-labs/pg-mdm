\set ON_ERROR_STOP on
\connect foundation postgres

CREATE TABLE public.scoped_state_source (
    id bigint PRIMARY KEY, display_name text, email_address text, updated_at timestamptz NOT NULL
);
INSERT INTO public.scoped_state_source
SELECT id, CASE WHEN id = 5 THEN 'Åcme "雪" \ Corp' ELSE 'Record ' || id END,
    CASE WHEN id IN (3,4) THEN 'shared@scope.test' ELSE id || '@scope.test' END, statement_timestamp()
FROM generate_series(1, 5) id;
GRANT SELECT, INSERT, UPDATE, DELETE, MAINTAIN ON public.scoped_state_source TO mdm_administrator;
GRANT EXECUTE ON FUNCTION mdm_steward.override_golden(text, uuid, text, jsonb, bigint, text),
    mdm_steward.clear_golden_override(text, uuid, text, bigint, text) TO mdm_administrator;
CREATE TABLE public.scoped_state_cases (label text PRIMARY KEY, state jsonb NOT NULL, historical jsonb NOT NULL);
GRANT SELECT, INSERT ON public.scoped_state_cases TO mdm_administrator;

CREATE FUNCTION public.scoped_state_snapshot() RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE entity uuid; revision bigint; result jsonb; rows jsonb; table_name text;
BEGIN
    SELECT entity_id, publication_revision INTO STRICT entity, revision
    FROM mdm_internal.entities WHERE entity_name = 'scoped_state';
    result := jsonb_build_object('publication_revision', revision);
    FOREACH table_name IN ARRAY ARRAY['identity_registry', 'memberships', 'identity_aliases',
        'identity_splits', 'reviews', 'golden_provenance', 'resolution_facts', 'publications'] LOOP
        EXECUTE format('SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), ''[]''::jsonb)
            FROM mdm_internal.%I t WHERE entity_id = $1', table_name) INTO rows USING entity;
        result := result || jsonb_build_object(table_name, rows);
    END LOOP;
    RETURN result || jsonb_build_object(
        'source_ids', (SELECT jsonb_object_agg(encode(pgtrickle.encode_row_id_v2('SCAN_KEY',
            ROW(entity, i.source_identity_id, s.id)), 'hex'), s.id)
            FROM public.scoped_state_source s CROSS JOIN mdm_internal.source_identities i WHERE i.entity_id = entity),
        'entities', (SELECT jsonb_agg(to_jsonb(t) ORDER BY mdm_id) FROM mdm_out.scoped_state t),
        'members', (SELECT jsonb_agg(to_jsonb(t) ORDER BY source_record_id) FROM mdm_out.scoped_state_members t),
        'output_reviews', COALESCE((SELECT jsonb_agg(to_jsonb(t) ORDER BY review_id) FROM mdm_out.scoped_state_review t), '[]'::jsonb));
END
$$;
REVOKE ALL ON FUNCTION public.scoped_state_snapshot() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.scoped_state_snapshot() TO mdm_administrator;

CREATE FUNCTION public.check_scoped_state(label text, expected_records integer, expected_components jsonb)
RETURNS void LANGUAGE plpgsql SET search_path = pg_catalog AS $$
DECLARE result jsonb; before_state jsonb; rebuilt jsonb; current_explanations jsonb := '[]';
    historical jsonb := '[]'; expected_history jsonb; row jsonb; subject jsonb; actual_components jsonb;
BEGIN
    result := mdm.refresh('scoped_state');
    IF result->>'resolver_strategy' IS DISTINCT FROM (CASE WHEN label = 'bootstrap' THEN 'full' ELSE 'affected' END)
       OR result->>'affected_records' IS DISTINCT FROM expected_records::text
       OR result->>'delta_lag' IS DISTINCT FROM '0' THEN
        RAISE EXCEPTION 'scoped-state % did not use the expected complete closure: %', label, result;
    END IF;
    before_state := public.scoped_state_snapshot();
    IF EXISTS (SELECT FROM jsonb_array_elements(before_state->'entities') e
        WHERE (e->>'has_review')::boolean IS DISTINCT FROM EXISTS (
            SELECT FROM jsonb_array_elements(before_state->'reviews') r WHERE r->>'status' = 'open')) THEN
        RAISE EXCEPTION 'scoped-state % public review flags differ from the complete review set: %', label, before_state->'entities';
    END IF;
    SELECT jsonb_agg(component ORDER BY component::text) INTO actual_components FROM (
        SELECT jsonb_agg(s.value::bigint ORDER BY s.value::bigint) AS component
        FROM jsonb_array_elements(before_state->'members') m
        JOIN jsonb_each_text(before_state->'source_ids') s ON m->'source_id'->>'source_record_key' = s.key
        WHERE (m->>'active')::boolean GROUP BY m->>'mdm_id'
    ) components;
    IF actual_components IS DISTINCT FROM expected_components THEN
        RAISE EXCEPTION 'scoped-state % components differ: expected %, actual %', label, expected_components, actual_components;
    END IF;
    FOR row IN SELECT value FROM jsonb_array_elements(before_state->'memberships') ORDER BY value->>'source_record_id' LOOP
        subject := jsonb_build_object('kind', 'source_record', 'id', row->>'source_record_id');
        current_explanations := current_explanations || jsonb_build_array(mdm.explain('scoped_state', subject, NULL, 100));
        historical := historical || jsonb_build_array(mdm.explain('scoped_state', subject, 1, 100) - 'retained_revision_range');
    END LOOP;
    FOR row IN SELECT value FROM jsonb_array_elements(before_state->'identity_registry') ORDER BY value->>'mdm_id' LOOP
        current_explanations := current_explanations || jsonb_build_array(mdm.explain('scoped_state',
            jsonb_build_object('kind', 'mdm_id', 'id', row->>'mdm_id'), NULL, 100));
    END LOOP;
    SELECT jsonb_build_object('kind', 'pair', 'left_id', min(m->>'source_record_id') FILTER (WHERE s.value = '3'),
        'right_id', min(m->>'source_record_id') FILTER (WHERE s.value = '4')) INTO subject
    FROM jsonb_array_elements(before_state->'memberships') m
    JOIN jsonb_each_text(before_state->'source_ids') s ON s.key = m->'source_id'->>'source_record_key';
    historical := historical || jsonb_build_array(mdm.explain('scoped_state', subject, 1, 100) - 'retained_revision_range');
    SELECT c.historical INTO expected_history FROM public.scoped_state_cases c WHERE c.label = 'bootstrap';
    IF expected_history IS NOT NULL AND historical IS DISTINCT FROM expected_history THEN
        RAISE EXCEPTION 'scoped-state % changed retained historical explanations', label;
    END IF;
    rebuilt := mdm_admin.rebuild('scoped_state');
    IF rebuilt->>'changed' IS DISTINCT FROM 'false' OR public.scoped_state_snapshot() IS DISTINCT FROM before_state THEN
        RAISE EXCEPTION 'scoped-state % full evaluation changed complete durable/output/history state: %', label, rebuilt;
    END IF;
    subject := '[]';
    FOR row IN SELECT value FROM jsonb_array_elements(before_state->'memberships') ORDER BY value->>'source_record_id' LOOP
        subject := subject || jsonb_build_array(mdm.explain('scoped_state',
            jsonb_build_object('kind', 'source_record', 'id', row->>'source_record_id'), NULL, 100));
    END LOOP;
    FOR row IN SELECT value FROM jsonb_array_elements(before_state->'identity_registry') ORDER BY value->>'mdm_id' LOOP
        subject := subject || jsonb_build_array(mdm.explain('scoped_state',
            jsonb_build_object('kind', 'mdm_id', 'id', row->>'mdm_id'), NULL, 100));
    END LOOP;
    IF subject IS DISTINCT FROM current_explanations THEN
        RAISE EXCEPTION 'scoped-state % full evaluation changed current explanations', label;
    END IF;
    INSERT INTO public.scoped_state_cases VALUES (label, before_state, historical);
END
$$;
REVOKE ALL ON FUNCTION public.check_scoped_state(text, integer, jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.check_scoped_state(text, integer, jsonb) TO mdm_administrator;
ALTER ROLE mdm_test_login SET pg_trickle.refresh_strategy = 'differential';
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET mdm.trace_graph_deltas = 'on';
SELECT * FROM mdm.create(mdm.entity(
    name => 'scoped_state', sources => ARRAY[mdm.source(name => 'crm',
        relation => 'public.scoped_state_source'::regclass, source_id => ARRAY['id'], mode => 'tracked',
        fields => '{"name":"display_name","email":"email_address"}'::jsonb, row_changed_at => 'updated_at')],
    fields => ARRAY[mdm.field(name => 'name', type => 'text', cleaner => 'company_name'),
        mdm.field(name => 'email', type => 'text', cleaner => 'email')],
    matches => ARRAY[mdm.match(name => 'email', fields => ARRAY['email'], comparison => 'exact', strength => 'identity',
        evidence_group => 'email', candidate => '{"kind":"exact","field":"email"}'::jsonb)],
    golden_values => ARRAY[mdm.golden_value(field => 'name', policy => 'first_non_null')]
));
SELECT public.check_scoped_state('bootstrap', 5, '[[1],[2],[3,4],[5]]');
SELECT m->>'source_record_id' AS first_anchor FROM jsonb_array_elements(public.scoped_state_snapshot()->'memberships') m
WHERE m->'source_id'->>'source_record_key' = (SELECT key FROM jsonb_each_text(public.scoped_state_snapshot()->'source_ids') WHERE value = '1') \gset
SELECT m->>'source_record_id' AS second_anchor FROM jsonb_array_elements(public.scoped_state_snapshot()->'memberships') m
WHERE m->'source_id'->>'source_record_key' = (SELECT key FROM jsonb_each_text(public.scoped_state_snapshot()->'source_ids') WHERE value = '2') \gset
SELECT * FROM mdm_steward.override_golden('scoped_state', :'first_anchor', 'name', '"First"', 0, 'fixture');
SELECT public.check_scoped_state('control_only', 5, '[[1],[2],[3,4],[5]]');
SELECT * FROM mdm_steward.override_golden('scoped_state', :'second_anchor', 'name', '"Second"', 0, 'fixture');
UPDATE public.scoped_state_source SET email_address = '1@scope.test' WHERE id = 2;
SELECT public.check_scoped_state('merge_conflict', 5, '[[1,2],[3,4],[5]]');
UPDATE public.scoped_state_source SET email_address = '2@scope.test' WHERE id = 2;
SELECT public.check_scoped_state('split_resolves', 2, '[[1],[2],[3,4],[5]]');
UPDATE public.scoped_state_source SET email_address = '1@scope.test' WHERE id = 2;
SELECT public.check_scoped_state('conflict_recurs', 2, '[[1,2],[3,4],[5]]');
DELETE FROM public.scoped_state_source WHERE id = 1;
SELECT public.check_scoped_state('inactive_anchor', 2, '[[2],[3,4],[5]]');
UPDATE public.scoped_state_source SET display_name = 'Changed remote record' WHERE id = 5;
SELECT public.check_scoped_state('remote_override_dependency', 1, '[[2],[3,4],[5]]');
SELECT * FROM mdm_steward.clear_golden_override('scoped_state', :'first_anchor', 'name', 1, 'fixture');
SELECT public.check_scoped_state('inactive_control_only', 5, '[[2],[3,4],[5]]');
INSERT INTO public.scoped_state_source VALUES (1, 'Record 1', '1@scope.test', statement_timestamp());
SELECT public.check_scoped_state('reactivation', 2, '[[1,2],[3,4],[5]]');
DELETE FROM public.scoped_state_source WHERE id = 5;
SELECT public.check_scoped_state('retirement', 1, '[[1,2],[3,4]]');
INSERT INTO public.scoped_state_source VALUES (5, 'Record 5', '5@scope.test', statement_timestamp());
SELECT public.check_scoped_state('retired_reactivation', 1, '[[1,2],[3,4],[5]]');
\connect foundation postgres
DO $$
DECLARE actual jsonb; initial jsonb; merged jsonb; split jsonb; first_id uuid; second_id uuid; parent uuid;
BEGIN
    SELECT jsonb_agg(jsonb_build_array(r.occurrence, r.status) ORDER BY r.occurrence) INTO actual
    FROM mdm_internal.reviews r JOIN mdm_internal.entities e USING (entity_id)
    WHERE e.entity_name = 'scoped_state' AND reason_code = 'GOLDEN_OVERRIDE_CONFLICT';
    IF actual IS DISTINCT FROM '[[1,"resolved"],[2,"resolved"]]'::jsonb THEN
        RAISE EXCEPTION 'scoped-state lost review recurrence: %', actual;
    END IF;
    SELECT state INTO STRICT initial FROM public.scoped_state_cases WHERE label = 'bootstrap';
    SELECT state INTO STRICT merged FROM public.scoped_state_cases WHERE label = 'merge_conflict';
    SELECT state INTO STRICT split FROM public.scoped_state_cases WHERE label = 'split_resolves';
    SELECT (m->>'mdm_id')::uuid INTO first_id FROM jsonb_array_elements(initial->'memberships') m
    WHERE initial->'source_ids'->>(m->'source_id'->>'source_record_key') = '1';
    SELECT (m->>'mdm_id')::uuid INTO second_id FROM jsonb_array_elements(initial->'memberships') m
    WHERE initial->'source_ids'->>(m->'source_id'->>'source_record_key') = '2';
    parent := least(first_id, second_id);
    SELECT jsonb_agg(jsonb_build_array(a->>'alias_mdm_id', a->>'canonical_mdm_id')) INTO actual
    FROM jsonb_array_elements(merged->'identity_aliases') a
    WHERE (a->>'alias_mdm_id')::uuid IN (first_id, second_id);
    IF actual IS DISTINCT FROM jsonb_build_array(jsonb_build_array(greatest(first_id, second_id), parent)) THEN
        RAISE EXCEPTION 'merge did not retain the exact expected alias: %', actual;
    END IF;
    SELECT jsonb_agg(jsonb_build_array(s->>'parent_mdm_id', s->>'child_mdm_id') ORDER BY s->>'child_mdm_id')
    INTO actual FROM jsonb_array_elements(split->'identity_splits') s;
    IF actual IS DISTINCT FROM (
        SELECT jsonb_agg(jsonb_build_array(parent, child) ORDER BY child) FROM (
            SELECT DISTINCT (m->>'mdm_id')::uuid child FROM jsonb_array_elements(split->'memberships') m
            WHERE split->'source_ids'->>(m->'source_id'->>'source_record_key') IN ('1','2')
        ) children
    ) THEN
        RAISE EXCEPTION 'split did not retain every exact successor: %', actual;
    END IF;
END
$$;
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DELETE FROM public.scoped_state_source WHERE id = 1;
SELECT mdm.refresh('scoped_state')->>'resolver_strategy' AS preparation_strategy;
BEGIN;
SAVEPOINT original_boundary;
INSERT INTO public.scoped_state_source VALUES (1, 'Record 1', '1@scope.test', '2026-10-18 00:00:00+00');
SELECT mdm.refresh('scoped_state')->>'resolver_strategy' AS strategy \gset affected_
SELECT jsonb_build_object('entities', (SELECT jsonb_agg(e - 'last_change_revision' ORDER BY e->>'mdm_id')
    FROM jsonb_array_elements(s->'entities') e), 'members', s->'members', 'reviews', s->'output_reviews') AS publication
FROM (SELECT public.scoped_state_snapshot() s) state \gset affected_
SELECT jsonb_agg(jsonb_build_array(e->'name', e->'member_count', e->'has_review') ORDER BY e->>'name') AS rows
FROM jsonb_array_elements(public.scoped_state_snapshot()->'entities') e \gset affected_
ROLLBACK TO SAVEPOINT original_boundary;
INSERT INTO public.scoped_state_source VALUES (1, 'Record 1', '1@scope.test', '2026-10-18 00:00:00+00');
SELECT mdm_admin.rebuild('scoped_state')->>'resolver_strategy' AS strategy \gset full_
SELECT jsonb_build_object('entities', (SELECT jsonb_agg(e - 'last_change_revision' ORDER BY e->>'mdm_id')
    FROM jsonb_array_elements(s->'entities') e), 'members', s->'members', 'reviews', s->'output_reviews') AS publication
FROM (SELECT public.scoped_state_snapshot() s) state \gset full_
SELECT :'affected_strategy' = 'affected' AND :'full_strategy' = 'full'
    AND :'affected_publication'::jsonb = :'full_publication'::jsonb
    AND :'affected_rows'::jsonb = '[["Record 3",2,true],["Record 5",1,true],["Second",2,true]]'::jsonb AS equivalent \gset
ROLLBACK;
\if :equivalent
\echo 'PASS: same-boundary affected and full entity/member/review publications match'
\else
\echo :affected_publication
\echo :full_publication
DO $$ BEGIN RAISE EXCEPTION 'same-boundary affected and full public publications differ'; END $$;
\endif
SELECT mdm_admin.drop_entity('scoped_state', 'scoped_state');
\connect foundation postgres
DROP FUNCTION public.check_scoped_state(text, integer, jsonb), public.scoped_state_snapshot();
DROP TABLE public.scoped_state_source;
REVOKE EXECUTE ON FUNCTION mdm_steward.override_golden(text, uuid, text, jsonb, bigint, text),
    mdm_steward.clear_golden_override(text, uuid, text, bigint, text) FROM mdm_administrator;
ALTER ROLE mdm_test_login RESET pg_trickle.refresh_strategy;
SELECT 'PASS: scoped prior-state history, aliases, splits, controls, remote/inactive overrides and review recurrence';

CREATE TABLE public.review_flag_source (
    id bigint PRIMARY KEY, display_name text, email_address text, updated_at timestamptz NOT NULL
);
INSERT INTO public.review_flag_source VALUES
    (1, 'Alpha', 'shared@flags.test', statement_timestamp()),
    (2, 'Alpha', 'shared@flags.test', statement_timestamp()),
    (3, 'Remote', 'remote@flags.test', statement_timestamp());
GRANT SELECT, UPDATE, MAINTAIN ON public.review_flag_source TO mdm_administrator;
CREATE FUNCTION public.assert_review_flag_output(result jsonb, strategy text, affected integer,
    expected_rows jsonb, expected_reviews jsonb) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE actual_rows jsonb; actual_reviews jsonb;
BEGIN
    SELECT jsonb_agg(jsonb_build_array(name, member_count, has_review, last_change_revision) ORDER BY name)
    INTO actual_rows FROM mdm_out.review_flags;
    SELECT COALESCE(jsonb_agg(jsonb_build_array(reason_code, status, occurrence, opened_revision,
        resolved_revision, last_change_revision, concurrency_version) ORDER BY occurrence), '[]'::jsonb)
    INTO actual_reviews FROM mdm_out.review_flags_review;
    IF result->>'resolver_strategy' IS DISTINCT FROM strategy
       OR result->>'affected_records' IS DISTINCT FROM affected::text
       OR result->>'delta_lag' IS DISTINCT FROM '0'
       OR actual_rows IS DISTINCT FROM expected_rows OR actual_reviews IS DISTINCT FROM expected_reviews THEN
        RAISE EXCEPTION 'public review-flag transition differs: result %, rows % expected %, reviews % expected %',
            result, actual_rows, expected_rows, actual_reviews, expected_reviews;
    END IF;
END
$$;
REVOKE ALL ON FUNCTION public.assert_review_flag_output(jsonb, text, integer, jsonb, jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.assert_review_flag_output(jsonb, text, integer, jsonb, jsonb) TO mdm_administrator;
ALTER ROLE mdm_test_login SET pg_trickle.refresh_strategy = 'differential';
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SELECT * FROM mdm.create(mdm.entity(
    name => 'review_flags', sources => ARRAY[mdm.source(name => 'crm',
        relation => 'public.review_flag_source'::regclass, source_id => ARRAY['id'], mode => 'tracked',
        fields => '{"name":"display_name","email":"email_address"}'::jsonb, row_changed_at => 'updated_at')],
    fields => ARRAY[mdm.field(name => 'name', type => 'text', cleaner => 'company_name'),
        mdm.field(name => 'email', type => 'text', cleaner => 'email')],
    matches => ARRAY[mdm.match(name => 'email', fields => ARRAY['email'], comparison => 'exact', strength => 'identity',
        evidence_group => 'email', candidate => '{"kind":"exact","field":"email"}'::jsonb)],
    golden_values => ARRAY[mdm.golden_value(field => 'name', policy => 'most_common')]
));
SELECT public.assert_review_flag_output(mdm.refresh('review_flags'), 'full', 3,
    '[["Alpha",2,false,1],["Remote",1,false,1]]', '[]');
UPDATE public.review_flag_source SET display_name = 'Beta' WHERE id = 2;
SELECT public.assert_review_flag_output(mdm.refresh('review_flags'), 'affected', 2,
    '[["Alpha",2,true,2],["Remote",1,true,2]]', '[["GOLDEN_TIE","open",1,2,null,2,1]]');
UPDATE public.review_flag_source SET display_name = 'Remote changed' WHERE id = 3;
SELECT public.assert_review_flag_output(mdm.refresh('review_flags'), 'affected', 1,
    '[["Alpha",2,true,2],["Remote changed",1,true,3]]', '[["GOLDEN_TIE","open",1,2,null,2,1]]');
UPDATE public.review_flag_source SET display_name = 'Alpha' WHERE id = 2;
SELECT public.assert_review_flag_output(mdm.refresh('review_flags'), 'affected', 2,
    '[["Alpha",2,false,4],["Remote changed",1,false,4]]', '[["GOLDEN_TIE","resolved",1,2,4,4,2]]');
SELECT public.assert_review_flag_output(mdm.refresh('review_flags'), 'skipped', 0,
    '[["Alpha",2,false,4],["Remote changed",1,false,4]]', '[["GOLDEN_TIE","resolved",1,2,4,4,2]]');
SELECT mdm_admin.drop_entity('review_flags', 'review_flags');
\connect foundation postgres
DROP FUNCTION public.assert_review_flag_output(jsonb, text, integer, jsonb, jsonb);
DROP TABLE public.review_flag_source;
ALTER ROLE mdm_test_login RESET pg_trickle.refresh_strategy;
SELECT 'PASS: complete review flags, unrelated components, first/last reviews and exact change revisions';
