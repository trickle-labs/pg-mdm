CREATE TABLE mdm_steward.policy_bindings_v1 (
    binding_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    automation_role_name pg_catalog.name NOT NULL,
    policy_digest bytea NOT NULL CHECK (octet_length(policy_digest) = 32),
    allowed_actions text[] NOT NULL,
    allowed_queues pg_catalog.name[] NOT NULL DEFAULT ARRAY[]::pg_catalog.name[],
    max_due_interval interval,
    max_escalation_level integer NOT NULL DEFAULT 0 CHECK (max_escalation_level >= 0),
    binding_version bigint NOT NULL DEFAULT 1 CHECK (binding_version > 0),
    replaced_by uuid,
    replaced_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    created_by_name text NOT NULL,
    created_as_role_name text NOT NULL,
    CHECK (allowed_actions <@ ARRAY['ASSIGN_QUEUE', 'ESCALATE', 'SET_DUE_AT']::text[]),
    CHECK (cardinality(allowed_actions) BETWEEN 1 AND 3),
    CHECK ('ASSIGN_QUEUE' <> ALL (allowed_actions) OR cardinality(allowed_queues) > 0),
    CHECK ('SET_DUE_AT' <> ALL (allowed_actions) OR (max_due_interval IS NOT NULL AND max_due_interval > interval '0')),
    CHECK ('ESCALATE' <> ALL (allowed_actions) OR max_escalation_level > 0),
    CHECK ((replaced_by IS NULL) = (replaced_at IS NULL))
);
CREATE UNIQUE INDEX policy_bindings_one_current_role
    ON mdm_steward.policy_bindings_v1 (entity_id, automation_role_name) WHERE replaced_by IS NULL;

CREATE TABLE mdm_internal.policy_binding_runtime (
    binding_id uuid PRIMARY KEY REFERENCES mdm_steward.policy_bindings_v1(binding_id),
    database_oid oid NOT NULL,
    automation_role_oid oid NOT NULL,
    state text NOT NULL CHECK (state IN ('active', 'paused')),
    runtime_version bigint NOT NULL CHECK (runtime_version > 0),
    changed_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp()
);

CREATE TABLE mdm_steward.policy_receipts_v1 (
    receipt_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    binding_id uuid NOT NULL REFERENCES mdm_steward.policy_bindings_v1(binding_id),
    request_key bytea NOT NULL CHECK (octet_length(request_key) = 32),
    request_digest bytea NOT NULL CHECK (octet_length(request_digest) = 32),
    request_body jsonb NOT NULL,
    actor pg_catalog.name NOT NULL,
    session_role_name text NOT NULL,
    selected_role_name text NOT NULL,
    binding_version bigint NOT NULL CHECK (binding_version > 0),
    case_key bigint NOT NULL CHECK (case_key > 0),
    action text NOT NULL CHECK (action IN ('ASSIGN_QUEUE', 'SET_DUE_AT', 'ESCALATE')),
    action_revision bigint NOT NULL CHECK (action_revision > 0),
    outcome text NOT NULL,
    reason_code text,
    control jsonb,
    resulting_publication_revision bigint,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    UNIQUE (binding_id, request_key)
);

SELECT pg_catalog.pg_extension_config_dump('mdm_steward.policy_bindings_v1'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_steward.policy_receipts_v1'::pg_catalog.regclass, '');
REVOKE ALL ON TABLE mdm_steward.policy_bindings_v1 FROM PUBLIC;
REVOKE ALL ON TABLE mdm_steward.policy_receipts_v1 FROM PUBLIC;
REVOKE ALL ON TABLE mdm_internal.policy_binding_runtime FROM PUBLIC;

CREATE FUNCTION mdm_internal.persist_create_policy_binding(request internal)
RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'persist_create_policy_binding_wrapper';
CREATE FUNCTION mdm_internal.persist_replace_policy_binding(request internal)
RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'persist_replace_policy_binding_wrapper';
CREATE FUNCTION mdm_internal.persist_set_policy_binding_state(request internal)
RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'persist_set_policy_binding_state_wrapper';
CREATE FUNCTION mdm_internal.persist_set_case_controls(request internal)
RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'persist_set_case_controls_wrapper';
CREATE FUNCTION mdm_internal.persist_policy_intent(request internal)
RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'persist_policy_intent_wrapper';

CREATE FUNCTION mdm_admin.create_policy_binding(entity_name text, automation_role_name text, policy_digest bytea, allowed_actions text[], allowed_queues text[], max_due_interval interval, max_escalation_level integer)
RETURNS TABLE (binding_id uuid, binding_version bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'create_policy_binding_wrapper';
CREATE FUNCTION mdm_admin.replace_policy_binding(binding_id uuid, expected_version bigint, policy_digest bytea, allowed_actions text[], allowed_queues text[], max_due_interval interval, max_escalation_level integer)
RETURNS TABLE (binding_id uuid, binding_version bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'replace_policy_binding_wrapper';
CREATE FUNCTION mdm_admin.set_policy_binding_state(binding_id uuid, expected_runtime_version bigint, state text, reason text)
RETURNS bigint LANGUAGE c AS 'MODULE_PATHNAME', 'set_policy_binding_state_wrapper';
CREATE FUNCTION mdm_steward.set_case_controls(case_key bigint, assigned_queue name, due_at timestamptz, escalation_level integer, manual_assignment_protected boolean, expected_action_revision bigint, reason text)
RETURNS TABLE (operation_id uuid, action_revision bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'set_case_controls_wrapper';
CREATE FUNCTION mdm_steward.submit_policy_intent(binding_id uuid, request_key bytea, case_key bigint, action text, arguments jsonb, expected_review_version bigint, expected_definition_version bigint, expected_publication_revision bigint, expected_stewardship_epoch bigint, expected_evidence_basis_digest bytea, expected_action_revision bigint, expected_policy_digest bytea, policy_revision text, evaluation_ref text, work_ref text)
RETURNS TABLE (receipt_id uuid, outcome text, reason_code text, case_key bigint, action_revision bigint, control jsonb, resulting_publication_revision bigint)
LANGUAGE c AS 'MODULE_PATHNAME', 'submit_policy_intent_wrapper';

REVOKE ALL ON FUNCTION mdm_internal.persist_create_policy_binding(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_replace_policy_binding(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_set_policy_binding_state(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_set_case_controls(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_policy_intent(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.create_policy_binding(text, text, bytea, text[], text[], interval, integer) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.replace_policy_binding(uuid, bigint, bytea, text[], text[], interval, integer) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.set_policy_binding_state(uuid, bigint, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_steward.set_case_controls(bigint, name, timestamptz, integer, boolean, bigint, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_steward.submit_policy_intent(uuid, bytea, bigint, text, jsonb, bigint, bigint, bigint, bigint, bytea, bigint, bytea, text, text, text) FROM PUBLIC;

DO $grant_entity_roles$
DECLARE
    entity_role record;
BEGIN
    FOR entity_role IN SELECT DISTINCT execution_role_name FROM mdm_internal.entities LOOP
        EXECUTE pg_catalog.format('GRANT USAGE ON SCHEMA mdm_admin, mdm_steward TO %I', entity_role.execution_role_name);
    EXECUTE pg_catalog.format('GRANT EXECUTE ON FUNCTION mdm_admin.create_policy_binding(text, text, bytea, text[], text[], interval, integer), mdm_admin.replace_policy_binding(uuid, bigint, bytea, text[], text[], interval, integer), mdm_admin.set_policy_binding_state(uuid, bigint, text, text), mdm_steward.set_case_controls(bigint, name, timestamptz, integer, boolean, bigint, text) TO %I', entity_role.execution_role_name);
    END LOOP;
END
$grant_entity_roles$;
