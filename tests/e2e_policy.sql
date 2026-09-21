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
