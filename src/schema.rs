use pgrx::prelude::*;

#[allow(unused_imports)]
use crate::api::constructors::{entity, field, golden_value, match_rule, source};
#[allow(unused_imports)]
use crate::api::create::{create, persist_entity, persist_recompile, recompile};
#[allow(unused_imports)]
use crate::api::describe::{describe, describe_entity};
#[allow(unused_imports)]
use crate::api::explain::{explain, explain_entity};
#[allow(unused_imports)]
use crate::api::lifecycle::{drop_entity, persist_drop_entity};
#[allow(unused_imports)]
use crate::api::rebind::{persist_rebind, prepare_rebind, rebind};
#[allow(unused_imports)]
use crate::api::refresh::{persist_refresh, preview, rebuild, refresh, refresh_access};
#[allow(unused_imports)]
use crate::api::steward::{
    clear_golden_override, decide, override_golden, persist_decision, persist_golden_override,
};
#[allow(unused_imports)]
use crate::catalog::verify_installation;
#[allow(unused_imports)]
use crate::comparators::levenshtein::normalized_levenshtein_score;
#[allow(unused_imports)]
use crate::evidence::evidence_digest;
#[allow(unused_imports)]
use crate::normalization::{normalize_date, normalize_text};

extension_sql!(
    r#"
CREATE SCHEMA mdm;
CREATE SCHEMA mdm_out;
CREATE SCHEMA mdm_steward;
CREATE SCHEMA mdm_admin;
CREATE SCHEMA mdm_internal;
CREATE SCHEMA mdm_graph;

CREATE TYPE mdm_internal.normalized_value AS (
    state text,
    normalized text,
    canonical_bytes bytea
);

CREATE TYPE mdm_graph.normalized_value AS (
    state text,
    normalized text,
    canonical_bytes bytea
);

CREATE TABLE mdm_internal.operations (
    operation_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    operation_kind text NOT NULL,
    entity_name pg_catalog.name,
    status text NOT NULL CHECK (status IN ('running', 'succeeded', 'failed')),
    result_code text,
    outcome jsonb NOT NULL DEFAULT '{}'::pg_catalog.jsonb,
    actor_name text NOT NULL,
    actor_role_name text NOT NULL,
    started_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    completed_at timestamptz,
    CHECK ((status = 'running') = (completed_at IS NULL))
);

CREATE TABLE mdm_internal.entities (
    entity_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_name pg_catalog.name NOT NULL UNIQUE,
    execution_role_name text NOT NULL,
    desired_version bigint NOT NULL CHECK (desired_version > 0),
    active_version bigint,
    decision_epoch bigint NOT NULL DEFAULT 0,
    publication_revision bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    created_by_name text NOT NULL,
    CHECK (active_version IS NULL OR active_version > 0),
    CHECK (decision_epoch >= 0),
    CHECK (publication_revision >= 0)
);

CREATE SEQUENCE mdm_internal.policy_case_key_seq AS bigint START WITH 1;

CREATE TABLE mdm_internal.definitions (
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    definition_version bigint NOT NULL CHECK (definition_version > 0),
    parent_version bigint,
    user_definition jsonb NOT NULL,
    expanded_definition jsonb NOT NULL,
    logical_candidate_plan jsonb NOT NULL,
    semantic_manifest jsonb NOT NULL,
    definition_digest bytea NOT NULL CHECK (octet_length(definition_digest) = 32),
    comment text,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    created_by_name text NOT NULL,
    PRIMARY KEY (entity_id, definition_version),
    CHECK (parent_version IS NULL OR parent_version < definition_version)
);

CREATE TABLE mdm_internal.source_identities (
    source_identity_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    source_name pg_catalog.name NOT NULL,
    relation_name text NOT NULL,
    key_contract jsonb NOT NULL,
    identity_digest bytea NOT NULL CHECK (octet_length(identity_digest) = 32),
    UNIQUE (entity_id, source_name)
);

CREATE TABLE mdm_internal.source_bindings (
    source_identity_id uuid PRIMARY KEY REFERENCES mdm_internal.source_identities(source_identity_id),
    relation_oid oid NOT NULL,
    binding_fingerprint jsonb NOT NULL,
    bound_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp()
);

CREATE TABLE mdm_internal.source_records (
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    source_identity_id uuid NOT NULL
        REFERENCES mdm_internal.source_identities(source_identity_id),
    source_record_key bytea NOT NULL,
    source_record_id uuid NOT NULL DEFAULT pg_catalog.uuidv7(),
    active boolean NOT NULL DEFAULT false,
    first_seen_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    last_seen_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    PRIMARY KEY (entity_id, source_identity_id, source_record_key),
    UNIQUE (source_record_id),
    UNIQUE (entity_id, source_record_id)
);

CREATE TABLE mdm_internal.output_names (
    output_name pg_catalog.name PRIMARY KEY,
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    output_kind text NOT NULL CHECK (output_kind IN ('entity', 'members', 'review')),
    UNIQUE (entity_id, output_kind)
);

CREATE TABLE mdm_internal.execution_role_bindings (
    entity_id uuid PRIMARY KEY REFERENCES mdm_internal.entities(entity_id),
    role_oid oid NOT NULL,
    bound_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp()
);

CREATE TABLE mdm_internal.definition_artifacts (
    artifact_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_id uuid NOT NULL,
    definition_version bigint NOT NULL,
    compiler_version integer NOT NULL CHECK (compiler_version > 0),
    artifact_format_version integer NOT NULL CHECK (artifact_format_version > 0),
    artifact_bytes bytea NOT NULL,
    artifact_digest bytea NOT NULL CHECK (octet_length(artifact_digest) = 32),
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    created_by_name text NOT NULL,
    UNIQUE (entity_id, definition_version, compiler_version, artifact_digest),
    FOREIGN KEY (entity_id, definition_version) REFERENCES mdm_internal.definitions(entity_id, definition_version)
);

CREATE TABLE mdm_internal.graph_bindings (
    graph_binding_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    definition_version bigint NOT NULL,
    artifact_id uuid NOT NULL
        REFERENCES mdm_internal.definition_artifacts(artifact_id),
    graph_generation bigint NOT NULL CHECK (graph_generation > 0),
    execution_role_oid oid NOT NULL,
    source_binding_digest bytea NOT NULL
        CHECK (octet_length(source_binding_digest) = 32),
    root_relation_oids oid[] NOT NULL,
    graph_contract_version smallint NOT NULL CHECK (graph_contract_version = 1),
    graph_digest bytea NOT NULL CHECK (octet_length(graph_digest) = 32),
    graph_contract jsonb NOT NULL,
    graph_binding_digest bytea NOT NULL
        CHECK (octet_length(graph_binding_digest) = 32),
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    UNIQUE (entity_id, graph_generation),
    FOREIGN KEY (entity_id, definition_version)
        REFERENCES mdm_internal.definitions(entity_id, definition_version)
);

CREATE TABLE mdm_internal.graph_members (
    graph_binding_id uuid NOT NULL
        REFERENCES mdm_internal.graph_bindings(graph_binding_id),
    logical_id text NOT NULL,
    topological_ordinal integer NOT NULL CHECK (topological_ordinal >= 0),
    relation_oid oid NOT NULL,
    relation_name text NOT NULL,
    contract_generation bigint NOT NULL CHECK (contract_generation > 0),
    contract_digest bytea NOT NULL CHECK (octet_length(contract_digest) = 32),
    contract jsonb NOT NULL,
    PRIMARY KEY (graph_binding_id, logical_id),
    UNIQUE (graph_binding_id, topological_ordinal),
    UNIQUE (relation_oid)
);

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

CREATE TABLE mdm_internal.steward_decisions (
    decision_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    left_source_record_id uuid NOT NULL,
    right_source_record_id uuid NOT NULL,
    decision text NOT NULL CHECK (decision IN ('MATCH', 'NOT_MATCH')),
    decision_version bigint NOT NULL CHECK (decision_version > 0),
    reason text NOT NULL CHECK (pg_catalog.length(pg_catalog.btrim(reason)) > 0),
    created_by_name text NOT NULL,
    created_as_role_name text NOT NULL,
    operation_id uuid NOT NULL REFERENCES mdm_internal.operations(operation_id),
    base_publication_revision bigint NOT NULL CHECK (base_publication_revision >= 0),
    decision_epoch bigint NOT NULL CHECK (decision_epoch > 0),
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    supersedes uuid UNIQUE,
    is_current boolean NOT NULL DEFAULT true,
    CHECK (left_source_record_id <> right_source_record_id),
    CHECK (supersedes IS NULL OR supersedes <> decision_id),
    UNIQUE (entity_id, left_source_record_id, right_source_record_id, decision_version),
    FOREIGN KEY (entity_id, left_source_record_id)
        REFERENCES mdm_internal.source_records(entity_id, source_record_id),
    FOREIGN KEY (entity_id, right_source_record_id)
        REFERENCES mdm_internal.source_records(entity_id, source_record_id)
);

CREATE UNIQUE INDEX steward_decisions_one_current_pair
    ON mdm_internal.steward_decisions (
        entity_id, left_source_record_id, right_source_record_id
    ) WHERE is_current;

CREATE OR REPLACE FUNCTION mdm_internal.validate_steward_decision_chain()
RETURNS trigger
LANGUAGE plpgsql
SET search_path TO pg_catalog, mdm_internal, pg_temp
AS $function$
DECLARE
    predecessor mdm_internal.steward_decisions%ROWTYPE;
    has_successor boolean;
    current_state boolean;
BEGIN
    IF NEW.supersedes IS NOT NULL THEN
        SELECT * INTO predecessor
        FROM mdm_internal.steward_decisions
        WHERE decision_id = NEW.supersedes;
        IF NOT FOUND
           OR predecessor.entity_id <> NEW.entity_id
           OR predecessor.left_source_record_id <> NEW.left_source_record_id
           OR predecessor.right_source_record_id <> NEW.right_source_record_id
           OR predecessor.decision_version + 1 <> NEW.decision_version
           OR predecessor.is_current THEN
            RAISE EXCEPTION 'invalid steward decision predecessor';
        END IF;
    END IF;
    SELECT EXISTS (
        SELECT 1 FROM mdm_internal.steward_decisions
        WHERE supersedes = NEW.decision_id
    ) INTO has_successor;
    SELECT is_current INTO current_state
    FROM mdm_internal.steward_decisions
    WHERE decision_id = NEW.decision_id;
    IF NOT FOUND OR current_state = has_successor THEN
        RAISE EXCEPTION 'steward decision current state does not match its successor';
    END IF;
    RETURN NEW;
END
$function$;

CREATE CONSTRAINT TRIGGER steward_decisions_chain_check
AFTER INSERT OR UPDATE ON mdm_internal.steward_decisions
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION mdm_internal.validate_steward_decision_chain();

CREATE TABLE mdm_internal.publications (
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    publication_revision bigint NOT NULL CHECK (publication_revision > 0),
    definition_version bigint NOT NULL,
    decision_epoch bigint NOT NULL,
    operation_id uuid NOT NULL REFERENCES mdm_internal.operations(operation_id),
    result_digest bytea NOT NULL CHECK (octet_length(result_digest) = 32),
    published_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    PRIMARY KEY (entity_id, publication_revision),
    FOREIGN KEY (entity_id, definition_version)
        REFERENCES mdm_internal.definitions(entity_id, definition_version)
);

CREATE TABLE mdm_internal.publication_observations (
    observation_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_id uuid NOT NULL,
    publication_revision bigint NOT NULL,
    decision_epoch bigint NOT NULL CHECK (decision_epoch >= 0),
    artifact_id uuid NOT NULL REFERENCES mdm_internal.definition_artifacts(artifact_id),
    operation_id uuid NOT NULL REFERENCES mdm_internal.operations(operation_id),
    graph_refresh_id bigint NOT NULL CHECK (graph_refresh_id > 0),
    source_boundary jsonb NOT NULL,
    source_boundary_digest bytea NOT NULL CHECK (octet_length(source_boundary_digest) = 32),
    node_results jsonb NOT NULL DEFAULT '{}'::jsonb,
    observed_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    FOREIGN KEY (entity_id, publication_revision)
        REFERENCES mdm_internal.publications(entity_id, publication_revision)
);

CREATE TABLE mdm_internal.identity_registry (
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    mdm_id uuid NOT NULL DEFAULT pg_catalog.uuidv7(),
    created_revision bigint NOT NULL,
    retired_revision bigint,
    status text NOT NULL CHECK (status IN ('active', 'merged', 'split', 'retired')),
    PRIMARY KEY (entity_id, mdm_id),
    CHECK (retired_revision IS NULL OR retired_revision >= created_revision),
    CHECK ((status = 'active') = (retired_revision IS NULL))
);

CREATE TABLE mdm_internal.memberships (
    entity_id uuid NOT NULL,
    source_record_id uuid NOT NULL,
    source_name name NOT NULL,
    source_id jsonb NOT NULL,
    mdm_id uuid NOT NULL,
    active boolean NOT NULL,
    first_membership_revision bigint NOT NULL,
    last_membership_revision bigint NOT NULL,
    membership_reason text NOT NULL,
    last_change_revision bigint NOT NULL,
    PRIMARY KEY (entity_id, source_record_id),
    FOREIGN KEY (entity_id, source_record_id)
        REFERENCES mdm_internal.source_records(entity_id, source_record_id),
    FOREIGN KEY (entity_id, mdm_id)
        REFERENCES mdm_internal.identity_registry(entity_id, mdm_id),
    CHECK (last_membership_revision >= first_membership_revision)
);

CREATE TABLE mdm_internal.identity_aliases (
    entity_id uuid NOT NULL,
    alias_mdm_id uuid NOT NULL,
    canonical_mdm_id uuid NOT NULL,
    publication_revision bigint NOT NULL,
    PRIMARY KEY (entity_id, alias_mdm_id),
    FOREIGN KEY (entity_id, alias_mdm_id)
        REFERENCES mdm_internal.identity_registry(entity_id, mdm_id),
    FOREIGN KEY (entity_id, canonical_mdm_id)
        REFERENCES mdm_internal.identity_registry(entity_id, mdm_id),
    CHECK (alias_mdm_id <> canonical_mdm_id)
);

CREATE TABLE mdm_internal.identity_splits (
    entity_id uuid NOT NULL,
    parent_mdm_id uuid NOT NULL,
    child_mdm_id uuid NOT NULL,
    publication_revision bigint NOT NULL,
    PRIMARY KEY (entity_id, parent_mdm_id, child_mdm_id, publication_revision),
    FOREIGN KEY (entity_id, parent_mdm_id)
        REFERENCES mdm_internal.identity_registry(entity_id, mdm_id),
    FOREIGN KEY (entity_id, child_mdm_id)
        REFERENCES mdm_internal.identity_registry(entity_id, mdm_id)
);

CREATE TABLE mdm_internal.output_fields (
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    field_name name NOT NULL,
    field_ordinal smallint NOT NULL CHECK (field_ordinal > 0),
    field_type_name text NOT NULL,
    created_definition_version bigint NOT NULL,
    PRIMARY KEY (entity_id, field_name),
    UNIQUE (entity_id, field_ordinal)
);

CREATE TABLE mdm_internal.golden_provenance (
    entity_id uuid NOT NULL,
    publication_revision bigint NOT NULL,
    mdm_id uuid NOT NULL,
    field_name name NOT NULL,
    value jsonb,
    normalized_value text,
    status text NOT NULL CHECK (status IN ('selected', 'override', 'override_conflict', 'no_value')),
    winning_source_record_id uuid,
    policy text NOT NULL,
    policy_version smallint NOT NULL,
    tie_break text NOT NULL,
    contributors jsonb NOT NULL,
    definition_version bigint NOT NULL,
    PRIMARY KEY (entity_id, publication_revision, mdm_id, field_name),
    FOREIGN KEY (entity_id, publication_revision)
        REFERENCES mdm_internal.publications(entity_id, publication_revision),
    FOREIGN KEY (entity_id, mdm_id)
        REFERENCES mdm_internal.identity_registry(entity_id, mdm_id)
);

CREATE TABLE mdm_internal.reviews (
    review_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    issue_key bytea NOT NULL CHECK (octet_length(issue_key) = 32),
    occurrence integer NOT NULL CHECK (occurrence > 0),
    status text NOT NULL CHECK (status IN ('open', 'resolved')),
    severity text NOT NULL,
    reason_code text NOT NULL,
    subjects jsonb NOT NULL,
    masked_summary jsonb NOT NULL,
    opened_revision bigint NOT NULL,
    resolved_revision bigint,
    last_change_revision bigint NOT NULL,
    concurrency_version bigint NOT NULL DEFAULT 1,
    UNIQUE (entity_id, issue_key, occurrence),
    CHECK ((status = 'resolved') = (resolved_revision IS NOT NULL))
);

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

CREATE TABLE mdm_internal.resolution_facts (
    entity_id uuid NOT NULL,
    publication_revision bigint NOT NULL,
    fact_number bigint NOT NULL,
    subject_kind text NOT NULL,
    subject_key bytea NOT NULL,
    fact_kind text NOT NULL,
    fact jsonb NOT NULL,
    PRIMARY KEY (entity_id, publication_revision, fact_number),
    FOREIGN KEY (entity_id, publication_revision)
        REFERENCES mdm_internal.publications(entity_id, publication_revision)
);

CREATE TABLE mdm_internal.golden_override_directives (
    override_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    field_name name NOT NULL,
    anchor_source_record_id uuid NOT NULL,
    action text NOT NULL CHECK (action IN ('SET', 'CLEAR')),
    value jsonb,
    value_type_name text,
    override_version bigint NOT NULL CHECK (override_version > 0),
    reason text NOT NULL CHECK (pg_catalog.length(pg_catalog.btrim(reason)) > 0),
    created_by_name text NOT NULL,
    created_as_role_name text NOT NULL,
    operation_id uuid NOT NULL REFERENCES mdm_internal.operations(operation_id),
    base_publication_revision bigint NOT NULL CHECK (base_publication_revision >= 0),
    decision_epoch bigint NOT NULL CHECK (decision_epoch > 0),
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    supersedes uuid UNIQUE,
    is_current boolean NOT NULL DEFAULT true,
    UNIQUE (entity_id, field_name, anchor_source_record_id, override_version),
    FOREIGN KEY (entity_id, anchor_source_record_id)
        REFERENCES mdm_internal.source_records(entity_id, source_record_id),
    CHECK (
        (action = 'SET' AND value IS NOT NULL AND value <> 'null'::jsonb AND value_type_name IS NOT NULL)
        OR (action = 'CLEAR' AND value IS NULL AND value_type_name IS NULL)
    )
);

CREATE UNIQUE INDEX golden_overrides_one_current_anchor
    ON mdm_internal.golden_override_directives (entity_id, field_name, anchor_source_record_id)
    WHERE is_current;

CREATE OR REPLACE FUNCTION mdm_internal.validate_golden_override_chain()
RETURNS trigger
LANGUAGE plpgsql
SET search_path TO pg_catalog, mdm_internal, pg_temp
AS $function$
DECLARE predecessor mdm_internal.golden_override_directives%ROWTYPE;
DECLARE has_successor boolean;
DECLARE current_state boolean;
BEGIN
    IF NEW.supersedes IS NOT NULL THEN
        SELECT * INTO predecessor FROM mdm_internal.golden_override_directives
         WHERE override_id = NEW.supersedes;
        IF NOT FOUND OR predecessor.entity_id <> NEW.entity_id
           OR predecessor.field_name <> NEW.field_name
           OR predecessor.anchor_source_record_id <> NEW.anchor_source_record_id
           OR predecessor.override_version + 1 <> NEW.override_version
           OR predecessor.is_current THEN
            RAISE EXCEPTION 'invalid golden override predecessor';
        END IF;
    END IF;
    SELECT EXISTS (SELECT 1 FROM mdm_internal.golden_override_directives WHERE supersedes = NEW.override_id)
      INTO has_successor;
    SELECT is_current INTO current_state FROM mdm_internal.golden_override_directives WHERE override_id = NEW.override_id;
    IF NOT FOUND OR current_state = has_successor THEN
        RAISE EXCEPTION 'golden override current state does not match its successor';
    END IF;
    RETURN NEW;
END
$function$;

CREATE CONSTRAINT TRIGGER golden_override_chain_check
AFTER INSERT OR UPDATE ON mdm_internal.golden_override_directives
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION mdm_internal.validate_golden_override_chain();

CREATE TABLE mdm_graph.source_identity_map (
    entity_id uuid NOT NULL,
    entity_name text NOT NULL,
    source_identity_id uuid NOT NULL,
    source_name text NOT NULL,
    PRIMARY KEY (entity_id, source_name)
);

CREATE TABLE mdm_graph.source_records (
    entity_id uuid NOT NULL,
    source_identity_id uuid NOT NULL,
    source_record_key bytea NOT NULL,
    source_record_id uuid PRIMARY KEY,
    active boolean NOT NULL
);

CREATE TABLE mdm_graph.definition_limits (
    entity_id uuid PRIMARY KEY,
    entity_name text NOT NULL,
    expanded_definition jsonb NOT NULL
);

CREATE FUNCTION mdm_graph.normalize_text(value text, cleaner text, cleaner_version integer, source_state text DEFAULT 'present', options jsonb DEFAULT '{}'::jsonb)
RETURNS mdm_graph.normalized_value
LANGUAGE sql IMMUTABLE PARALLEL SAFE SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
AS $graph$ SELECT ROW((n).state, (n).normalized, (n).canonical_bytes)::mdm_graph.normalized_value
FROM (SELECT mdm_internal.normalize_text($1, $2, $3, $4, $5) AS n) s $graph$;

CREATE FUNCTION mdm_graph.normalize_date(value date, cleaner text DEFAULT 'date', cleaner_version integer DEFAULT 1, source_state text DEFAULT 'present', options jsonb DEFAULT '{}'::jsonb)
RETURNS mdm_graph.normalized_value
LANGUAGE sql IMMUTABLE PARALLEL SAFE SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
AS $graph$ SELECT ROW((n).state, (n).normalized, (n).canonical_bytes)::mdm_graph.normalized_value
FROM (SELECT mdm_internal.normalize_date($1, $2, $3, $4, $5) AS n) s $graph$;

CREATE FUNCTION mdm_graph.normalized_levenshtein_score(left_value text, right_value text, max_work bigint)
RETURNS integer
LANGUAGE sql IMMUTABLE PARALLEL SAFE SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
AS $graph$ SELECT mdm_internal.normalized_levenshtein_score($1, $2, $3) $graph$;

CREATE FUNCTION mdm_graph.evidence_digest(value text)
RETURNS bytea
LANGUAGE sql IMMUTABLE PARALLEL SAFE SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
AS $graph$ SELECT mdm_internal.evidence_digest($1) $graph$;

COMMENT ON TABLE mdm_internal.operations IS
    'Durable: committed MDM operation outcomes; included in logical dumps.';

SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.operations'::pg_catalog.regclass,
    ''
);

SELECT pg_catalog.pg_extension_config_dump('mdm_internal.entities'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.policy_case_key_seq'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.definitions'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.source_identities'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.source_records'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.output_names'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.definition_artifacts'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.graph_bindings'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.graph_members'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.graph_delta_consumers'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.steward_decisions'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.publications'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.publication_observations'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.identity_registry'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.memberships'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.identity_aliases'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.identity_splits'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.output_fields'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.golden_provenance'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.reviews'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_steward.policy_cases_v1'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.resolution_facts'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.golden_override_directives'::pg_catalog.regclass, '');

REVOKE ALL ON SCHEMA mdm_internal FROM PUBLIC;
REVOKE ALL ON SEQUENCE mdm_internal.policy_case_key_seq FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA mdm_internal FROM PUBLIC;
REVOKE ALL ON TABLE mdm_steward.policy_cases_v1 FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.validate_golden_override_chain() FROM PUBLIC;
REVOKE CREATE ON SCHEMA mdm, mdm_out, mdm_steward, mdm_admin, mdm_graph FROM PUBLIC;
REVOKE ALL ON SCHEMA mdm_graph FROM PUBLIC;
REVOKE ALL ON TABLE mdm_internal.graph_bindings, mdm_internal.graph_members, mdm_internal.graph_delta_consumers FROM PUBLIC;
REVOKE ALL ON TABLE mdm_graph.source_identity_map, mdm_graph.source_records, mdm_graph.definition_limits FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_graph.normalize_text(text, text, integer, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_graph.normalize_date(date, text, integer, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_graph.normalized_levenshtein_score(text, text, bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_graph.evidence_digest(text) FROM PUBLIC;
"#,
    name = "pg_mdm_foundation",
    bootstrap,
);

extension_sql!(
    r#"
REVOKE ALL ON FUNCTION mdm_internal.integration_capabilities() FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.require_graph_v1() FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.verify_installation() FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_entity(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_recompile(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.describe_entity(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.explain_entity(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.prepare_rebind(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_rebind(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.rebind(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.drop_entity(text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.backfill_policy_case_opened_at(bigint, timestamptz, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.normalize_text(text, text, integer, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.normalize_date(date, text, integer, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.normalized_levenshtein_score(text, text, bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.evidence_digest(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.validate_steward_decision_chain() FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_decision(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_golden_override(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_drop_entity(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_backfill_policy_case_opened_at(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_refresh(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.preview_entity(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.refresh_access(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.recompile(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_steward.decide(text, uuid, uuid, text, bigint, text) FROM PUBLIC;
    "#,
    name = "pg_mdm_acl_policy",
    requires = [verify_installation, normalize_text, normalize_date],
    finalize,
);
