\set ON_ERROR_STOP on
\connect foundation postgres
CREATE EXTENSION IF NOT EXISTS dblink;
CREATE SCHEMA policy_pause_probe;
SELECT b.binding_id, r.runtime_version
FROM mdm_steward.policy_bindings_v1 b
JOIN mdm_internal.policy_binding_runtime r USING(binding_id)
JOIN mdm_internal.entities e USING(entity_id)
WHERE e.entity_name='policy_qualification' AND b.replaced_by IS NULL
ORDER BY b.created_at DESC LIMIT 1
\gset probe_
CREATE TABLE policy_pause_probe.before AS
SELECT jsonb_build_object(
    'binding',(SELECT to_jsonb(b) FROM mdm_steward.policy_bindings_v1 b WHERE binding_id=:'probe_binding_id'::uuid),
    'runtime',(SELECT to_jsonb(r) FROM mdm_internal.policy_binding_runtime r WHERE binding_id=:'probe_binding_id'::uuid),
    'cases',(SELECT jsonb_agg(to_jsonb(c) ORDER BY case_key) FROM mdm_steward.policy_cases_v1 c),
    'receipts',(SELECT jsonb_agg(to_jsonb(r) ORDER BY created_at,receipt_id) FROM mdm_steward.policy_receipts_v1 r)) AS vector;
SELECT dblink_connect('binding_holder','dbname=foundation user=postgres');
SELECT dblink_exec('binding_holder','BEGIN');
SELECT * FROM dblink('binding_holder',format(
    'SELECT binding_id::text FROM mdm_steward.policy_bindings_v1 WHERE binding_id=%L::uuid FOR UPDATE',
    :'probe_binding_id')) AS held(binding_id text);
CREATE TABLE policy_pause_probe.holder AS
SELECT * FROM dblink('binding_holder','SELECT pg_backend_pid()') AS held(pid integer);
SELECT dblink_connect('operator_pause','dbname=foundation user=mdm_legacy_login');
SELECT dblink_exec('operator_pause','SET ROLE mdm_legacy_administrator');
SELECT dblink_exec('operator_pause','SET application_name=''policy-pause-barrier''');
SELECT dblink_exec('operator_pause','SET statement_timeout=''15s''');
SELECT dblink_send_query('operator_pause',format(
    'SELECT mdm_admin.set_policy_binding_state(%L::uuid,%s,''paused'',''Binding delivery barrier regression'')',
    :'probe_binding_id',:probe_runtime_version));
DO $barrier$
DECLARE deadline timestamptz := clock_timestamp()+interval '10 seconds';
BEGIN
    LOOP
        IF dblink_is_busy('operator_pause')=0 THEN
            RAISE EXCEPTION 'pause acknowledged before the in-flight binding barrier was released';
        END IF;
        PERFORM pg_stat_clear_snapshot();
        EXIT WHEN EXISTS (
            SELECT FROM pg_stat_activity a,policy_pause_probe.holder h
            WHERE a.application_name='policy-pause-barrier' AND h.pid=ANY(pg_blocking_pids(a.pid)));
        IF clock_timestamp() >= deadline THEN
            RAISE EXCEPTION 'pause did not reach the binding barrier within its bound';
        END IF;
        PERFORM pg_sleep(0.01);
    END LOOP;
END
$barrier$;
SELECT dblink_exec('binding_holder','COMMIT');
SELECT runtime_version=:probe_runtime_version+1 AS pause_version_exact
FROM dblink_get_result('operator_pause') AS paused(runtime_version bigint)
\gset
\if :pause_version_exact
\else
\quit 1
\endif
DO $exact$
DECLARE b jsonb := (SELECT vector FROM policy_pause_probe.before);
    a jsonb;
BEGIN
    SELECT jsonb_build_object(
        'binding',(SELECT to_jsonb(x) FROM mdm_steward.policy_bindings_v1 x WHERE x.binding_id=(b#>>'{binding,binding_id}')::uuid),
        'runtime',(SELECT to_jsonb(x) FROM mdm_internal.policy_binding_runtime x WHERE x.binding_id=(b#>>'{binding,binding_id}')::uuid),
        'cases',(SELECT jsonb_agg(to_jsonb(c) ORDER BY case_key) FROM mdm_steward.policy_cases_v1 c),
        'receipts',(SELECT jsonb_agg(to_jsonb(r) ORDER BY created_at,receipt_id) FROM mdm_steward.policy_receipts_v1 r)) INTO a;
    IF a-'runtime' IS DISTINCT FROM b-'runtime'
       OR (a->'runtime')-'changed_at' IS DISTINCT FROM
          ((b->'runtime')-'changed_at') || jsonb_build_object('state','paused',
              'runtime_version',(b#>>'{runtime,runtime_version}')::bigint+1)
       OR (a#>>'{runtime,changed_at}')::timestamptz < (b#>>'{runtime,changed_at}')::timestamptz THEN
        RAISE EXCEPTION 'pause changed non-runtime state or returned an incorrect complete runtime row: before %, after %',b,a;
    END IF;
END
$exact$;
SELECT dblink_disconnect('operator_pause');
SELECT dblink_disconnect('binding_holder');
SELECT 'policy pause binding barrier and complete state passed' AS result;
