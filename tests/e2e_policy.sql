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
\echo PASS: policy publication identity, resolution, recurrence, unknown opening, and backfill assertions
\echo PASS: policy reader exact-row access and private-history denial
