-- v0.7 adds durable identity, publication, golden, review, and explanation state.

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
    source_boundary jsonb NOT NULL,
    source_boundary_digest bytea NOT NULL CHECK (octet_length(source_boundary_digest) = 32),
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
        SELECT * INTO predecessor FROM mdm_internal.golden_override_directives WHERE override_id = NEW.supersedes;
        IF NOT FOUND OR predecessor.entity_id <> NEW.entity_id
           OR predecessor.field_name <> NEW.field_name
           OR predecessor.anchor_source_record_id <> NEW.anchor_source_record_id
           OR predecessor.override_version + 1 <> NEW.override_version
           OR predecessor.is_current THEN
            RAISE EXCEPTION 'invalid golden override predecessor';
        END IF;
    END IF;
    SELECT EXISTS (SELECT 1 FROM mdm_internal.golden_override_directives WHERE supersedes = NEW.override_id) INTO has_successor;
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

SELECT pg_catalog.pg_extension_config_dump('mdm_internal.publications'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.publication_observations'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.identity_registry'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.memberships'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.identity_aliases'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.identity_splits'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.output_fields'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.golden_provenance'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.reviews'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.resolution_facts'::pg_catalog.regclass, '');
SELECT pg_catalog.pg_extension_config_dump('mdm_internal.golden_override_directives'::pg_catalog.regclass, '');

CREATE FUNCTION mdm_internal.explain_entity(request internal)
RETURNS jsonb SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'explain_entity_wrapper';

CREATE FUNCTION mdm.explain(entity_name text, subject jsonb, publication_revision bigint DEFAULT NULL, max_facts integer DEFAULT 100)
RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'explain_wrapper';

CREATE FUNCTION mdm_internal.persist_golden_override(request internal)
RETURNS jsonb SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'persist_golden_override_wrapper';

CREATE FUNCTION mdm_steward.override_golden(entity_name text, anchor_source_record_id uuid, field_name text, value jsonb, expected_version bigint, reason text)
RETURNS TABLE (operation_id uuid, override_id uuid, override_version bigint, decision_epoch bigint)
LANGUAGE c AS 'MODULE_PATHNAME', 'override_golden_wrapper';

CREATE FUNCTION mdm_steward.clear_golden_override(entity_name text, anchor_source_record_id uuid, field_name text, expected_version bigint, reason text)
RETURNS TABLE (operation_id uuid, override_id uuid, override_version bigint, decision_epoch bigint)
LANGUAGE c AS 'MODULE_PATHNAME', 'clear_golden_override_wrapper';

REVOKE ALL ON FUNCTION mdm_internal.validate_golden_override_chain() FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.explain_entity(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_golden_override(internal) FROM PUBLIC;
