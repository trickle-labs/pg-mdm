\set ON_ERROR_STOP on
\connect foundation mdm_test_login
SET ROLE mdm_administrator;
SET jit = off;
DELETE FROM public.scoped_state_source WHERE id = 1;
SELECT mdm.refresh('scoped_state')->>'resolver_strategy' AS preparation_strategy;
BEGIN;
SAVEPOINT original_boundary;
INSERT INTO public.scoped_state_source VALUES (1, 'Record 1', '1@scope.test', '2026-10-18 00:00:00+00');
SELECT mdm.refresh('scoped_state')->>'resolver_strategy' AS strategy \gset affected_
SELECT jsonb_agg(jsonb_build_array(r->'name', r->'member_count', r->'has_review') ORDER BY r->>'name') AS rows
FROM jsonb_array_elements(public.scoped_state_snapshot()->'entities') r \gset affected_
ROLLBACK TO SAVEPOINT original_boundary;
INSERT INTO public.scoped_state_source VALUES (1, 'Record 1', '1@scope.test', '2026-10-18 00:00:00+00');
SELECT mdm_admin.rebuild('scoped_state')->>'resolver_strategy' AS strategy \gset full_
SELECT jsonb_agg(jsonb_build_array(r->'name', r->'member_count', r->'has_review') ORDER BY r->>'name') AS rows
FROM jsonb_array_elements(public.scoped_state_snapshot()->'entities') r \gset full_
SELECT jsonb_build_object('affected_strategy', :'affected_strategy', 'full_strategy', :'full_strategy',
    'affected_rows', :'affected_rows'::jsonb, 'full_rows', :'full_rows'::jsonb) AS comparison;
SELECT :'affected_rows'::jsonb IS NOT DISTINCT FROM :'full_rows'::jsonb AS equivalent \gset
ROLLBACK;
\if :equivalent
\echo 'PASS: same-boundary affected and full public review flags match'
\else
\echo 'FAIL: same-boundary affected and full public review flags differ (R6/R11)'
DO $$ BEGIN RAISE EXCEPTION 'R6/R11: same-boundary affected and full public review flags differ'; END $$;
\endif
