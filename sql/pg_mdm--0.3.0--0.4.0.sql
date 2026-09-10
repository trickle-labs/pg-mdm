-- v0.4 adds candidate generation to newly compiled artifacts.
-- No durable tables change; existing v0.3 definitions keep their original
-- candidate semantics until an explicit definition version supplies v0.4
-- limits and channel parameters.

CREATE OR REPLACE FUNCTION mdm.create(definition jsonb, expected_version bigint DEFAULT NULL, comment text DEFAULT NULL)
RETURNS TABLE (operation_id uuid, entity_name text, desired_version bigint, changed boolean, definition_digest bytea, artifact_digest bytea)
LANGUAGE c AS 'MODULE_PATHNAME', 'create_wrapper';

CREATE OR REPLACE FUNCTION mdm.describe(entity_name text, format text DEFAULT 'summary')
RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'describe_wrapper';
