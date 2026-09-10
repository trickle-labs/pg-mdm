CREATE TABLE mdm_internal.entities (
    entity_id uuid PRIMARY KEY DEFAULT pg_catalog.uuidv7(),
    entity_name pg_catalog.name NOT NULL UNIQUE,
    execution_role_name text NOT NULL,
    desired_version bigint NOT NULL CHECK (desired_version > 0),
    active_version bigint,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp(),
    created_by_name text NOT NULL,
    CHECK (active_version IS NULL OR active_version > 0)
);

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

SELECT pg_catalog.pg_extension_config_dump('mdm_internal.entities'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.definitions'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.source_identities'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.output_names'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.definition_artifacts'::pg_catalog.regclass, '');

CREATE FUNCTION mdm_internal.describe_entity(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'describe_entity_wrapper';
CREATE FUNCTION mdm.describe(entity_name text, format text DEFAULT 'summary') RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'describe_wrapper';
CREATE FUNCTION mdm.entity(name text, sources jsonb[], fields jsonb[], matches jsonb[], golden_values jsonb[], preset text DEFAULT NULL, limits jsonb DEFAULT '{}'::jsonb, execution_role text DEFAULT NULL) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'entity_wrapper';
CREATE FUNCTION mdm.field(name text, type text, cleaner text, cleaner_options jsonb DEFAULT '{}'::jsonb, display text DEFAULT 'masked') RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'field_wrapper';
CREATE FUNCTION mdm.golden_value(field text, policy text, sources text[] DEFAULT NULL) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'golden_value_wrapper';
CREATE FUNCTION mdm.match(name text, fields text[], comparison text, strength text, evidence_group text, threshold integer DEFAULT NULL, candidate jsonb DEFAULT NULL) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'match_rule_wrapper';
CREATE FUNCTION mdm_internal.persist_entity(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_entity_wrapper';
CREATE FUNCTION mdm.create(definition jsonb, expected_version bigint DEFAULT NULL, comment text DEFAULT NULL) RETURNS TABLE (operation_id uuid, entity_name text, desired_version bigint, changed boolean, definition_digest bytea, artifact_digest bytea) LANGUAGE c AS 'MODULE_PATHNAME', 'create_wrapper';
CREATE FUNCTION mdm.source(name text, relation regclass, source_id text[], mode text, fields jsonb, row_changed_at text DEFAULT NULL, soft_delete_when jsonb DEFAULT NULL, authority jsonb DEFAULT '{}'::jsonb) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'source_wrapper';

REVOKE ALL ON FUNCTION mdm_internal.describe_entity(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_entity(internal) FROM PUBLIC;

CREATE FUNCTION mdm_internal.prepare_rebind(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'prepare_rebind_wrapper';
CREATE FUNCTION mdm_internal.persist_rebind(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_rebind_wrapper';
CREATE FUNCTION mdm_admin.rebind(entity_name text) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'rebind_wrapper';
REVOKE ALL ON FUNCTION mdm_internal.prepare_rebind(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_rebind(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.rebind(text) FROM PUBLIC;
