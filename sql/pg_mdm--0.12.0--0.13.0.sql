CREATE TABLE mdm_internal.graph_delta_consumers (
    graph_binding_id uuid NOT NULL,
    logical_id text NOT NULL,
    consumer_id uuid NOT NULL UNIQUE,
    delta_relation_name text NOT NULL,
    row_identity_version smallint NOT NULL CHECK (row_identity_version > 0),
    PRIMARY KEY (graph_binding_id, logical_id),
    FOREIGN KEY (graph_binding_id, logical_id)
        REFERENCES mdm_internal.graph_members(graph_binding_id, logical_id)
);

CREATE SEQUENCE mdm_internal.policy_case_key_seq AS bigint START WITH 1;

CREATE TABLE mdm_steward.policy_cases_v1 (
    case_key bigint PRIMARY KEY CHECK (case_key > 0),
    review_id uuid NOT NULL UNIQUE REFERENCES mdm_internal.reviews(review_id),
    entity_name pg_catalog.name NOT NULL,
    issue_key bytea NOT NULL CHECK (octet_length(issue_key) = 32),
    occurrence integer NOT NULL CHECK (occurrence > 0),
    status text NOT NULL CHECK (status IN ('open', 'resolved')),
    severity text NOT NULL,
    reason_code text NOT NULL,
    approved_metadata jsonb NOT NULL,
    permitted_actions text[] NOT NULL,
    assigned_queue pg_catalog.name,
    due_at timestamptz,
    escalation_level integer NOT NULL CHECK (escalation_level >= 0),
    manual_assignment_protected boolean NOT NULL,
    opened_at timestamptz,
    opened_at_source text NOT NULL CHECK (opened_at_source IN ('publication', 'administrator', 'unknown')),
    resolved_at timestamptz,
    review_version bigint NOT NULL CHECK (review_version > 0),
    definition_version bigint NOT NULL CHECK (definition_version > 0),
    publication_revision bigint NOT NULL CHECK (publication_revision >= 0),
    stewardship_epoch bigint NOT NULL CHECK (stewardship_epoch >= 0),
    evidence_basis_digest bytea NOT NULL CHECK (octet_length(evidence_basis_digest) = 32),
    action_revision bigint NOT NULL CHECK (action_revision > 0),
    pending_stewardship boolean NOT NULL,
    last_observed_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    UNIQUE (entity_name, issue_key, occurrence),
    CHECK (
        permitted_actions = '{}'::text[]
        OR permitted_actions = ARRAY['ASSIGN_QUEUE']::text[]
        OR permitted_actions = ARRAY['ASSIGN_QUEUE', 'ESCALATE']::text[]
        OR permitted_actions = ARRAY['ASSIGN_QUEUE', 'ESCALATE', 'SET_DUE_AT']::text[]
        OR permitted_actions = ARRAY['ASSIGN_QUEUE', 'SET_DUE_AT']::text[]
        OR permitted_actions = ARRAY['ESCALATE']::text[]
        OR permitted_actions = ARRAY['ESCALATE', 'SET_DUE_AT']::text[]
        OR permitted_actions = ARRAY['SET_DUE_AT']::text[]
    ),
    CHECK ((status = 'resolved') = (cardinality(permitted_actions) = 0))
);

SELECT pg_catalog.pg_extension_config_dump('mdm_internal.policy_case_key_seq'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_steward.policy_cases_v1'::pg_catalog.regclass, '');
REVOKE ALL ON SEQUENCE mdm_internal.policy_case_key_seq FROM PUBLIC;
REVOKE ALL ON TABLE mdm_steward.policy_cases_v1 FROM PUBLIC;

INSERT INTO mdm_steward.policy_cases_v1 (
    case_key, review_id, entity_name, issue_key, occurrence, status, severity,
    reason_code, approved_metadata, permitted_actions, assigned_queue, due_at,
    escalation_level, manual_assignment_protected, opened_at, opened_at_source,
    resolved_at, review_version, definition_version, publication_revision,
    stewardship_epoch, evidence_basis_digest, action_revision, pending_stewardship
)
SELECT
    pg_catalog.nextval('mdm_internal.policy_case_key_seq'),
    review_id,
    entity_name,
    issue_key,
    occurrence,
    status,
    severity,
    reason_code,
    '{}'::jsonb,
    CASE
        WHEN status = 'resolved' THEN '{}'::text[]
        WHEN opened_at IS NULL THEN ARRAY['ASSIGN_QUEUE', 'ESCALATE']::text[]
        ELSE ARRAY['ASSIGN_QUEUE', 'ESCALATE', 'SET_DUE_AT']::text[]
    END,
    NULL,
    NULL,
    0,
    false,
    opened_at,
    CASE WHEN opened_at IS NULL THEN 'unknown' ELSE 'publication' END,
    resolved_at,
    concurrency_version,
    definition_version,
    opened_revision,
    stewardship_epoch,
    mdm_internal.evidence_digest(
        review_id::text || ':' || issue_key::text || ':' || occurrence::text
    ),
    1,
    current_decision_epoch <> stewardship_epoch
FROM (
    SELECT
        r.review_id,
        e.entity_name,
        r.issue_key,
        r.occurrence,
        r.status,
        r.severity,
        r.reason_code,
        r.concurrency_version,
        r.opened_revision,
        r.resolved_revision,
        COALESCE(opened.publication_definition_version, e.desired_version) AS definition_version,
        COALESCE(opened.publication_decision_epoch, 0) AS stewardship_epoch,
        e.decision_epoch AS current_decision_epoch,
        opened.published_at AS opened_at,
        resolved.published_at AS resolved_at
    FROM mdm_internal.reviews r
    JOIN mdm_internal.entities e ON e.entity_id = r.entity_id
    LEFT JOIN LATERAL (
        SELECT p.published_at, p.definition_version AS publication_definition_version,
               p.decision_epoch AS publication_decision_epoch
        FROM mdm_internal.publications p
        WHERE p.entity_id = r.entity_id AND p.publication_revision = r.opened_revision
    ) opened ON true
    LEFT JOIN LATERAL (
        SELECT p.published_at
        FROM mdm_internal.publications p
        WHERE p.entity_id = r.entity_id AND p.publication_revision = r.resolved_revision
    ) resolved ON true
    ORDER BY e.entity_name, r.issue_key, r.occurrence
) ordered;

SELECT pg_catalog.setval(
    'mdm_internal.policy_case_key_seq',
    COALESCE((SELECT max(case_key) FROM mdm_steward.policy_cases_v1), 1),
    EXISTS (SELECT 1 FROM mdm_steward.policy_cases_v1)
);

SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.graph_bindings'::pg_catalog.regclass,
    ''
);
SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.graph_members'::pg_catalog.regclass,
    ''
);
SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.graph_delta_consumers'::pg_catalog.regclass,
    ''
);

REVOKE ALL ON TABLE mdm_internal.graph_delta_consumers FROM PUBLIC;

CREATE FUNCTION mdm_internal.persist_recompile(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_recompile_wrapper';
CREATE FUNCTION mdm_admin.recompile(entity_name text) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'recompile_wrapper';
CREATE FUNCTION mdm_admin.backfill_policy_case_opened_at(case_key bigint, opened_at timestamptz, reason text) RETURNS TABLE (operation_id uuid, action_revision bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'backfill_policy_case_opened_at_wrapper';
CREATE FUNCTION mdm_internal.persist_backfill_policy_case_opened_at(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_backfill_policy_case_opened_at_wrapper';

REVOKE ALL ON FUNCTION mdm_internal.persist_recompile(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.recompile(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.backfill_policy_case_opened_at(bigint, timestamptz, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_backfill_policy_case_opened_at(internal) FROM PUBLIC;
