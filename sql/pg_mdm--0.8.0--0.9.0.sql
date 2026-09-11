-- v0.9 adds strict Graph V1 refresh and atomic MDM publication.

ALTER TABLE mdm_internal.publication_observations
    ADD COLUMN graph_refresh_id bigint;

UPDATE mdm_internal.publication_observations
SET graph_refresh_id = publication_revision
WHERE graph_refresh_id IS NULL;

ALTER TABLE mdm_internal.publication_observations
    ALTER COLUMN graph_refresh_id SET NOT NULL;

ALTER TABLE mdm_internal.publication_observations
    ADD CONSTRAINT publication_observations_graph_refresh_id_check
    CHECK (graph_refresh_id > 0);

ALTER TABLE mdm_internal.publication_observations
    ADD COLUMN node_results jsonb NOT NULL DEFAULT '{}'::jsonb;

CREATE FUNCTION mdm.refresh(entity_name text, full_policy text DEFAULT 'ALLOW')
RETURNS jsonb
LANGUAGE c AS 'MODULE_PATHNAME', 'refresh_wrapper';

CREATE FUNCTION mdm.preview(entity_name text, mode text DEFAULT 'validation', options jsonb DEFAULT '{}'::jsonb)
RETURNS jsonb
LANGUAGE c AS 'MODULE_PATHNAME', 'preview_wrapper';

CREATE FUNCTION mdm_admin.rebuild(entity_name text, full_policy text DEFAULT 'ALLOW')
RETURNS jsonb
LANGUAGE c AS 'MODULE_PATHNAME', 'rebuild_wrapper';

CREATE FUNCTION mdm_internal.persist_refresh(request internal)
RETURNS jsonb
SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'persist_refresh_wrapper';

CREATE FUNCTION mdm_internal.preview_entity(request internal)
RETURNS jsonb
SECURITY DEFINER
SET search_path TO pg_catalog, mdm_internal, pg_temp
LANGUAGE c AS 'MODULE_PATHNAME', 'preview_entity_wrapper';

REVOKE ALL ON FUNCTION mdm_internal.persist_refresh(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_internal.preview_entity(internal) FROM PUBLIC;
