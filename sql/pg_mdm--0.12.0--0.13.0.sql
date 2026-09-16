CREATE TABLE mdm_internal.graph_delta_consumers (
    graph_binding_id uuid NOT NULL
        REFERENCES mdm_internal.graph_bindings(graph_binding_id),
    logical_id text NOT NULL,
    consumer_id uuid NOT NULL UNIQUE,
    delta_relation_name text NOT NULL,
    output_contract_digest bytea NOT NULL
        CHECK (octet_length(output_contract_digest) = 32),
    row_identity_version smallint NOT NULL CHECK (row_identity_version > 0),
    PRIMARY KEY (graph_binding_id, logical_id),
    FOREIGN KEY (graph_binding_id, logical_id)
        REFERENCES mdm_internal.graph_members(graph_binding_id, logical_id)
);

SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.graph_bindings'::pg_catalog.regclass,
    ''
);
SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.graph_members'::pg_catalog.regclass,
    ''
);
SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.graph_delta_consumers'::pg_catalog.regclass,
    ''
);

REVOKE ALL ON TABLE mdm_internal.graph_delta_consumers FROM PUBLIC;

CREATE FUNCTION mdm_internal.persist_recompile(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_recompile_wrapper';
CREATE FUNCTION mdm_admin.recompile(entity_name text) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'recompile_wrapper';

REVOKE ALL ON FUNCTION mdm_internal.persist_recompile(internal) FROM PUBLIC;
REVOKE ALL ON FUNCTION mdm_admin.recompile(text) FROM PUBLIC;
