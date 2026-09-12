\if :{?helper_owner}
\else
\echo 'set helper_owner, for example: -v helper_owner=mdm_helper_owner'
\quit 1
\endif

SELECT pg_catalog.count(*) = 1
       AND pg_catalog.bool_and(NOT rolsuper AND NOT rolcanlogin AND NOT rolbypassrls)
       AND NOT EXISTS (
           SELECT 1
           FROM pg_catalog.pg_auth_members membership
           JOIN pg_catalog.pg_roles role ON role.oid = membership.roleid
           WHERE role.rolname = :'helper_owner'
              OR membership.member = (
                  SELECT oid FROM pg_catalog.pg_roles WHERE rolname = :'helper_owner'
              )
       ) AS helper_owner_safe
FROM pg_catalog.pg_roles
WHERE rolname = :'helper_owner'
\gset

\if :helper_owner_safe
\else
\echo 'helper owner must exist and be NOLOGIN, NOSUPERUSER, NOBYPASSRLS, with no memberships or members'
\quit 1
\endif

BEGIN;
GRANT USAGE ON SCHEMA mdm_admin, mdm_internal, mdm_graph TO :"helper_owner";
GRANT USAGE, CREATE ON SCHEMA mdm_out TO :"helper_owner";
GRANT USAGE ON SCHEMA pgtrickle TO :"helper_owner" WITH GRANT OPTION;
GRANT EXECUTE ON FUNCTION pgtrickle.integration_capabilities() TO :"helper_owner";
GRANT EXECUTE ON FUNCTION pgtrickle.create_stream_table(text, text, text, text, boolean, text, text, text, boolean, boolean, text, integer, double precision, text, boolean, text, integer, text, text) TO :"helper_owner";
GRANT EXECUTE ON FUNCTION pgtrickle.stream_table_contract(regclass) TO :"helper_owner";
GRANT EXECUTE ON FUNCTION pgtrickle.graph_contract(regclass[]) TO :"helper_owner";
GRANT EXECUTE ON FUNCTION pgtrickle.refresh_graph_strict(regclass[], bytea, text) TO :"helper_owner";
GRANT EXECUTE ON FUNCTION pgtrickle.encode_row_id_v2(text, anyelement) TO :"helper_owner" WITH GRANT OPTION;
GRANT EXECUTE ON FUNCTION pgtrickle.drop_stream_table(text, boolean) TO :"helper_owner";
GRANT EXECUTE ON FUNCTION mdm_internal.normalize_text(text, text, integer, text, jsonb) TO :"helper_owner";
GRANT EXECUTE ON FUNCTION mdm_internal.normalize_date(date, text, integer, text, jsonb) TO :"helper_owner";
ALTER TABLE mdm_internal.operations OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.entities OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.definitions OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.source_identities OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.source_bindings OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.output_names OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.source_records OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.execution_role_bindings OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.definition_artifacts OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.graph_bindings OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.graph_members OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.steward_decisions OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.publications OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.publication_observations OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.identity_registry OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.memberships OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.identity_aliases OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.identity_splits OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.output_fields OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.golden_provenance OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.reviews OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.resolution_facts OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.golden_override_directives OWNER TO :"helper_owner";
ALTER FUNCTION mdm_admin.verify_installation() OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.persist_entity(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.describe_entity(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.explain_entity(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.prepare_rebind(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.persist_rebind(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.persist_refresh(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.refresh_access(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.preview_entity(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.persist_decision(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.persist_golden_override(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_admin.drop_entity(text, text) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_admin.rebuild(text, text) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.persist_drop_entity(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.normalized_levenshtein_score(text, text, bigint) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.evidence_digest(text) OWNER TO :"helper_owner";
ALTER SCHEMA mdm_graph OWNER TO :"helper_owner";
ALTER TABLE mdm_graph.source_identity_map OWNER TO :"helper_owner";
ALTER TABLE mdm_graph.source_records OWNER TO :"helper_owner";
ALTER TABLE mdm_graph.definition_limits OWNER TO :"helper_owner";
ALTER FUNCTION mdm_graph.normalize_text(text, text, integer, text, jsonb) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_graph.normalize_date(date, text, integer, text, jsonb) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_graph.normalized_levenshtein_score(text, text, bigint) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_graph.evidence_digest(text) OWNER TO :"helper_owner";
COMMIT;
