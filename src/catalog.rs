use pgrx::prelude::*;
use pgrx::{Internal, JsonB};
use serde_json::{Value, json};

use crate::error::MdmError;
use crate::integration::PgTrickleCapabilities;
#[allow(unused_imports)]
use crate::integration::require_graph_v1_sql;
use crate::version::{PACKAGE_VERSION, PG_TRICKLE_VERSION};

#[derive(Debug)]
pub(crate) struct Role {
    pub(crate) oid: pg_sys::Oid,
    pub(crate) name: String,
    pub(crate) superuser: bool,
    pub(crate) can_login: bool,
    pub(crate) bypass_rls: bool,
}

fn role(oid: pg_sys::Oid) -> Result<Role, MdmError> {
    Spi::connect(|client| {
        let table = client
            .select(
                "SELECT rolname::text, rolsuper, rolcanlogin, rolbypassrls FROM pg_catalog.pg_roles WHERE oid = $1",
                Some(1),
                &[oid.into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if table.is_empty() {
            return Err(MdmError::Unauthorized(format!(
                "role OID {oid} does not exist"
            )));
        }
        let row = table.first();
        Ok(Role {
            oid,
            name: row
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("role name is NULL".into()))?,
            superuser: row
                .get::<bool>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
            can_login: row
                .get::<bool>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
            bypass_rls: row
                .get::<bool>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
        })
    })
}

pub(crate) fn session_user_id() -> pg_sys::Oid {
    unsafe {
        // SAFETY: PostgreSQL invokes extension functions on the backend thread.
        pg_sys::GetSessionUserId()
    }
}

pub(crate) fn outer_user_id() -> pg_sys::Oid {
    unsafe {
        // SAFETY: PostgreSQL preserves the invoker ID while it applies SECURITY DEFINER.
        pg_sys::GetOuterUserId()
    }
}

/// Invoke a private helper with an in-memory request that SQL callers cannot construct.
/// Each constant helper name must be paired with its request's Rust type.
pub(crate) fn call_helper<T: 'static>(name: &str, request: T) -> Result<JsonB, MdmError> {
    let owner = validate_helper_owner()?;
    let oid = Spi::get_one_with_args::<pg_sys::Oid>(
        "SELECT p.oid FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = 'mdm_internal' AND p.proname = $1 AND p.pronargs = 1 AND p.proargtypes[0] = 'pg_catalog.internal'::pg_catalog.regtype AND p.prorettype = 'pg_catalog.jsonb'::pg_catalog.regtype AND p.prosecdef AND p.proowner = $2",
        &[name.into(), owner.oid.into()],
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .ok_or_else(|| MdmError::HelperOwnerUnsafe(format!("private helper {name} is missing or has an unsafe owner")))?;
    let argument = Internal::new(request)
        .into_datum()
        .expect("internal request is initialized");
    // SAFETY: callers pair a fixed helper with its request type. PostgreSQL's fmgr
    // applies SECURITY DEFINER and restores its context, including on ERROR.
    // Every helper returns a non-NULL jsonb; SQL cannot construct an internal value.
    unsafe {
        let result = pg_sys::OidFunctionCall1Coll(oid, pg_sys::InvalidOid, argument);
        JsonB::from_datum(result, false)
    }
    .ok_or_else(|| MdmError::OperationState("private helper returned NULL".into()))
}

pub(crate) fn validate_helper_owner() -> Result<Role, MdmError> {
    let owner_oid = Spi::get_one::<pg_sys::Oid>(
        "SELECT p.proowner FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = 'mdm_admin' AND p.proname = 'verify_installation' AND p.pronargs = 0",
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .ok_or_else(|| MdmError::HelperOwnerUnsafe("verification helper is missing".into()))?;
    let table_owner = Spi::get_one::<pg_sys::Oid>(
        "SELECT c.relowner FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'mdm_internal' AND c.relname = 'operations'",
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .ok_or_else(|| MdmError::HelperOwnerUnsafe("operations table is missing".into()))?;
    if owner_oid != table_owner {
        return Err(MdmError::HelperOwnerUnsafe(
            "the helper and operations table have different owners".into(),
        ));
    }

    let mismatched_table = Spi::get_one_with_args::<bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'mdm_internal' AND c.relkind = 'r' AND c.relowner <> $1)",
        &[owner_oid.into()],
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .unwrap_or(true);
    if mismatched_table {
        return Err(MdmError::HelperOwnerUnsafe(
            "protected tables have different owners".into(),
        ));
    }

    let owner = role(owner_oid)?;
    if owner.superuser || owner.can_login || owner.bypass_rls {
        return Err(MdmError::HelperOwnerUnsafe(
            "owner must be NOLOGIN, NOSUPERUSER, and NOBYPASSRLS".into(),
        ));
    }
    let has_membership = Spi::get_one_with_args::<bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_auth_members WHERE roleid = $1 OR member = $1)",
        &[owner_oid.into()],
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .unwrap_or(false);
    if has_membership {
        return Err(MdmError::HelperOwnerUnsafe(
            "owner must not have role memberships or members".into(),
        ));
    }
    Ok(owner)
}

pub(crate) fn validate_caller(helper_owner: &Role) -> Result<(Role, Role), MdmError> {
    let session = role(session_user_id())?;
    let selected = role(outer_user_id())?;
    if session.superuser || session.bypass_rls || selected.superuser || selected.bypass_rls {
        return Err(MdmError::Unauthorized(
            "superuser and BYPASSRLS roles cannot record MDM operations".into(),
        ));
    }
    if selected.oid == helper_owner.oid {
        return Err(MdmError::Unauthorized(
            "the helper owner cannot act as an application role".into(),
        ));
    }
    let may_set_role = session.oid == selected.oid
        || Spi::get_one_with_args::<bool>(
            "SELECT pg_catalog.pg_has_role($1, $2, 'SET')",
            &[session.oid.into(), selected.oid.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .unwrap_or(false);
    if !may_set_role {
        return Err(MdmError::Unauthorized(format!(
            "{} cannot SET ROLE {}",
            session.name, selected.name
        )));
    }
    Ok((session, selected))
}

fn operation_outcome(capabilities: PgTrickleCapabilities) -> Value {
    json!({
        "package_version": PACKAGE_VERSION,
        "pg_trickle_version": PG_TRICKLE_VERSION,
        "external_graph_refresh": capabilities.external_graph_refresh,
        "output_delta_consumer": capabilities.output_delta_consumer,
    })
}

pub(crate) fn record_foundation_check(
    capabilities: PgTrickleCapabilities,
) -> Result<String, MdmError> {
    let helper_owner = validate_helper_owner()?;
    let (session, selected) = validate_caller(&helper_owner)?;
    let outcome = JsonB(operation_outcome(capabilities));
    let mut operation_id = None;
    Spi::connect_mut(|client| {
        let started = client
            .update(
                "INSERT INTO mdm_internal.operations (operation_kind, status, outcome, actor_name, actor_role_name) \
                 VALUES ('foundation_check', 'running', $1, $2, $3) \
                 RETURNING operation_id::text",
                Some(1),
                &[outcome.into(), session.name.into(), selected.name.into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if started.is_empty() {
            return Err(MdmError::OperationState(
                "operation did not enter running state".into(),
            ));
        }
        let id = started
            .first()
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::OperationState("operation ID is NULL".into()))?;
        let completed = client
            .update(
                "UPDATE mdm_internal.operations \
                    SET status = 'succeeded', result_code = 'MDM_OK', completed_at = pg_catalog.statement_timestamp() \
                  WHERE operation_id = $1::pg_catalog.uuid \
                    AND status = 'running' \
                    AND completed_at IS NULL \
                 RETURNING operation_id::text",
                Some(1),
                &[id.into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if completed.is_empty() {
            return Err(MdmError::OperationState(
                "running operation did not become succeeded".into(),
            ));
        }
        operation_id = completed
            .first()
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        Ok::<_, MdmError>(())
    })?;
    operation_id.ok_or_else(|| {
        MdmError::OperationState("running operation did not become succeeded".into())
    })
}

#[pg_extern(
    name = "verify_installation",
    security_definer,
    requires = [require_graph_v1_sql],
    sql = "CREATE FUNCTION mdm_admin.verify_installation() RETURNS text STRICT SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'verify_installation_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn verify_installation() -> String {
    let capabilities = match crate::integration::integration_capabilities() {
        Ok(capabilities) => capabilities,
        Err(error) => crate::raise(error),
    };
    match record_foundation_check(capabilities) {
        Ok(operation_id) => operation_id,
        Err(error) => crate::raise(error),
    }
}
