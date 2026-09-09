use pgrx::prelude::*;

#[allow(unused_imports)]
use crate::catalog::verify_installation;

extension_sql!(
    r#"
CREATE SCHEMA mdm;
CREATE SCHEMA mdm_out;
CREATE SCHEMA mdm_steward;
CREATE SCHEMA mdm_admin;
CREATE SCHEMA mdm_internal;

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

COMMENT ON TABLE mdm_internal.operations IS
    'Durable: committed MDM operation outcomes; included in logical dumps.';

SELECT pg_catalog.pg_extension_config_dump(
    'mdm_internal.operations'::pg_catalog.regclass,
    ''
);

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
    "#,
    name = "pg_mdm_acl_policy",
    requires = [verify_installation],
    finalize,
);
