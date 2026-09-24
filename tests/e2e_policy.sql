\set ON_ERROR_STOP on

\connect foundation postgres

DO $$
DECLARE
    columns text[];
    review_count bigint;
    case_count bigint;
BEGIN
    SELECT pg_catalog.array_agg(
        a.attname || ':' || pg_catalog.format_type(a.atttypid, a.atttypmod)
        ORDER BY a.attnum
    )
      INTO columns
      FROM pg_catalog.pg_attribute a
     WHERE a.attrelid = 'mdm_steward.policy_cases_v1'::pg_catalog.regclass
       AND a.attnum > 0
       AND NOT a.attisdropped;
    IF columns IS DISTINCT FROM ARRAY[
        'case_key:bigint', 'review_id:uuid', 'entity_name:name',
        'issue_key:bytea', 'occurrence:integer', 'status:text',
        'severity:text', 'reason_code:text', 'approved_metadata:jsonb',
        'permitted_actions:text[]', 'assigned_queue:name',
        'due_at:timestamp with time zone', 'escalation_level:integer',
        'manual_assignment_protected:boolean',
        'opened_at:timestamp with time zone', 'opened_at_source:text',
        'resolved_at:timestamp with time zone', 'review_version:bigint',
        'definition_version:bigint', 'publication_revision:bigint',
        'stewardship_epoch:bigint', 'evidence_basis_digest:bytea',
        'action_revision:bigint', 'pending_stewardship:boolean',
        'last_observed_at:timestamp with time zone'
    ] THEN
        RAISE EXCEPTION 'policy projection columns are invalid: %', columns;
    END IF;
    SELECT count(*) INTO review_count FROM mdm_internal.reviews;
    SELECT count(*) INTO case_count FROM mdm_steward.policy_cases_v1;
    IF review_count <> case_count THEN
        RAISE EXCEPTION 'policy projection lost review occurrences: reviews %, cases %', review_count, case_count;
    END IF;
    IF EXISTS (
        SELECT 1 FROM mdm_steward.policy_cases_v1
        WHERE octet_length(issue_key) <> 32
           OR octet_length(evidence_basis_digest) <> 32
           OR case_key <= 0
           OR occurrence <= 0
           OR review_version <= 0
           OR definition_version <= 0
           OR publication_revision < 0
           OR stewardship_epoch < 0
           OR action_revision <= 0
           OR approved_metadata <> '{}'::jsonb
           OR (status = 'resolved' AND cardinality(permitted_actions) <> 0)
           OR (status = 'open' AND 'SET_DUE_AT' = ANY(permitted_actions) AND opened_at IS NULL)
    ) THEN
        RAISE EXCEPTION 'policy projection contains invalid row values';
    END IF;
    IF pg_catalog.has_table_privilege('public', 'mdm_steward.policy_cases_v1', 'SELECT') THEN
        RAISE EXCEPTION 'policy projection grants SELECT to PUBLIC';
    END IF;
    IF (SELECT last_value FROM mdm_internal.policy_case_key_seq)
       < COALESCE((SELECT max(case_key) FROM mdm_steward.policy_cases_v1), 0) THEN
        RAISE EXCEPTION 'policy case sequence is behind restored rows';
    END IF;
END
$$;

\connect foundation postgres
GRANT EXECUTE ON FUNCTION mdm_admin.backfill_policy_case_opened_at(bigint, timestamptz, text)
    TO mdm_administrator, mdm_legacy_administrator;
GRANT USAGE ON SCHEMA mdm_steward TO mdm_legacy_administrator;
GRANT USAGE ON SCHEMA mdm_steward TO mdm_output_reader;
GRANT SELECT ON mdm_steward.policy_cases_v1 TO mdm_output_reader;

SELECT count(*) = 2 AS legacy_unknown_open,
       min(case_key) AS legacy_case_key,
       max(case_key) AS rollback_case_key
FROM mdm_steward.policy_cases_v1
WHERE entity_name = 'policy_qualification'
  AND status = 'open'
  AND opened_at IS NULL
  AND opened_at_source = 'unknown'
\gset
\if :legacy_unknown_open
\else
\quit 1
\endif

CREATE TABLE public.e2e_policy_vars (
    legacy_case_key bigint NOT NULL,
    rollback_case_key bigint NOT NULL,
    backfill_operation_id uuid,
    backfill_action_revision bigint,
    legacy_history_before text,
    known_case_key bigint,
    known_issue_key text,
    known_review_id text,
    known_opened_revision bigint
);
INSERT INTO public.e2e_policy_vars (legacy_case_key, rollback_case_key)
VALUES (:'legacy_case_key', :'rollback_case_key');
REVOKE ALL ON public.e2e_policy_vars FROM PUBLIC;
GRANT SELECT ON public.e2e_policy_vars TO mdm_administrator, mdm_legacy_administrator;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM mdm_steward.policy_cases_v1
        WHERE entity_name = 'policy_qualification'
          AND opened_at_source = 'unknown'
          AND 'SET_DUE_AT' = ANY(permitted_actions)
    ) THEN
        RAISE EXCEPTION 'unknown policy cases incorrectly permit SET_DUE_AT';
    END IF;
END
$$;

SELECT md5(COALESCE(string_agg(
    p.publication_revision::text || ':' || pg_catalog.encode(p.result_digest, 'hex'),
    '|' ORDER BY p.publication_revision), '')) AS legacy_history_before
FROM mdm_internal.publications p
JOIN mdm_internal.entities e USING (entity_id)
WHERE e.entity_name = 'policy_qualification'
\gset

UPDATE public.e2e_policy_vars
SET legacy_history_before = :'legacy_history_before';

\connect foundation mdm_legacy_login
SET ROLE mdm_legacy_administrator;
SELECT operation_id, action_revision
FROM mdm_admin.backfill_policy_case_opened_at(
    :legacy_case_key,
    '2020-01-02 00:00:00+00'::timestamptz,
    'legacy opening time recovered'
)
\gset backfill_

\connect foundation postgres
UPDATE public.e2e_policy_vars
SET backfill_operation_id = :'backfill_operation_id'::uuid,
    backfill_action_revision = :'backfill_action_revision';
DO $$
DECLARE operation_row record;
BEGIN
    SELECT o.operation_kind, o.status, o.result_code, o.actor_name, o.actor_role_name,
           o.outcome->>'case_key' AS case_key, o.outcome->>'reason' AS reason,
           o.outcome->>'action_revision' AS action_revision
    INTO STRICT operation_row
    FROM mdm_internal.operations o
    WHERE o.operation_id = (SELECT backfill_operation_id FROM public.e2e_policy_vars);
    IF operation_row.operation_kind <> 'policy_case_opened_at_backfill'
       OR operation_row.status <> 'succeeded'
       OR operation_row.result_code <> 'MDM_OK'
       OR operation_row.actor_name <> 'mdm_legacy_login'
       OR operation_row.actor_role_name <> 'mdm_legacy_administrator'
       OR operation_row.case_key <> (SELECT legacy_case_key::text FROM public.e2e_policy_vars)
       OR operation_row.reason <> 'legacy opening time recovered'
       OR operation_row.action_revision <> (SELECT backfill_action_revision::text FROM public.e2e_policy_vars) THEN
        RAISE EXCEPTION 'backfill operation audit is invalid: %', operation_row;
    END IF;
    IF (SELECT opened_at FROM mdm_steward.policy_cases_v1 WHERE case_key = (SELECT legacy_case_key FROM public.e2e_policy_vars))
           IS DISTINCT FROM '2020-01-02 00:00:00+00'::timestamptz
       OR (SELECT opened_at_source FROM mdm_steward.policy_cases_v1 WHERE case_key = (SELECT legacy_case_key FROM public.e2e_policy_vars))
           <> 'administrator'
       OR (SELECT action_revision FROM mdm_steward.policy_cases_v1 WHERE case_key = (SELECT legacy_case_key FROM public.e2e_policy_vars))
           <> (SELECT backfill_action_revision FROM public.e2e_policy_vars)
       OR array_position((SELECT permitted_actions FROM mdm_steward.policy_cases_v1 WHERE case_key = (SELECT legacy_case_key FROM public.e2e_policy_vars)), 'SET_DUE_AT') IS NULL THEN
        RAISE EXCEPTION 'authorized backfill did not update the exact public row';
    END IF;
    IF md5(COALESCE((SELECT string_agg(
        p.publication_revision::text || ':' || pg_catalog.encode(p.result_digest, 'hex'),
        '|' ORDER BY p.publication_revision)
        FROM mdm_internal.publications p
        JOIN mdm_internal.entities e USING (entity_id)
        WHERE e.entity_name = 'policy_qualification'), '')) <> (SELECT legacy_history_before FROM public.e2e_policy_vars) THEN
        RAISE EXCEPTION 'backfill changed publication history';
    END IF;
END
$$;

CREATE TABLE public.e2e_policy_backfill_snapshot (
    case_key bigint PRIMARY KEY,
    state jsonb NOT NULL,
    operation_count bigint NOT NULL
);
INSERT INTO public.e2e_policy_backfill_snapshot
SELECT c.case_key, pg_catalog.to_jsonb(c),
       (SELECT count(*) FROM mdm_internal.operations)
FROM mdm_steward.policy_cases_v1 c
WHERE c.case_key IN (:legacy_case_key, :rollback_case_key);
REVOKE ALL ON public.e2e_policy_backfill_snapshot FROM PUBLIC;

\connect foundation mdm_test_login
SET ROLE mdm_administrator;
DO $$
BEGIN
    BEGIN
        PERFORM mdm_admin.backfill_policy_case_opened_at(
            (SELECT rollback_case_key FROM public.e2e_policy_vars),
            '2020-01-03 00:00:00+00'::timestamptz, 'wrong role');
        RAISE EXCEPTION 'unauthorized backfill succeeded';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_UNAUTHORIZED') = 0 THEN RAISE; END IF;
    END;
END
$$;
RESET ROLE;

\connect foundation mdm_legacy_login
SET ROLE mdm_legacy_administrator;
DO $$
BEGIN
    BEGIN
        PERFORM mdm_admin.backfill_policy_case_opened_at(
            999999999, '2020-01-03 00:00:00+00'::timestamptz, 'missing case');
        RAISE EXCEPTION 'missing-case backfill succeeded';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_POLICY_PROJECTION') = 0 THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM mdm_admin.backfill_policy_case_opened_at(
            (SELECT rollback_case_key FROM public.e2e_policy_vars),
            '2020-01-03 00:00:00+00'::timestamptz, '');
        RAISE EXCEPTION 'empty backfill reason succeeded';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_POLICY_PROJECTION') = 0 THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM mdm_admin.backfill_policy_case_opened_at(
            (SELECT rollback_case_key FROM public.e2e_policy_vars),
            '2999-01-01 00:00:00+00'::timestamptz, 'too late');
        RAISE EXCEPTION 'late backfill succeeded';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_POLICY_PROJECTION') = 0 THEN RAISE; END IF;
    END;
    BEGIN
        PERFORM mdm_admin.backfill_policy_case_opened_at(
            (SELECT legacy_case_key FROM public.e2e_policy_vars),
            '2020-01-03 00:00:00+00'::timestamptz, 'second backfill');
        RAISE EXCEPTION 'second backfill succeeded';
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'MDM_POLICY_PROJECTION') = 0 THEN RAISE; END IF;
    END;
END
$$;
BEGIN;
DO $$
DECLARE result record;
BEGIN
    SELECT * INTO STRICT result
    FROM mdm_admin.backfill_policy_case_opened_at(
        (SELECT rollback_case_key FROM public.e2e_policy_vars),
        '2020-01-02 00:00:00+00'::timestamptz, 'rolled back');
    IF result.action_revision <> 2 THEN
        RAISE EXCEPTION 'rollback backfill returned an unexpected action revision: %', result;
    END IF;
END
$$;
ROLLBACK;

\connect foundation postgres
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM mdm_steward.policy_cases_v1 c
        JOIN public.e2e_policy_backfill_snapshot s USING (case_key)
        WHERE pg_catalog.to_jsonb(c) IS DISTINCT FROM s.state
    ) OR EXISTS (
        SELECT 1
        FROM public.e2e_policy_backfill_snapshot s
        WHERE s.operation_count <> (SELECT count(*) FROM mdm_internal.operations)
    ) THEN
        RAISE EXCEPTION 'rejected or rolled-back backfill changed a case or audit row';
    END IF;
END
$$;
DROP TABLE public.e2e_policy_backfill_snapshot;

\connect foundation postgres
INSERT INTO public.policy_qualification_source VALUES
    (101, 'Known One', 'known-one@example.test', statement_timestamp()),
    (102, 'Known One', 'known-pair@example.test', statement_timestamp()),
    (103, 'Known Two', 'known-two@example.test', statement_timestamp()),
    (104, 'Known Two', 'known-pair@example.test', statement_timestamp()),
    (105, 'Publication One', 'publication-one@example.test', statement_timestamp()),
    (106, 'Publication Two', 'publication-two@example.test', statement_timestamp());
\connect foundation mdm_legacy_login
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('policy_qualification', 'ALLOW');
    IF result->>'changed' IS DISTINCT FROM 'true' THEN
        RAISE EXCEPTION 'known policy opening did not publish: %', result;
    END IF;
END
$$;

\connect foundation postgres
SELECT case_key,
       encode(c.issue_key, 'hex') AS issue_key,
       c.review_id::text AS review_id,
       r.opened_revision,
       c.action_revision,
       c.publication_revision
FROM mdm_steward.policy_cases_v1 c
JOIN mdm_internal.reviews r USING (review_id)
WHERE c.entity_name = 'policy_qualification'
  AND c.opened_at_source = 'publication'
  AND c.occurrence = 1
  AND c.status = 'open'
ORDER BY case_key DESC
LIMIT 1
\gset known_
SELECT :known_case_key IS NOT NULL AS known_case_found
\gset
\if :known_case_found
\else
\quit 1
\endif

UPDATE public.e2e_policy_vars
SET known_case_key = :'known_case_key',
    known_issue_key = :'known_issue_key',
    known_review_id = :'known_review_id',
    known_opened_revision = :'known_opened_revision';

DO $$
BEGIN
    IF (SELECT c.opened_at
        FROM mdm_steward.policy_cases_v1 c
        WHERE c.case_key = (SELECT known_case_key FROM public.e2e_policy_vars)) IS DISTINCT FROM
       (SELECT p.published_at
        FROM mdm_internal.publications p
        JOIN mdm_internal.entities e USING (entity_id)
        WHERE e.entity_name = 'policy_qualification'
          AND p.publication_revision = (SELECT known_opened_revision FROM public.e2e_policy_vars)) THEN
        RAISE EXCEPTION 'known policy opening time is not the retained publication time';
    END IF;
END
$$;

CREATE TABLE public.e2e_policy_lifecycle_snapshot (
    case_key bigint PRIMARY KEY,
    state jsonb NOT NULL,
    resolved_state jsonb,
    publication_revision bigint NOT NULL
);
INSERT INTO public.e2e_policy_lifecycle_snapshot
SELECT c.case_key, pg_catalog.to_jsonb(c), NULL,
       (SELECT max(p.publication_revision) FROM mdm_internal.publications p
        JOIN mdm_internal.entities e USING (entity_id)
        WHERE e.entity_name = 'policy_qualification')
FROM mdm_steward.policy_cases_v1 c
WHERE c.case_key = :known_case_key;
REVOKE ALL ON public.e2e_policy_lifecycle_snapshot FROM PUBLIC;

UPDATE public.policy_qualification_source
SET display_name = 'Publication Two', updated_at = statement_timestamp()
WHERE id = 105;
\connect foundation mdm_legacy_login
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('policy_qualification', 'ALLOW');
    IF result->>'changed' IS DISTINCT FROM 'true' THEN
        RAISE EXCEPTION 'publication-only policy update did not publish: %', result;
    END IF;
END
$$;

\connect foundation postgres
DO $$
DECLARE before_state jsonb; after_state jsonb;
BEGIN
    SELECT state INTO STRICT before_state
    FROM public.e2e_policy_lifecycle_snapshot
    WHERE case_key = (SELECT known_case_key FROM public.e2e_policy_vars);
    SELECT pg_catalog.to_jsonb(c) INTO STRICT after_state
    FROM mdm_steward.policy_cases_v1 c
    WHERE c.case_key = (SELECT known_case_key FROM public.e2e_policy_vars);
    IF after_state - 'last_observed_at' IS DISTINCT FROM before_state - 'last_observed_at'
       OR (SELECT publication_revision FROM public.e2e_policy_lifecycle_snapshot
           WHERE case_key = (SELECT known_case_key FROM public.e2e_policy_vars)) + 1 <>
          (SELECT max(p.publication_revision) FROM mdm_internal.publications p
           JOIN mdm_internal.entities e USING (entity_id)
           WHERE e.entity_name = 'policy_qualification')
       OR (after_state->>'last_observed_at')::timestamptz < (before_state->>'last_observed_at')::timestamptz THEN
        RAISE EXCEPTION 'publication-only update changed policy meaning or was not retained';
    END IF;
END
$$;

DELETE FROM public.policy_qualification_source WHERE id = 102;
\connect foundation mdm_legacy_login
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('policy_qualification', 'ALLOW');
    IF result->>'changed' IS DISTINCT FROM 'true' THEN
        RAISE EXCEPTION 'policy resolution did not publish: %', result;
    END IF;
END
$$;

\connect foundation postgres
DO $$
DECLARE resolved_row jsonb; resolved_revision bigint; resolved_at timestamptz;
BEGIN
    SELECT pg_catalog.to_jsonb(c), r.resolved_revision, c.resolved_at
    INTO STRICT resolved_row, resolved_revision, resolved_at
    FROM mdm_steward.policy_cases_v1 c
    JOIN mdm_internal.reviews r USING (review_id)
    WHERE c.case_key = (SELECT known_case_key FROM public.e2e_policy_vars);
    IF resolved_row->>'status' <> 'resolved'
       OR (SELECT cardinality(permitted_actions) FROM mdm_steward.policy_cases_v1
           WHERE case_key = (SELECT known_case_key FROM public.e2e_policy_vars)) <> 0
       OR resolved_row->>'case_key' <> (SELECT known_case_key::text FROM public.e2e_policy_vars)
       OR resolved_row->>'review_id' <> (SELECT known_review_id FROM public.e2e_policy_vars)
       OR encode((SELECT issue_key
                  FROM mdm_steward.policy_cases_v1
                  WHERE case_key = (SELECT known_case_key FROM public.e2e_policy_vars)), 'hex')
          IS DISTINCT FROM (SELECT known_issue_key FROM public.e2e_policy_vars)
       OR resolved_row->>'occurrence' <> '1'
       OR resolved_row->>'opened_at' IS NULL
       OR resolved_revision IS NULL
       OR resolved_at IS DISTINCT FROM (
           SELECT p.published_at
           FROM mdm_internal.publications p
           JOIN mdm_internal.entities e USING (entity_id)
           WHERE e.entity_name = 'policy_qualification'
             AND p.publication_revision = resolved_revision) THEN
        RAISE EXCEPTION 'resolved policy row lost identity, opening time, or matching resolution time: %', resolved_row;
    END IF;
    UPDATE public.e2e_policy_lifecycle_snapshot
    SET resolved_state = resolved_row
    WHERE case_key = (SELECT known_case_key FROM public.e2e_policy_vars);
END
$$;

INSERT INTO public.policy_qualification_source
VALUES (102, 'Known One', 'known-pair@example.test', statement_timestamp());
\connect foundation mdm_legacy_login
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('policy_qualification', 'ALLOW');
    IF result->>'changed' IS DISTINCT FROM 'true' THEN
        RAISE EXCEPTION 'policy recurrence did not publish: %', result;
    END IF;
END
$$;

\connect foundation postgres
DO $$
DECLARE old_state jsonb; new_state jsonb; new_opened_at timestamptz;
        opening_publication_at timestamptz;
BEGIN
    SELECT pg_catalog.to_jsonb(c) INTO STRICT old_state
    FROM mdm_steward.policy_cases_v1 c
    WHERE c.entity_name = 'policy_qualification'
      AND c.issue_key = pg_catalog.decode((SELECT known_issue_key FROM public.e2e_policy_vars), 'hex')
      AND c.occurrence = 1;
    SELECT pg_catalog.to_jsonb(c), c.opened_at, p.published_at
    INTO STRICT new_state, new_opened_at, opening_publication_at
    FROM mdm_steward.policy_cases_v1 c
    JOIN mdm_internal.reviews r USING (review_id)
    JOIN mdm_internal.entities e ON e.entity_name = c.entity_name
    JOIN mdm_internal.publications p
      ON p.entity_id = e.entity_id AND p.publication_revision = r.opened_revision
    WHERE c.entity_name = 'policy_qualification'
      AND c.issue_key = pg_catalog.decode((SELECT known_issue_key FROM public.e2e_policy_vars), 'hex')
      AND c.occurrence = 2;
    IF new_state->>'case_key' IS NULL
       OR (new_state->>'case_key')::bigint <= (SELECT known_case_key FROM public.e2e_policy_vars)
       OR new_state->>'review_id' = (SELECT known_review_id FROM public.e2e_policy_vars)
       OR new_state->>'occurrence' <> '2'
       OR new_state->>'action_revision' <> '1'
       OR new_state->>'opened_at_source' <> 'publication'
       OR new_opened_at IS DISTINCT FROM opening_publication_at
       OR old_state - 'last_observed_at' IS DISTINCT FROM
          ((SELECT resolved_state FROM public.e2e_policy_lifecycle_snapshot
            WHERE case_key = (SELECT known_case_key FROM public.e2e_policy_vars)) - 'last_observed_at') THEN
        RAISE EXCEPTION 'recurrence did not allocate a new exact policy row: old %, new %', old_state, new_state;
    END IF;
END
$$;

\connect foundation postgres
INSERT INTO public.policy_qualification_source VALUES
    (201, 'Restore One', 'restore-one@example.test', statement_timestamp()),
    (202, 'Restore One', 'restore-pair@example.test', statement_timestamp()),
    (203, 'Restore Two', 'restore-two@example.test', statement_timestamp()),
    (204, 'Restore Two', 'restore-pair@example.test', statement_timestamp());
\connect foundation mdm_legacy_login
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('policy_qualification', 'ALLOW');
    IF result->>'changed' IS DISTINCT FROM 'true' THEN
        RAISE EXCEPTION 'restore policy opening did not publish: %', result;
    END IF;
END
$$;
\connect foundation postgres
DELETE FROM public.policy_qualification_source WHERE id = 202;
\connect foundation mdm_legacy_login
SET ROLE mdm_legacy_administrator;
DO $$
DECLARE result jsonb;
BEGIN
    result := mdm.refresh('policy_qualification', 'ALLOW');
    IF result->>'changed' IS DISTINCT FROM 'true' THEN
        RAISE EXCEPTION 'restore policy resolution did not publish: %', result;
    END IF;
END
$$;


\connect foundation postgres
DO $$
BEGIN
    IF (SELECT status FROM mdm_steward.policy_cases_v1
        WHERE entity_name = 'policy_qualification'
        ORDER BY case_key DESC LIMIT 1) <> 'resolved' THEN
        RAISE EXCEPTION 'restore fixture did not finish with a resolved case';
    END IF;
END
$$;

CREATE TEMP TABLE e2e_policy_reader_expected AS
SELECT COALESCE(
    pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) ORDER BY c.case_key),
    '[]'::jsonb
) AS state
FROM mdm_steward.policy_cases_v1 c
WHERE c.entity_name = 'policy_qualification';
GRANT SELECT ON e2e_policy_reader_expected TO mdm_output_reader;
SET ROLE mdm_output_reader;
DO $$
DECLARE expected jsonb; actual jsonb;
BEGIN
    SELECT state INTO STRICT expected FROM e2e_policy_reader_expected;
    SELECT COALESCE(
        pg_catalog.jsonb_agg(pg_catalog.to_jsonb(c) ORDER BY c.case_key),
        '[]'::jsonb
    ) INTO actual
    FROM mdm_steward.policy_cases_v1 c
    WHERE c.entity_name = 'policy_qualification';
    IF actual IS DISTINCT FROM expected THEN
        RAISE EXCEPTION 'policy reader did not receive the exact ordered public rows: %', actual;
    END IF;
    BEGIN
        PERFORM count(*) FROM mdm_internal.reviews;
        RAISE EXCEPTION 'policy reader accessed private review history';
    EXCEPTION WHEN insufficient_privilege THEN NULL;
    END;
END
$$;
RESET ROLE;
\connect foundation postgres
DROP TABLE public.e2e_policy_vars;
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mdm_policy_worker') THEN
        CREATE ROLE mdm_policy_worker NOLOGIN NOSUPERUSER NOBYPASSRLS;
    END IF;
END
$$;
GRANT mdm_policy_worker TO mdm_legacy_login WITH SET TRUE, INHERIT FALSE;
SELECT execution_role_name AS policy_execution_role
FROM mdm_internal.entities
WHERE entity_name = 'policy_qualification'
\gset
\connect foundation mdm_legacy_login
SET ROLE :"policy_execution_role";
SELECT binding_id AS first_binding_id, binding_version AS first_binding_version
FROM mdm_admin.create_policy_binding(
    'policy_qualification', 'mdm_policy_worker', pg_catalog.decode(pg_catalog.repeat('a', 64), 'hex'),
    ARRAY['ASSIGN_QUEUE'], ARRAY['priority']::text[], NULL, 0
)
\gset
SELECT binding_id AS second_binding_id, binding_version AS second_binding_version
FROM mdm_admin.replace_policy_binding(
    :'first_binding_id'::uuid, :first_binding_version,
    pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    ARRAY['ASSIGN_QUEUE', 'ESCALATE', 'SET_DUE_AT'],
    ARRAY['priority', 'urgent']::text[], INTERVAL '30 days', 3
)
\gset
RESET ROLE;
\connect foundation postgres
DO $$
BEGIN
    IF (SELECT state FROM mdm_internal.policy_binding_runtime r
        WHERE r.binding_id = (SELECT b.binding_id FROM mdm_steward.policy_bindings_v1 b WHERE b.automation_role_name = 'mdm_policy_worker' AND b.replaced_by IS NOT NULL)) <> 'paused'
       OR (SELECT state FROM mdm_internal.policy_binding_runtime r
           WHERE r.binding_id = (SELECT b.replaced_by FROM mdm_steward.policy_bindings_v1 b WHERE b.automation_role_name = 'mdm_policy_worker' AND b.replaced_by IS NOT NULL)) <> 'paused'
       OR (SELECT binding_version FROM mdm_steward.policy_bindings_v1
           WHERE automation_role_name = 'mdm_policy_worker' AND replaced_by IS NULL) <> 1
       OR (SELECT count(*) FROM mdm_steward.policy_bindings_v1 WHERE automation_role_name = 'mdm_policy_worker') <> 2 THEN
        RAISE EXCEPTION 'replacement did not retain and pause exact binding state';
    END IF;
END
$$;
\connect foundation mdm_legacy_login
SET ROLE :"policy_execution_role";
SELECT mdm_admin.set_policy_binding_state(:'second_binding_id'::uuid, 1, 'active', 'reconciled') AS active_runtime_version
\gset
\connect foundation mdm_legacy_login
SET ROLE mdm_policy_worker;
SELECT case_key, review_version, definition_version, publication_revision,
       stewardship_epoch, pg_catalog.encode(evidence_basis_digest, 'hex') AS evidence_digest,
       action_revision,
       CASE WHEN assigned_queue::text = 'priority' THEN 'urgent' ELSE 'priority' END AS policy_queue
FROM mdm_steward.policy_cases_v1
WHERE entity_name = 'policy_qualification'
  AND status = 'open'
  AND case_key <> :legacy_case_key
  AND NOT pending_stewardship
  AND NOT manual_assignment_protected
  AND 'ASSIGN_QUEUE' = ANY(permitted_actions)
ORDER BY case_key
LIMIT 1
\gset
SELECT receipt_id, outcome, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('c', 64), 'hex'),
    :case_key, 'ASSIGN_QUEUE', pg_catalog.jsonb_build_object('queue', :'policy_queue'),
    :review_version, :definition_version, :publication_revision, :stewardship_epoch,
    pg_catalog.decode(:'evidence_digest', 'hex'), :action_revision,
    pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'), 'policy-1', 'eval-1', 'work-1'
)
\gset applied_
SELECT receipt_id, outcome, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('c', 64), 'hex'),
    :case_key, 'ASSIGN_QUEUE', pg_catalog.jsonb_build_object('queue', :'policy_queue'),
    :review_version, :definition_version, :publication_revision, :stewardship_epoch,
    pg_catalog.decode(:'evidence_digest', 'hex'), :action_revision,
    pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'), 'policy-1', 'eval-1', 'work-1'
)
\gset retry_
SELECT :'applied_outcome' = 'APPLIED_CONTROL'
   AND :'retry_outcome' = 'APPLIED_CONTROL'
   AND :'applied_receipt_id'::uuid = :'retry_receipt_id'::uuid
   AND :applied_action_revision = :action_revision + 1
   AND :retry_action_revision = :applied_action_revision AS binding_intent_retry_ok
\gset
\if :binding_intent_retry_ok
\else
\quit 1
\endif
SELECT assigned_queue::text = :'policy_queue'
   AND action_revision = :applied_action_revision AS binding_intent_control_ok
FROM mdm_steward.policy_cases_v1 WHERE case_key = :case_key
\gset
\if :binding_intent_control_ok
\else
\quit 1
\endif
SELECT count(*) = 1 AND pg_catalog.bool_and(resulting_publication_revision IS NULL)
       AS binding_intent_receipt_ok
FROM mdm_steward.policy_receipts_v1
WHERE binding_id = :'second_binding_id'::uuid
  AND request_key = pg_catalog.decode(pg_catalog.repeat('c', 64), 'hex')
  AND outcome = 'APPLIED_CONTROL';
\gset
\if :binding_intent_receipt_ok
\else
\quit 1
\endif
SELECT case_key, review_version, definition_version, publication_revision,
       stewardship_epoch, pg_catalog.encode(evidence_basis_digest, 'hex') AS evidence_digest,
       action_revision, assigned_queue::text AS policy_queue
FROM mdm_steward.policy_cases_v1
WHERE case_key = :case_key
\gset current_
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('d', 64), 'hex'),
    :current_case_key, 'ASSIGN_QUEUE', pg_catalog.jsonb_build_object('queue', :'current_policy_queue'),
    :current_review_version, :current_definition_version, :current_publication_revision,
    :current_stewardship_epoch, pg_catalog.decode(:'current_evidence_digest', 'hex'),
    :current_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-noop', 'work-noop'
)
\gset noop_
SELECT :'noop_outcome' = 'NO_CHANGE'
   AND :'noop_reason_code' = 'CONTROL_UNCHANGED'
   AND :noop_action_revision = :current_action_revision
   AND (SELECT assigned_queue::text = :'current_policy_queue'
               AND action_revision = :current_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :current_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'NO_CHANGE'
               AND reason_code = 'CONTROL_UNCHANGED'
               AND action_revision = :current_action_revision
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('d', 64), 'hex'))
   AS binding_intent_noop_ok
\gset
\if :binding_intent_noop_ok
\else
\quit 1
\endif
SELECT outcome, reason_code, receipt_id IS NULL AS no_new_receipt, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('c', 64), 'hex'),
    :current_case_key, 'ASSIGN_QUEUE', pg_catalog.jsonb_build_object('queue', 'not-bound'),
    :current_review_version, :current_definition_version, :current_publication_revision,
    :current_stewardship_epoch, pg_catalog.decode(:'current_evidence_digest', 'hex'),
    :current_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-conflict', 'work-conflict'
)
\gset conflict_
SELECT :'conflict_outcome' = 'IDEMPOTENCY_CONFLICT'
   AND :'conflict_reason_code' = 'REQUEST_KEY_BODY_MISMATCH'
   AND :'conflict_no_new_receipt'::boolean
   AND :conflict_action_revision = :current_action_revision
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'APPLIED_CONTROL'
               AND reason_code = 'CONTROL_APPLIED'
               AND action_revision = :applied_action_revision
               AND control->>'assigned_queue' = :'policy_queue')
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('c', 64), 'hex'))
   AND (SELECT assigned_queue::text = :'policy_queue'
               AND action_revision = :applied_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :current_case_key)
   AS binding_intent_conflict_ok
\gset
\if :binding_intent_conflict_ok
\else
\quit 1
\endif
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('e', 64), 'hex'),
    :current_case_key, 'ASSIGN_QUEUE', pg_catalog.jsonb_build_object('queue', 'not-bound'),
    :current_review_version, :current_definition_version, :current_publication_revision,
    :current_stewardship_epoch, pg_catalog.decode(:'current_evidence_digest', 'hex'),
    :current_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-denied', 'work-denied'
)
\gset denied_queue_
SELECT :'denied_queue_outcome' = 'ACTION_DENIED'
   AND :'denied_queue_reason_code' = 'QUEUE_NOT_ALLOWED'
   AND :denied_queue_action_revision = :current_action_revision
   AND (SELECT assigned_queue::text = :'policy_queue'
               AND action_revision = :applied_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :current_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'ACTION_DENIED'
               AND reason_code = 'QUEUE_NOT_ALLOWED'
               AND action_revision = :current_action_revision
               AND control->>'assigned_queue' = :'policy_queue'
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('e', 64), 'hex'))
   AS binding_intent_denied_queue_ok
\gset
\if :binding_intent_denied_queue_ok
\else
\quit 1
\endif
SELECT case_key, review_version, definition_version, publication_revision,
       stewardship_epoch, pg_catalog.encode(evidence_basis_digest, 'hex') AS evidence_digest,
       action_revision, opened_at::text AS opened_at, escalation_level,
       COALESCE(due_at::text, '') AS before_due
FROM mdm_steward.policy_cases_v1
WHERE entity_name = 'policy_qualification'
  AND status = 'open'
  AND opened_at <= pg_catalog.statement_timestamp()
  AND opened_at IS NOT NULL
  AND NOT pending_stewardship
  AND 'SET_DUE_AT' = ANY(permitted_actions)
  AND 'ESCALATE' = ANY(permitted_actions)
  AND escalation_level < 3
  AND due_at IS DISTINCT FROM opened_at
ORDER BY case_key
LIMIT 1
\gset deadline_
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('1', 64), 'hex'),
    :deadline_case_key, 'SET_DUE_AT',
    pg_catalog.jsonb_build_object('due_at', :'deadline_opened_at'::timestamptz - INTERVAL '1 second'),
    :deadline_review_version, :deadline_definition_version, :deadline_publication_revision,
    :deadline_stewardship_epoch, pg_catalog.decode(:'deadline_evidence_digest', 'hex'),
    :deadline_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-due-lower-bound', 'work-due-lower-bound'
)
\gset due_lower_
SELECT :'due_lower_outcome' = 'LIMIT_EXCEEDED'
   AND :'due_lower_reason_code' = 'DUE_AT_OUT_OF_BOUNDS'
   AND :due_lower_action_revision = :deadline_action_revision
   AND (SELECT due_at IS NOT DISTINCT FROM NULLIF(:'deadline_before_due', '')::timestamptz
               AND action_revision = :deadline_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :deadline_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'LIMIT_EXCEEDED'
               AND reason_code = 'DUE_AT_OUT_OF_BOUNDS'
               AND action_revision = :deadline_action_revision
               AND control->>'due_at' IS NOT DISTINCT FROM NULLIF(:'deadline_before_due', '')
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('1', 64), 'hex'))
   AS binding_intent_due_lower_bound_ok
\gset
\if :binding_intent_due_lower_bound_ok
\else
\quit 1
\endif
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('4', 64), 'hex'),
    :deadline_case_key, 'SET_DUE_AT',
    pg_catalog.jsonb_build_object('due_at', :'deadline_opened_at'::timestamptz + INTERVAL '31 days'),
    :deadline_review_version, :deadline_definition_version, :deadline_publication_revision,
    :deadline_stewardship_epoch, pg_catalog.decode(:'deadline_evidence_digest', 'hex'),
    :deadline_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-due-upper-bound', 'work-due-upper-bound'
)
\gset due_upper_
SELECT :'due_upper_outcome' = 'LIMIT_EXCEEDED'
   AND :'due_upper_reason_code' = 'DUE_AT_OUT_OF_BOUNDS'
   AND :due_upper_action_revision = :deadline_action_revision
   AND (SELECT due_at IS NOT DISTINCT FROM NULLIF(:'deadline_before_due', '')::timestamptz
               AND action_revision = :deadline_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :deadline_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'LIMIT_EXCEEDED'
               AND reason_code = 'DUE_AT_OUT_OF_BOUNDS'
               AND action_revision = :deadline_action_revision
               AND control->>'due_at' IS NOT DISTINCT FROM NULLIF(:'deadline_before_due', '')
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('4', 64), 'hex'))
   AS binding_intent_due_upper_bound_ok
\gset
\if :binding_intent_due_upper_bound_ok
\else
\quit 1
\endif
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('2', 64), 'hex'),
    :deadline_case_key, 'SET_DUE_AT',
    pg_catalog.jsonb_build_object('due_at', :'deadline_opened_at'::timestamptz),
    :deadline_review_version, :deadline_definition_version, :deadline_publication_revision,
    :deadline_stewardship_epoch, pg_catalog.decode(:'deadline_evidence_digest', 'hex'),
    :deadline_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-due-valid', 'work-due-valid'
)
\gset due_valid_
SELECT :'due_valid_outcome' = 'APPLIED_CONTROL'
   AND :'due_valid_reason_code' = 'CONTROL_APPLIED'
   AND :due_valid_action_revision = :deadline_action_revision + 1
   AND (SELECT due_at = :'deadline_opened_at'::timestamptz
               AND action_revision = :due_valid_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :deadline_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'APPLIED_CONTROL'
               AND reason_code = 'CONTROL_APPLIED'
               AND action_revision = :due_valid_action_revision
               AND (control->>'due_at')::timestamptz = :'deadline_opened_at'::timestamptz
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('2', 64), 'hex'))
   AS binding_intent_due_valid_ok
\gset
\if :binding_intent_due_valid_ok
\else
\quit 1
\endif
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('3', 64), 'hex'),
    :deadline_case_key, 'ESCALATE', pg_catalog.jsonb_build_object('level', :deadline_escalation_level + 1),
    :deadline_review_version, :deadline_definition_version, :deadline_publication_revision,
    :deadline_stewardship_epoch, pg_catalog.decode(:'deadline_evidence_digest', 'hex'),
    :due_valid_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-escalate-due', 'work-escalate-due'
)
\gset escalate_due_
SELECT :'escalate_due_outcome' = 'APPLIED_CONTROL'
   AND :'escalate_due_reason_code' = 'CONTROL_APPLIED'
   AND :escalate_due_action_revision = :due_valid_action_revision + 1
   AND (SELECT escalation_level = :deadline_escalation_level + 1
               AND due_at = :'deadline_opened_at'::timestamptz
               AND action_revision = :escalate_due_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :deadline_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'APPLIED_CONTROL'
               AND reason_code = 'CONTROL_APPLIED'
               AND action_revision = :escalate_due_action_revision
               AND control->>'escalation_level' = (:deadline_escalation_level + 1)::text
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('3', 64), 'hex'))
   AS binding_intent_escalate_due_ok
\gset
\if :binding_intent_escalate_due_ok
\else
\quit 1
\endif
SELECT assigned_queue::text AS queue, action_revision
FROM mdm_steward.policy_cases_v1 WHERE case_key = :current_case_key
\gset malformed_before_
DO $$
DECLARE rejected boolean := false;
BEGIN
    BEGIN
        PERFORM * FROM mdm_steward.submit_policy_intent(
            '00000000-0000-0000-0000-000000000001'::uuid,
            pg_catalog.decode(pg_catalog.repeat('f', 64), 'hex'), 1,
            'ASSIGN_QUEUE', '{"queue":"priority","extra":true}'::jsonb,
            1, 1, 0, 0, pg_catalog.decode(pg_catalog.repeat('f', 64), 'hex'),
            1, pg_catalog.decode(pg_catalog.repeat('f', 64), 'hex'),
            'policy-1', 'eval-malformed', 'work-malformed'
        );
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'ASSIGN_QUEUE arguments must contain only queue') = 0
           OR pg_catalog.strpos(SQLERRM, 'MDM_POLICY_INTENT') = 0 THEN
            RAISE;
        END IF;
        rejected := true;
    END;
    IF NOT rejected OR EXISTS (
        SELECT 1 FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = '00000000-0000-0000-0000-000000000001'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('f', 64), 'hex')
    ) THEN
        RAISE EXCEPTION 'malformed intent created a receipt or was accepted';
    END IF;
END
$$;
SELECT assigned_queue::text = :'malformed_before_queue'
   AND action_revision = :malformed_before_action_revision
   AS malformed_intent_control_unchanged
FROM mdm_steward.policy_cases_v1 WHERE case_key = :current_case_key
\gset
\if :malformed_intent_control_unchanged
\else
\quit 1
\endif
SELECT case_key, review_version, definition_version, publication_revision,
       stewardship_epoch, pg_catalog.encode(evidence_basis_digest, 'hex') AS evidence_digest,
       action_revision,
       CASE WHEN assigned_queue::text = 'priority' THEN 'urgent' ELSE 'priority' END AS queue_after_rollback
FROM mdm_steward.policy_cases_v1
WHERE case_key = :current_case_key
\gset atomic_
BEGIN;
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('5', 64), 'hex'),
    :atomic_case_key, 'ASSIGN_QUEUE', pg_catalog.jsonb_build_object('queue', :'atomic_queue_after_rollback'),
    :atomic_review_version, :atomic_definition_version, :atomic_publication_revision,
    :atomic_stewardship_epoch, pg_catalog.decode(:'atomic_evidence_digest', 'hex'),
    :atomic_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-atomic', 'work-atomic'
)
\gset atomic_first_
SELECT :'atomic_first_outcome' = 'APPLIED_CONTROL'
   AND :'atomic_first_reason_code' = 'CONTROL_APPLIED'
   AND :atomic_first_action_revision = :atomic_action_revision + 1
   AND (SELECT assigned_queue::text = :'atomic_queue_after_rollback'
               AND action_revision = :atomic_first_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :atomic_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'APPLIED_CONTROL'
               AND reason_code = 'CONTROL_APPLIED'
               AND action_revision = :atomic_first_action_revision
               AND control->>'assigned_queue' = :'atomic_queue_after_rollback')
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('5', 64), 'hex'))
   AS binding_intent_atomic_first_ok
\gset
\if :binding_intent_atomic_first_ok
\else
\quit 1
\endif
ROLLBACK;
SELECT count(*) = 0 AS binding_intent_atomic_rollback_ok
FROM mdm_steward.policy_receipts_v1
WHERE binding_id = :'second_binding_id'::uuid
  AND request_key = pg_catalog.decode(pg_catalog.repeat('5', 64), 'hex')
\gset
\if :binding_intent_atomic_rollback_ok
\else
\quit 1
\endif
SELECT assigned_queue::text <> :'atomic_queue_after_rollback'
   AND action_revision = :atomic_action_revision AS binding_intent_atomic_control_rollback_ok
FROM mdm_steward.policy_cases_v1 WHERE case_key = :atomic_case_key
\gset
\if :binding_intent_atomic_control_rollback_ok
\else
\quit 1
\endif
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('5', 64), 'hex'),
    :atomic_case_key, 'ASSIGN_QUEUE', pg_catalog.jsonb_build_object('queue', :'atomic_queue_after_rollback'),
    :atomic_review_version, :atomic_definition_version, :atomic_publication_revision,
    :atomic_stewardship_epoch, pg_catalog.decode(:'atomic_evidence_digest', 'hex'),
    :atomic_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-atomic', 'work-atomic'
)
\gset atomic_retry_
SELECT :'atomic_retry_outcome' = 'APPLIED_CONTROL'
   AND :'atomic_retry_reason_code' = 'CONTROL_APPLIED'
   AND :atomic_retry_action_revision = :atomic_action_revision + 1
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'APPLIED_CONTROL'
               AND reason_code = 'CONTROL_APPLIED'
               AND action_revision = :atomic_retry_action_revision
               AND control->>'assigned_queue' = :'atomic_queue_after_rollback'
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('5', 64), 'hex'))
   AS binding_intent_atomic_retry_ok
\gset
\if :binding_intent_atomic_retry_ok
\else
\quit 1
\endif
RESET ROLE;
\connect foundation mdm_legacy_login
SET ROLE :"policy_execution_role";
DO $$
DECLARE rejected boolean := false;
BEGIN
    BEGIN
        PERFORM * FROM mdm_admin.create_policy_binding(
            'policy_qualification', 'mdm_policy_worker',
            pg_catalog.decode(pg_catalog.repeat('d', 64), 'hex'),
            ARRAY['ESCALATE', 'ASSIGN_QUEUE'], ARRAY['priority']::text[], NULL, 3
        );
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'allowed_actions must be sorted, distinct') = 0
           OR pg_catalog.strpos(SQLERRM, 'MDM_POLICY_BINDING') = 0 THEN RAISE; END IF;
        rejected := true;
    END;
    IF NOT rejected THEN RAISE EXCEPTION 'noncanonical binding actions were accepted'; END IF;
END
$$;
CREATE FUNCTION pg_temp.e2e_reject_stale_binding_replace(p_binding_id uuid)
RETURNS boolean LANGUAGE plpgsql AS $$
BEGIN
    PERFORM * FROM mdm_admin.replace_policy_binding(
        p_binding_id, 999, pg_catalog.decode(pg_catalog.repeat('d', 64), 'hex'),
        ARRAY['ASSIGN_QUEUE', 'ESCALATE', 'SET_DUE_AT'],
        ARRAY['priority', 'urgent']::text[], INTERVAL '30 days', 3
    );
    RETURN false;
EXCEPTION WHEN OTHERS THEN
    IF pg_catalog.strpos(SQLERRM, 'binding is replaced or expected version is stale') = 0
       OR pg_catalog.strpos(SQLERRM, 'MDM_POLICY_BINDING') = 0 THEN RAISE; END IF;
    RETURN true;
END
$$;
SELECT pg_temp.e2e_reject_stale_binding_replace(:'second_binding_id'::uuid) AS stale_binding_replace_rejected
\gset
\if :stale_binding_replace_rejected
\else
\quit 1
\endif
RESET ROLE;
\connect foundation postgres
SELECT count(*) = 2
   AND count(*) FILTER (WHERE binding_id = :'first_binding_id'::uuid
                        AND replaced_by = :'second_binding_id'::uuid) = 1
   AND count(*) FILTER (WHERE binding_id = :'second_binding_id'::uuid
                        AND replaced_by IS NULL AND binding_version = 1) = 1
   AS binding_invalid_cases_state_ok
FROM mdm_steward.policy_bindings_v1 WHERE automation_role_name = 'mdm_policy_worker'
\gset
\if :binding_invalid_cases_state_ok
\else
\quit 1
\endif
\connect foundation postgres
SELECT case_key, assigned_queue::text AS queue, COALESCE(due_at::text, '') AS due_at,
       escalation_level AS level, action_revision AS revision,
       manual_assignment_protected AS protected
FROM mdm_steward.policy_cases_v1
WHERE case_key = :current_case_key
\gset human_
\connect foundation mdm_legacy_login
SET ROLE :"policy_execution_role";
CREATE TEMP TABLE e2e_policy_human_before AS
SELECT :human_case_key::bigint AS case_key, :'human_queue'::name AS assigned_queue,
       NULLIF(:'human_due_at', '')::timestamptz AS due_at,
       :human_level::integer AS escalation_level, :human_revision::bigint AS action_revision,
       :human_protected::boolean AS manual_assignment_protected;
SELECT action_revision
FROM mdm_admin.set_case_controls(
    :human_case_key, :'human_queue'::name, NULLIF(:'human_due_at', '')::timestamptz,
    :human_level, :'human_protected'::boolean, :human_revision, 'e2e unchanged human controls'
)
\gset human_noop_
SELECT :human_noop_action_revision = :human_revision
   AND (SELECT action_revision = :human_revision
               AND manual_assignment_protected = :'human_protected'::boolean
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key)
   AS human_control_noop_ok
\gset
\if :human_control_noop_ok
\else
\quit 1
\endif
SELECT action_revision
FROM mdm_admin.set_case_controls(
    :human_case_key, :'human_queue'::name, NULLIF(:'human_due_at', '')::timestamptz,
    :human_level, true, :human_revision, 'e2e manual assignment protection'
)
\gset human_changed_
SELECT :human_changed_action_revision = :human_revision + 1
   AND (SELECT manual_assignment_protected AND action_revision = :human_changed_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key)
   AS human_control_changed_ok
\gset
\if :human_control_changed_ok
\else
\quit 1
\endif
DO $$
DECLARE rejected boolean := false;
BEGIN
    BEGIN
        PERFORM * FROM mdm_admin.set_case_controls(
            (SELECT case_key FROM e2e_policy_human_before),
            (SELECT assigned_queue FROM e2e_policy_human_before),
            (SELECT due_at FROM e2e_policy_human_before),
            (SELECT escalation_level FROM e2e_policy_human_before),
            (SELECT manual_assignment_protected FROM e2e_policy_human_before),
            (SELECT action_revision FROM e2e_policy_human_before), 'e2e stale human controls'
        );
    EXCEPTION WHEN OTHERS THEN
        IF pg_catalog.strpos(SQLERRM, 'action revision changed concurrently') = 0
           OR pg_catalog.strpos(SQLERRM, 'MDM_POLICY_CONTROL') = 0 THEN
            RAISE;
        END IF;
        rejected := true;
    END;
    IF NOT rejected THEN RAISE EXCEPTION 'stale human controls were accepted'; END IF;
END
$$;
DROP TABLE e2e_policy_human_before;
RESET ROLE;
\connect foundation mdm_legacy_login
SET ROLE mdm_policy_worker;
SELECT review_version, definition_version, publication_revision, stewardship_epoch,
       pg_catalog.encode(evidence_basis_digest, 'hex') AS evidence_digest, action_revision,
       assigned_queue::text AS queue
FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key
\gset protected_
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('6', 64), 'hex'),
    :human_case_key, 'ASSIGN_QUEUE',
    pg_catalog.jsonb_build_object('queue', CASE WHEN :'protected_queue' = 'priority' THEN 'urgent' ELSE 'priority' END),
    :protected_review_version, :protected_definition_version, :protected_publication_revision,
    :protected_stewardship_epoch, pg_catalog.decode(:'protected_evidence_digest', 'hex'),
    :protected_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-protected', 'work-protected'
)
\gset protected_intent_
SELECT :'protected_intent_outcome' = 'MANUAL_PROTECTION'
   AND :'protected_intent_reason_code' = 'MANUAL_ASSIGNMENT_PROTECTED'
   AND :protected_intent_action_revision = :protected_action_revision
   AND (SELECT manual_assignment_protected AND action_revision = :protected_action_revision
               AND assigned_queue::text = :'protected_queue'
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'MANUAL_PROTECTION'
               AND reason_code = 'MANUAL_ASSIGNMENT_PROTECTED'
               AND action_revision = :protected_action_revision
               AND control->>'manual_assignment_protected' = 'true'
               AND control->>'assigned_queue' = :'protected_queue'
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('6', 64), 'hex'))
   AS binding_intent_manual_protection_ok
\gset
\if :binding_intent_manual_protection_ok
\else
\quit 1
\endif
RESET ROLE;
\connect foundation mdm_legacy_login
SET ROLE :"policy_execution_role";
SELECT assigned_queue::text AS escalation_queue, action_revision AS escalation_revision
FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key
\gset early_seed_
SELECT action_revision
FROM mdm_admin.set_case_controls(
    :human_case_key, :'early_seed_escalation_queue'::name,
    pg_catalog.statement_timestamp() + INTERVAL '1 day', 0, true,
    :early_seed_escalation_revision, 'e2e future due for early escalation'
)
\gset early_seeded_
SELECT :early_seeded_action_revision = :early_seed_escalation_revision + 1 AS early_seed_ok
\gset
\if :early_seed_ok
\else
\quit 1
\endif
RESET ROLE;
\connect foundation mdm_legacy_login
SET ROLE mdm_policy_worker;
SELECT review_version, definition_version, publication_revision, stewardship_epoch,
       pg_catalog.encode(evidence_basis_digest, 'hex') AS evidence_digest,
       action_revision, escalation_level
FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key
\gset early_
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('7', 64), 'hex'),
    :human_case_key, 'ESCALATE', pg_catalog.jsonb_build_object('level', :early_escalation_level + 1),
    :early_review_version, :early_definition_version, :early_publication_revision,
    :early_stewardship_epoch, pg_catalog.decode(:'early_evidence_digest', 'hex'),
    :early_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-early-escalation', 'work-early-escalation'
)
\gset early_intent_
SELECT :'early_intent_outcome' = 'ACTION_DENIED'
   AND :'early_intent_reason_code' = 'ESCALATION_NOT_DUE'
   AND :early_intent_action_revision = :early_action_revision
   AND (SELECT due_at > pg_catalog.statement_timestamp()
               AND escalation_level = :early_escalation_level
               AND action_revision = :early_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'ACTION_DENIED'
               AND reason_code = 'ESCALATION_NOT_DUE'
               AND action_revision = :early_action_revision
               AND (control->>'due_at')::timestamptz > pg_catalog.statement_timestamp()
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('7', 64), 'hex'))
   AS binding_intent_escalate_early_ok
\gset
\if :binding_intent_escalate_early_ok
\else
\quit 1
\endif
RESET ROLE;
\connect foundation mdm_legacy_login
SET ROLE :"policy_execution_role";
SELECT assigned_queue::text AS escalation_queue, action_revision AS escalation_revision
FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key
\gset limit_seed_
SELECT action_revision
FROM mdm_admin.set_case_controls(
    :human_case_key, :'limit_seed_escalation_queue'::name,
    pg_catalog.statement_timestamp() - INTERVAL '1 day', 2, true,
    :limit_seed_escalation_revision, 'e2e due escalation at limit'
)
\gset limit_seeded_
SELECT :limit_seeded_action_revision = :limit_seed_escalation_revision + 1 AS escalation_limit_seed_ok
\gset
\if :escalation_limit_seed_ok
\else
\quit 1
\endif
RESET ROLE;
\connect foundation mdm_legacy_login
SET ROLE mdm_policy_worker;
SELECT review_version, definition_version, publication_revision, stewardship_epoch,
       pg_catalog.encode(evidence_basis_digest, 'hex') AS evidence_digest,
       action_revision, escalation_level
FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key
\gset at_limit_
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('8', 64), 'hex'),
    :human_case_key, 'ESCALATE', pg_catalog.jsonb_build_object('level', :at_limit_escalation_level + 2),
    :at_limit_review_version, :at_limit_definition_version, :at_limit_publication_revision,
    :at_limit_stewardship_epoch, pg_catalog.decode(:'at_limit_evidence_digest', 'hex'),
    :at_limit_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-skipped-escalation', 'work-skipped-escalation'
)
\gset skipped_level_
SELECT :'skipped_level_outcome' = 'STALE_CASE'
   AND :'skipped_level_reason_code' = 'ESCALATION_NOT_NEXT'
   AND :skipped_level_action_revision = :at_limit_action_revision
   AND (SELECT escalation_level = 2 AND action_revision = :at_limit_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'STALE_CASE'
               AND reason_code = 'ESCALATION_NOT_NEXT'
               AND action_revision = :at_limit_action_revision
               AND control->>'escalation_level' = '2'
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('8', 64), 'hex'))
   AS binding_intent_escalate_not_next_ok
\gset
\if :binding_intent_escalate_not_next_ok
\else
\quit 1
\endif
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('9', 64), 'hex'),
    :human_case_key, 'ESCALATE', pg_catalog.jsonb_build_object('level', 3),
    :at_limit_review_version, :at_limit_definition_version, :at_limit_publication_revision,
    :at_limit_stewardship_epoch, pg_catalog.decode(:'at_limit_evidence_digest', 'hex'),
    :at_limit_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-escalation-limit', 'work-escalation-limit'
)
\gset escalate_to_limit_
SELECT :'escalate_to_limit_outcome' = 'APPLIED_CONTROL'
   AND :'escalate_to_limit_reason_code' = 'CONTROL_APPLIED'
   AND :escalate_to_limit_action_revision = :at_limit_action_revision + 1
   AND (SELECT escalation_level = 3 AND action_revision = :escalate_to_limit_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'APPLIED_CONTROL'
               AND reason_code = 'CONTROL_APPLIED'
               AND action_revision = :escalate_to_limit_action_revision
               AND control->>'escalation_level' = '3'
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('9', 64), 'hex'))
   AS binding_intent_escalate_to_max_ok
\gset
\if :binding_intent_escalate_to_max_ok
\else
\quit 1
\endif
SELECT receipt_id, outcome, reason_code, action_revision
FROM mdm_steward.submit_policy_intent(
    :'second_binding_id'::uuid, pg_catalog.decode(pg_catalog.repeat('a', 64), 'hex'),
    :human_case_key, 'ESCALATE', pg_catalog.jsonb_build_object('level', 4),
    :at_limit_review_version, :at_limit_definition_version, :at_limit_publication_revision,
    :at_limit_stewardship_epoch, pg_catalog.decode(:'at_limit_evidence_digest', 'hex'),
    :escalate_to_limit_action_revision, pg_catalog.decode(pg_catalog.repeat('b', 64), 'hex'),
    'policy-1', 'eval-over-limit', 'work-over-limit'
)
\gset over_limit_
SELECT :'over_limit_outcome' = 'LIMIT_EXCEEDED'
   AND :'over_limit_reason_code' = 'ESCALATION_LIMIT'
   AND :over_limit_action_revision = :escalate_to_limit_action_revision
   AND (SELECT escalation_level = 3 AND action_revision = :escalate_to_limit_action_revision
        FROM mdm_steward.policy_cases_v1 WHERE case_key = :human_case_key)
   AND (SELECT count(*) = 1 AND bool_and(outcome = 'LIMIT_EXCEEDED'
               AND reason_code = 'ESCALATION_LIMIT'
               AND action_revision = :escalate_to_limit_action_revision
               AND control->>'escalation_level' = '3'
               AND resulting_publication_revision IS NULL)
        FROM mdm_steward.policy_receipts_v1
        WHERE binding_id = :'second_binding_id'::uuid
          AND request_key = pg_catalog.decode(pg_catalog.repeat('a', 64), 'hex'))
   AS binding_intent_escalate_over_limit_ok
\gset
\if :binding_intent_escalate_over_limit_ok
\else
\quit 1
\endif
RESET ROLE;
\connect foundation mdm_legacy_login
SET ROLE :"policy_execution_role";
SELECT mdm_admin.set_policy_binding_state(:'second_binding_id'::uuid, :active_runtime_version, 'paused', 'paused') AS paused_runtime_version
\gset
RESET ROLE;
\connect foundation postgres
DO $$
BEGIN
    IF (SELECT runtime_version FROM mdm_internal.policy_binding_runtime
        WHERE binding_id = (SELECT binding_id FROM mdm_steward.policy_bindings_v1 WHERE automation_role_name = 'mdm_policy_worker' AND replaced_by IS NULL)) <> 3
       OR (SELECT state FROM mdm_internal.policy_binding_runtime
           WHERE binding_id = (SELECT binding_id FROM mdm_steward.policy_bindings_v1 WHERE automation_role_name = 'mdm_policy_worker' AND replaced_by IS NULL)) <> 'paused'
       OR EXISTS (SELECT 1 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = 'mdm_steward' AND p.proname IN ('register_policy_binding', 'pause_policy_binding', 'replace_policy_binding')) THEN
        RAISE EXCEPTION 'binding activation, pause, or retired API state is invalid';
    END IF;
END
$$;
\echo PASS: policy publication identity, resolution, recurrence, unknown opening, and backfill assertions
\echo PASS: policy binding lifecycle, intent application, retry, no-op, denial, deadline, escalation and rollback controls, runtime reconciliation, and retired API assertions
\echo PASS: policy reader exact-row access and private-history denial
