CREATE TYPE mdm_internal.normalized_value AS (
    state text,
    normalized text,
    canonical_bytes bytea
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

SELECT pg_catalog.pg_extension_config_dump('mdm_internal.source_records'::pg_catalog.regclass, '');

REVOKE ALL ON TABLE mdm_internal.source_records FROM PUBLIC;

CREATE FUNCTION mdm_internal.normalize_text(value text, cleaner text, cleaner_version integer, source_state text DEFAULT 'present', options jsonb DEFAULT '{}'::jsonb) RETURNS mdm_internal.normalized_value IMMUTABLE PARALLEL SAFE SET search_path = pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'normalize_text_wrapper';
CREATE FUNCTION mdm_internal.normalize_date(value date, cleaner text DEFAULT 'date', cleaner_version integer DEFAULT 1, source_state text DEFAULT 'present', options jsonb DEFAULT '{}'::jsonb) RETURNS mdm_internal.normalized_value IMMUTABLE PARALLEL SAFE SET search_path = pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'normalize_date_wrapper';

REVOKE ALL ON FUNCTION mdm_internal.normalize_text(text, text, integer, text, jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.normalize_date(date, text, integer, text, jsonb) FROM PUBLIC;

CREATE OR REPLACE FUNCTION mdm.field(name text, type text, cleaner text, cleaner_options jsonb DEFAULT '{}'::jsonb, display text DEFAULT 'masked') RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'field_wrapper';
CREATE OR REPLACE FUNCTION mdm.describe(entity_name text, format text DEFAULT 'summary') RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'describe_wrapper';
