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
GRANT USAGE ON SCHEMA mdm_admin, mdm_internal, pgtrickle TO :"helper_owner";
GRANT EXECUTE ON FUNCTION pgtrickle.integration_capabilities() TO :"helper_owner";
ALTER TABLE mdm_internal.operations OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.entities OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.definitions OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.source_identities OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.source_bindings OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.output_names OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.source_records OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.execution_role_bindings OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.definition_artifacts OWNER TO :"helper_owner";
ALTER TABLE mdm_internal.steward_decisions OWNER TO :"helper_owner";
ALTER FUNCTION mdm_admin.verify_installation() OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.persist_entity(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.describe_entity(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.prepare_rebind(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.persist_rebind(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.persist_decision(internal) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.normalized_levenshtein_score(text, text, bigint) OWNER TO :"helper_owner";
ALTER FUNCTION mdm_internal.evidence_digest(text) OWNER TO :"helper_owner";
COMMIT;
