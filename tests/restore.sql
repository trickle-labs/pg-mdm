\set ON_ERROR_STOP on

SELECT 'public.crm_customer'::regclass::oid <> :original_source_oid
   AND 'mdm_administrator'::regrole::oid <> :original_role_oid AS new_bindings
\gset
\if :new_bindings
\else
\quit 1
\endif

CREATE TEMP TABLE e2e_restore_vars AS
SELECT :original_operations::bigint AS original_operations,
       :original_policy_max::bigint AS original_policy_max,
       :'original_policy_digest'::text AS original_policy_digest,
       :'restore_issue_key'::text AS restore_issue_key;

SELECT pgtrickle.recover_capture_instance();

DO $$
DECLARE actual jsonb;
BEGIN
    actual := jsonb_build_object(
        'customer', (SELECT to_jsonb(e) FROM mdm_internal.entities e WHERE entity_name = 'customer'),
        'definitions', (SELECT count(*) FROM mdm_internal.definitions d
                        JOIN mdm_internal.entities e USING (entity_id)
                        WHERE e.entity_name = 'customer'),
        'definition_artifacts', (SELECT count(*) FROM mdm_internal.definition_artifacts a
                                 JOIN mdm_internal.entities e USING (entity_id)
                                 WHERE e.entity_name = 'customer'),
        'source_identities', (SELECT count(*) FROM mdm_internal.source_identities s
                              JOIN mdm_internal.entities e USING (entity_id)
                              WHERE e.entity_name = 'customer'),
        'output_names', (SELECT count(*) FROM mdm_internal.output_names o
                         JOIN mdm_internal.entities e USING (entity_id)
                         WHERE e.entity_name = 'customer'),
        'steward_decisions', (SELECT count(*) FROM mdm_internal.steward_decisions d
                              JOIN mdm_internal.entities e USING (entity_id)
                              WHERE e.entity_name = 'customer'),
        'succeeded_operations', (SELECT count(*) FROM mdm_internal.operations WHERE status = 'succeeded' AND actor_name = 'mdm_test_login'),
        'active_source_records', (SELECT count(*) FROM mdm_internal.source_records r
                                  JOIN mdm_internal.entities e USING (entity_id)
                                  WHERE e.entity_name = 'customer' AND r.active)
    );
    -- The directive and pair-decision races add two active records and six successful operations.
    IF (SELECT count(*) FROM mdm_internal.entities WHERE entity_name = 'customer' AND desired_version = 4) <> 1
       OR (SELECT count(*) FROM mdm_internal.definitions d
           JOIN mdm_internal.entities e USING (entity_id)
           WHERE e.entity_name = 'customer') <> 4
       OR (SELECT count(*) FROM mdm_internal.definition_artifacts a
           JOIN mdm_internal.entities e USING (entity_id)
           WHERE e.entity_name = 'customer') <> 4
       OR (SELECT count(*) FROM mdm_internal.source_identities s
           JOIN mdm_internal.entities e USING (entity_id)
           WHERE e.entity_name = 'customer') <> 1
       OR (SELECT count(*) FROM mdm_internal.output_names o
           JOIN mdm_internal.entities e USING (entity_id)
           WHERE e.entity_name = 'customer') <> 3
       OR (SELECT count(*) FROM mdm_internal.steward_decisions d
           JOIN mdm_internal.entities e USING (entity_id)
           WHERE e.entity_name = 'customer') <> 4
       OR (SELECT decision_epoch FROM mdm_internal.entities WHERE entity_name = 'customer') <> 5
       OR (SELECT publication_revision FROM mdm_internal.entities WHERE entity_name = 'customer') <> 14
       OR (SELECT count(*) FROM mdm_internal.operations) <>
          (SELECT original_operations FROM e2e_restore_vars)
       OR (SELECT count(*) FROM mdm_internal.source_records r
           JOIN mdm_internal.entities e USING (entity_id)
           WHERE e.entity_name = 'customer' AND r.active) <> 9 THEN
        RAISE EXCEPTION 'durable catalog data did not survive restore: %', actual;
    END IF;
    IF (SELECT count(*) FROM mdm_internal.steward_decisions WHERE is_current) <> 3
       OR (SELECT count(*) FROM mdm_internal.steward_decisions WHERE NOT is_current) <> 1
       OR NOT EXISTS (
            SELECT 1
            FROM mdm_internal.steward_decisions newer
            JOIN mdm_internal.steward_decisions older
              ON older.decision_id = newer.supersedes
             AND older.entity_id = newer.entity_id
             AND older.left_source_record_id = newer.left_source_record_id
             AND older.right_source_record_id = newer.right_source_record_id
            WHERE newer.decision_version = 2
              AND newer.decision = 'NOT_MATCH'
              AND newer.is_current
              AND older.decision_version = 1
              AND older.decision = 'MATCH'
              AND NOT older.is_current
       )
       OR EXISTS (
            SELECT 1 FROM mdm_internal.steward_decisions
            WHERE created_by_name <> 'mdm_test_login'
               OR created_as_role_name <> 'mdm_administrator'
               OR operation_id IS NULL
               OR decision_epoch NOT BETWEEN 1 AND 5
       ) THEN
        RAISE EXCEPTION 'current and superseded steward decisions lost their durable meaning';
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM mdm_internal.golden_override_directives d
        JOIN mdm_internal.entities e USING (entity_id)
        JOIN mdm_internal.operations o USING (operation_id)
        WHERE e.entity_name = 'customer'
          AND d.action = 'SET'
          AND d.value = '"Race override"'::jsonb
          AND d.override_version = 1
          AND d.base_publication_revision = 11
          AND d.decision_epoch = 4
          AND d.reason = 'publication race'
          AND d.created_by_name = 'mdm_test_login'
          AND d.created_as_role_name = 'mdm_administrator'
          AND d.is_current
          AND o.operation_kind = 'golden_override'
          AND o.status = 'succeeded'
          AND o.result_code = 'MDM_OK'
          AND o.actor_name = 'mdm_test_login'
          AND o.actor_role_name = 'mdm_administrator'
    ) THEN
        RAISE EXCEPTION 'golden override or actor audit did not survive restore';
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM mdm_internal.steward_decisions d
        JOIN mdm_internal.entities e USING (entity_id)
        JOIN mdm_internal.operations o USING (operation_id)
        WHERE e.entity_name = 'customer'
          AND d.decision = 'MATCH'
          AND d.reason = 'pair decision publication race'
          AND d.decision_version = 1
          AND d.base_publication_revision = 13
          AND d.decision_epoch = 5
          AND d.created_by_name = 'mdm_test_login'
          AND d.created_as_role_name = 'mdm_administrator'
          AND d.is_current
          AND o.operation_kind = 'steward_decide'
          AND o.status = 'succeeded'
          AND o.result_code = 'MDM_OK'
          AND o.actor_name = 'mdm_test_login'
          AND o.actor_role_name = 'mdm_administrator'
          AND o.outcome->>'decision_id' = d.decision_id::text
          AND (o.outcome->>'decision_epoch')::bigint = d.decision_epoch
    ) THEN
        RAISE EXCEPTION 'pair decision or actor audit did not survive restore';
    END IF;
    IF EXISTS (SELECT FROM mdm_internal.source_bindings)
       OR EXISTS (SELECT FROM mdm_internal.execution_role_bindings) THEN
        RAISE EXCEPTION 'database-local bindings were included in the dump';
    END IF;
END
$$;

SELECT md5(COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) - 'last_observed_at' ORDER BY c.case_key)
                     FROM mdm_steward.policy_cases_v1 c
                     WHERE c.entity_name = 'policy_qualification'), '[]'::jsonb)::text)
       = :'original_policy_digest' AS policy_rows_survived
\gset
\if :policy_rows_survived
\else
\quit 1
\endif

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM mdm_steward.policy_cases_v1
        WHERE entity_name = 'policy_qualification'
          AND status = 'open'
          AND occurrence = 2
    ) OR NOT EXISTS (
        SELECT 1 FROM mdm_steward.policy_cases_v1
        WHERE entity_name = 'policy_qualification'
          AND status = 'resolved'
          AND occurrence = 1
    ) OR NOT EXISTS (
        SELECT 1 FROM mdm_steward.policy_cases_v1
        WHERE entity_name = 'policy_qualification'
          AND opened_at_source = 'administrator'
          AND opened_at = '2020-01-02 00:00:00+00'::timestamptz
          AND action_revision = 2
    ) OR EXISTS (
        SELECT 1
        FROM mdm_steward.policy_cases_v1 c
        LEFT JOIN mdm_internal.reviews r ON r.review_id = c.review_id
        WHERE c.entity_name = 'policy_qualification'
          AND r.review_id IS NULL
    ) OR EXISTS (
        SELECT 1
        FROM mdm_steward.policy_cases_v1 c
        JOIN mdm_internal.reviews r ON r.review_id = c.review_id
        WHERE c.entity_name = 'policy_qualification'
          AND c.opened_at_source = 'publication'
          AND NOT EXISTS (
              SELECT 1
              FROM mdm_internal.publications p
              JOIN mdm_internal.entities e USING (entity_id)
              WHERE e.entity_name = c.entity_name
                AND p.publication_revision = r.opened_revision)
    ) OR EXISTS (
        SELECT 1
        FROM mdm_steward.policy_cases_v1 c
        JOIN mdm_internal.reviews r ON r.review_id = c.review_id
        LEFT JOIN mdm_internal.publications p
          ON p.entity_id = r.entity_id AND p.publication_revision = r.resolved_revision
        WHERE c.entity_name = 'policy_qualification'
          AND c.status = 'resolved'
          AND (p.publication_revision IS NULL OR c.resolved_at IS DISTINCT FROM p.published_at)
    ) THEN
        RAISE EXCEPTION 'restored policy cases lost their retained review or opening publication';
    END IF;
END
$$;

GRANT USAGE ON SCHEMA public, mdm, mdm_admin, mdm_steward TO mdm_legacy_administrator;
GRANT SELECT, MAINTAIN ON public.policy_qualification_source TO mdm_legacy_administrator;
GRANT EXECUTE ON FUNCTION mdm_admin.rebind(text), mdm_admin.recompile(text), mdm_admin.rebuild(text), mdm.refresh(text, text),
    mdm_admin.verify_installation(),
    mdm_admin.backfill_policy_case_opened_at(bigint, timestamptz, text)
    TO mdm_legacy_administrator;
GRANT USAGE ON SCHEMA mdm_steward TO mdm_output_reader;
GRANT SELECT ON mdm_steward.policy_cases_v1 TO mdm_output_reader;
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

\connect restored postgres
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm_admin.rebind('policy_qualification');
    IF result->>'entity_name' IS DISTINCT FROM 'policy_qualification'
       OR result->>'rebound_sources' IS DISTINCT FROM '1' THEN
        RAISE EXCEPTION 'policy qualification rebind result is invalid: %', result;
    END IF;
END
$$;
RESET ROLE;

\connect restored mdm_legacy_login
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm_admin.recompile('policy_qualification');
    IF result->>'graph_recompiled' IS DISTINCT FROM 'true' THEN
        RAISE EXCEPTION 'policy qualification graph was not recompiled after restore: %', result;
    END IF;
END
$$;
RESET ROLE;

\connect restored mdm_legacy_login
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm_admin.rebuild('policy_qualification', 'ALLOW');
    IF result->>'entity_name' IS DISTINCT FROM 'policy_qualification' THEN
        RAISE EXCEPTION 'policy qualification graph rebuild is invalid: %', result;
    END IF;
END
$$;
RESET ROLE;

\connect restored postgres
CREATE TEMP TABLE e2e_restore_vars AS
SELECT :original_operations::bigint AS original_operations,
       :original_policy_max::bigint AS original_policy_max,
       :'original_policy_digest'::text AS original_policy_digest,
       :'restore_issue_key'::text AS restore_issue_key;
SET ROLE mdm_output_reader;
SELECT md5(COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) - 'last_observed_at' ORDER BY c.case_key)
                     FROM mdm_steward.policy_cases_v1 c
                     WHERE c.entity_name = 'policy_qualification'), '[]'::jsonb)::text)
       = :'original_policy_digest' AS policy_reader_rows_survived
\gset
\if :policy_reader_rows_survived
\else
\quit 1
\endif
RESET ROLE;

\connect restored postgres
CREATE TEMP TABLE e2e_restore_vars AS
SELECT :original_operations::bigint AS original_operations,
       :original_policy_max::bigint AS original_policy_max,
       :'original_policy_digest'::text AS original_policy_digest,
       :'restore_issue_key'::text AS restore_issue_key;
SELECT md5(COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) - 'last_observed_at' ORDER BY c.case_key)
                     FROM mdm_steward.policy_cases_v1 c
                     WHERE c.entity_name = 'policy_qualification'), '[]'::jsonb)::text)
       = :'original_policy_digest' AS policy_rows_survived_rebuild
\gset
\if :policy_rows_survived_rebuild
\else
\quit 1
\endif
CREATE TABLE public.e2e_restore_policy_snapshot AS
SELECT c.case_key, pg_catalog.to_jsonb(c) - 'last_observed_at' AS state
FROM mdm_steward.policy_cases_v1 c
WHERE c.entity_name = 'policy_qualification'
  AND c.case_key <= :original_policy_max;
UPDATE public.policy_qualification_source
SET email_address = 'restore-pair@example.test', updated_at = statement_timestamp()
WHERE id = 202;
SELECT md5(COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) - 'last_observed_at' ORDER BY c.case_key)
                     FROM mdm_steward.policy_cases_v1 c
                     WHERE c.entity_name = 'policy_qualification'
                       AND c.case_key <= :original_policy_max), '[]'::jsonb)::text)
       = :'original_policy_digest' AS restored_rows_before_recurrence
\gset
\if :restored_rows_before_recurrence
\else
\quit 1
\endif

\connect restored mdm_test_login
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('policy_qualification', 'ALLOW');
    IF result->>'changed' IS DISTINCT FROM 'true' THEN
        RAISE EXCEPTION 'post-restore recurrence did not publish: %', result;
    END IF;
END
$$;
RESET ROLE;

\connect restored postgres
CREATE TEMP TABLE e2e_restore_vars AS
SELECT :original_operations::bigint AS original_operations,
       :original_policy_max::bigint AS original_policy_max,
       :'original_policy_digest'::text AS original_policy_digest,
       :'restore_issue_key'::text AS restore_issue_key;
DO $$
DECLARE new_case record; old_case record; opening_at timestamptz; changed_rows jsonb;
BEGIN
    SELECT c.case_key, c.review_id, c.issue_key, c.occurrence, c.opened_at,
           c.opened_at_source, c.action_revision, c.status, r.opened_revision
    INTO STRICT new_case
    FROM mdm_steward.policy_cases_v1 c
    JOIN mdm_internal.reviews r USING (review_id)
    WHERE c.entity_name = 'policy_qualification'
      AND c.issue_key = pg_catalog.decode(
          (SELECT restore_issue_key FROM e2e_restore_vars), 'hex')
      AND c.occurrence = 2;
    SELECT c.case_key, c.review_id, c.issue_key, c.occurrence
    INTO STRICT old_case
    FROM mdm_steward.policy_cases_v1 c
    WHERE c.entity_name = 'policy_qualification'
      AND c.issue_key = pg_catalog.decode(
          (SELECT restore_issue_key FROM e2e_restore_vars), 'hex')
      AND c.occurrence = 1;
    SELECT p.published_at INTO STRICT opening_at
    FROM mdm_internal.publications p
    JOIN mdm_internal.entities e USING (entity_id)
    WHERE e.entity_name = 'policy_qualification'
      AND p.publication_revision = new_case.opened_revision;
    IF new_case.case_key <= (SELECT original_policy_max FROM e2e_restore_vars)
       OR old_case.review_id = new_case.review_id
       OR new_case.issue_key IS DISTINCT FROM pg_catalog.decode(
           (SELECT restore_issue_key FROM e2e_restore_vars), 'hex')
       OR old_case.occurrence <> 1
       OR new_case.occurrence <> 2
       OR new_case.status <> 'open'
       OR new_case.opened_at_source <> 'publication'
       OR new_case.action_revision <> 1
       OR new_case.opened_at IS DISTINCT FROM opening_at THEN
        RAISE EXCEPTION 'post-restore recurrence changed identity or opening time: old %, new %', old_case, new_case;
    END IF;
    SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(
               'case_key', c.case_key,
               'before', s.state,
               'after', pg_catalog.to_jsonb(c) - 'last_observed_at')
           ORDER BY c.case_key)
    INTO changed_rows
    FROM mdm_steward.policy_cases_v1 c
    JOIN public.e2e_restore_policy_snapshot s USING (case_key)
    WHERE pg_catalog.to_jsonb(c) - 'last_observed_at' IS DISTINCT FROM s.state;
    IF changed_rows IS NOT NULL THEN
        RAISE EXCEPTION 'restored policy rows changed while publishing the new recurrence: %', changed_rows;
    END IF;
END
$$;
DROP TABLE public.e2e_restore_policy_snapshot;

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
