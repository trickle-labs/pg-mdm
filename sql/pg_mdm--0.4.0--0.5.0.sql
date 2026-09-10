-- v0.5 adds durable steward decisions and evidence metadata.

ALTER TABLE mdm_internal.entities
    ADD COLUMN decision_epoch bigint NOT NULL DEFAULT 0,
    ADD COLUMN publication_revision bigint NOT NULL DEFAULT 0;
ALTER TABLE mdm_internal.entities
    ADD CONSTRAINT entities_decision_epoch_nonnegative CHECK (decision_epoch >= 0),
    ADD CONSTRAINT entities_publication_revision_nonnegative CHECK (publication_revision >= 0);

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
    ON mdm_internal.steward_decisions
        (entity_id, left_source_record_id, right_source_record_id)
    WHERE is_current;

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
        SELECT 1
        FROM mdm_internal.steward_decisions
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

SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.steward_decisions'::pg_catalog.regclass,
    ''
);

CREATE FUNCTION mdm_internal.normalized_levenshtein_score(left_value text, right_value text, max_work bigint)
RETURNS integer IMMUTABLE PARALLEL SAFE
SET search_path = pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'normalized_levenshtein_score_wrapper';

CREATE FUNCTION mdm_internal.evidence_digest(value text)
RETURNS bytea IMMUTABLE PARALLEL SAFE
SET search_path = pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'evidence_digest_wrapper';

CREATE OR REPLACE FUNCTION mdm_steward.decide(entity_name text, left_source_record_id uuid, right_source_record_id uuid, decision text, expected_version bigint, reason text)
RETURNS TABLE (operation_id uuid, decision_id uuid, decision_version bigint, decision_epoch bigint)
LANGUAGE c AS 'MODULE_PATHNAME', 'decide_wrapper';

CREATE FUNCTION mdm_internal.persist_decision(request internal)
RETURNS jsonb SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'persist_decision_wrapper';

REVOKE ALL ON FUNCTION mdm_internal.normalized_levenshtein_score(text, text, bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.evidence_digest(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.validate_steward_decision_chain() FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.persist_decision(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_steward.decide(text, uuid, uuid, text, bigint, text) FROM PUBLIC;

CREATE OR REPLACE FUNCTION mdm.create(definition jsonb, expected_version bigint DEFAULT NULL, comment text DEFAULT NULL)
RETURNS TABLE (operation_id uuid, entity_name text, desired_version bigint, changed boolean, definition_digest bytea, artifact_digest bytea)
LANGUAGE c AS 'MODULE_PATHNAME', 'create_wrapper';

CREATE OR REPLACE FUNCTION mdm.describe(entity_name text, format text DEFAULT 'summary')
RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'describe_wrapper';
