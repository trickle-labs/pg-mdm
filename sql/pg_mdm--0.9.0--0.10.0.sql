-- v0.10 bounds operational lookup and refresh paths.

CREATE INDEX IF NOT EXISTS graph_bindings_entity_definition_generation
    ON mdm_internal.graph_bindings (entity_id, definition_version, graph_generation DESC);

CREATE INDEX IF NOT EXISTS graph_members_binding_ordinal
    ON mdm_internal.graph_members (graph_binding_id, topological_ordinal);

CREATE INDEX IF NOT EXISTS operations_entity_started
    ON mdm_internal.operations (entity_name, started_at DESC);

CREATE FUNCTION mdm_internal.refresh_access(request internal)
RETURNS jsonb
SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'refresh_access_wrapper';

REVOKE ALL ON FUNCTION mdm_internal.refresh_access(internal) FROM PUBLIC;
