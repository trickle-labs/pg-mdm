use pgrx::prelude::*;

#[allow(unused_imports)]
use crate::api::constructors::{entity, field, golden_value, match_rule, source};
#[allow(unused_imports)]
use crate::api::create::{create, persist_entity};
#[allow(unused_imports)]
use crate::api::describe::{describe, describe_entity};
#[allow(unused_imports)]
use crate::api::rebind::{persist_rebind, prepare_rebind, rebind};
#[allow(unused_imports)]
use crate::catalog::verify_installation;
#[allow(unused_imports)]
use crate::normalization::{normalize_date, normalize_text};

extension_sql!(
    r#"
CREATE SCHEMA mdm;
CREATE SCHEMA mdm_out;
CREATE SCHEMA mdm_steward;
CREATE SCHEMA mdm_admin;
CREATE SCHEMA mdm_internal;

CREATE TYPE mdm_internal.normalized_value AS (
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

COMMENT ON TABLE mdm_internal.operations IS
    'Durable: committed MDM operation outcomes; included in logical dumps.';

SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.operations'::pg_catalog.regclass,
    ''
);

SELECT pg_catalog.pg_extension_config_dump('mdm_internal.entities'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.definitions'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.source_identities'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.source_records'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.output_names'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.definition_artifacts'::pg_catalog.regclass, '');

REVOKE ALL ON SCHEMA mdm_internal FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA mdm_internal FROM PUBLIC;
REVOKE CREATE ON SCHEMA mdm, mdm_out, mdm_steward, mdm_admin FROM PUBLIC;
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
REVOKE ALL ON FUNCTION mdm_internal.describe_entity(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.prepare_rebind(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_rebind(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.rebind(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.normalize_text(text, text, integer, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.normalize_date(date, text, integer, text, jsonb) FROM PUBLIC;
    "#,
    name = "pg_mdm_acl_policy",
    requires = [verify_installation, normalize_text, normalize_date],
    finalize,
);
