-- v0.8 installs private executable Graph V1 bindings.

CREATE SCHEMA mdm_graph;

CREATE TYPE mdm_graph.normalized_value AS (
    state text,
    normalized text,
    canonical_bytes bytea
);

CREATE TABLE mdm_internal.graph_bindings (
    graph_binding_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_id uuid NOT NULL REFERENCES mdm_internal.entities(entity_id),
    definition_version bigint NOT NULL,
    artifact_id uuid NOT NULL REFERENCES mdm_internal.definition_artifacts(artifact_id),
    graph_generation bigint NOT NULL CHECK (graph_generation > 0),
    execution_role_oid oid NOT NULL,
    source_binding_digest bytea NOT NULL CHECK (octet_length(source_binding_digest) = 32),
    root_relation_oids oid[] NOT NULL,
    graph_contract_version smallint NOT NULL CHECK (graph_contract_version = 1),
    graph_digest bytea NOT NULL CHECK (octet_length(graph_digest) = 32),
    graph_contract jsonb NOT NULL,
    graph_binding_digest bytea NOT NULL CHECK (octet_length(graph_binding_digest) = 32),
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    UNIQUE (entity_id, graph_generation),
    FOREIGN KEY (entity_id, definition_version)
        REFERENCES mdm_internal.definitions(entity_id, definition_version)
);

CREATE TABLE mdm_internal.graph_members (
    graph_binding_id uuid NOT NULL REFERENCES mdm_internal.graph_bindings(graph_binding_id),
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

CREATE FUNCTION mdm_internal.persist_drop_entity(request internal)
RETURNS jsonb
SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'persist_drop_entity_wrapper';

CREATE FUNCTION mdm_admin.drop_entity(entity_name text, confirm text)
RETURNS jsonb
LANGUAGE c AS 'MODULE_PATHNAME', 'drop_entity_wrapper';

REVOKE ALL ON SCHEMA mdm_graph FROM PUBLIC;
REVOKE ALL ON TABLE mdm_internal.graph_bindings, mdm_internal.graph_members FROM PUBLIC;
REVOKE ALL ON TABLE mdm_graph.source_identity_map, mdm_graph.source_records, mdm_graph.definition_limits FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_graph.normalize_text(text, text, integer, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_graph.normalize_date(date, text, integer, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_graph.normalized_levenshtein_score(text, text, bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_graph.evidence_digest(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_drop_entity(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.drop_entity(text, text) FROM PUBLIC;
