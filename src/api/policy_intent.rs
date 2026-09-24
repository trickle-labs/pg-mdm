use pgrx::datum::{Interval, TimestampWithTimeZone};
use pgrx::prelude::*;
use pgrx::spi::SpiClient;
use pgrx::{Internal, JsonB, Uuid};
use serde_json::{Value, json};

use crate::catalog;
use crate::error::MdmError;
use crate::policy::{
    IntentArguments, PolicyIntentBody, intent_digest, parse_intent_arguments, queue_is_allowed,
    validate_reference,
};
use crate::source_record::quote_identifier;

#[derive(Clone)]
struct BindingRequest {
    entity_name: String,
    automation_role_name: String,
    policy_digest: Vec<u8>,
    allowed_actions: Vec<String>,
    allowed_queues: Vec<String>,
    max_due_interval: Option<String>,
    max_escalation_level: i32,
    actor: String,
    expected_version: Option<i64>,
    replace_binding_id: Option<Uuid>,
}

struct BindingStateRequest {
    binding_id: Uuid,
    expected_runtime_version: i64,
    state: String,
    reason: String,
    actor: String,
}

struct ControlRequest {
    case_key: i64,
    assigned_queue: Option<String>,
    due_at: Option<TimestampWithTimeZone>,
    escalation_level: i32,
    manual_assignment_protected: bool,
    expected_action_revision: i64,
    reason: String,
}

struct IntentRequest {
    binding_id: Uuid,
    request_key: Vec<u8>,
    case_key: i64,
    action: String,
    arguments: JsonB,
    expected_review_version: i64,
    expected_definition_version: i64,
    expected_publication_revision: i64,
    expected_stewardship_epoch: i64,
    expected_evidence_basis_digest: Vec<u8>,
    expected_action_revision: i64,
    expected_policy_digest: Vec<u8>,
    policy_revision: String,
    evaluation_ref: String,
    work_ref: String,
    actor: String,
    session_role_name: String,
    selected_role_name: String,
}

#[derive(Debug)]
struct Receipt {
    receipt_id: String,
    outcome: String,
    reason_code: String,
    case_key: i64,
    action_revision: i64,
    control: Option<Value>,
    resulting_publication_revision: Option<i64>,
}

fn parse_uuid(value: &str) -> Result<Uuid, MdmError> {
    let bytes = value
        .split('-')
        .flat_map(|part| {
            (0..part.len()).step_by(2).filter_map(|index| {
                part.get(index..index + 2)
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            })
        })
        .collect::<Vec<_>>();
    Uuid::from_slice(&bytes).map_err(|error| MdmError::OperationState(error.to_string()))
}

fn current_user() -> Result<String, MdmError> {
    Spi::get_one::<String>("SELECT current_user::text")
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Unauthorized("current_user is NULL".into()))
}

fn session_user() -> Result<String, MdmError> {
    Spi::get_one::<String>("SELECT session_user::text")
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Unauthorized("session_user is NULL".into()))
}

fn validate_entity_admin(entity_name: &str) -> Result<(String, String, pg_sys::Oid), MdmError> {
    let helper = catalog::validate_helper_owner()?;
    let (_, selected) = catalog::validate_caller(&helper)?;
    let entity = Spi::connect(|client| {
        let rows = client.select(
            "SELECT e.entity_id::text, e.execution_role_name, b.role_oid FROM mdm_internal.entities e JOIN mdm_internal.execution_role_bindings b USING (entity_id) WHERE e.entity_name = $1::pg_catalog.name",
            Some(1), &[entity_name.into()],
        ).map_err(|error| MdmError::Spi(error.to_string()))?;
        if rows.is_empty() {
            return Err(MdmError::Unauthorized(
                "entity execution role is not bound".into(),
            ));
        }
        let row = rows.first();
        Ok((
            row.get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("entity ID is NULL".into()))?,
            row.get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("execution role name is NULL".into()))?,
            row.get::<pg_sys::Oid>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("execution role OID is NULL".into()))?,
        ))
    })?;
    if selected.name != entity.1 || selected.oid != entity.2 {
        return Err(MdmError::Unauthorized(
            "binding administration requires the entity execution role".into(),
        ));
    }
    Ok((entity.0, selected.name, selected.oid))
}

fn validate_principal_role(
    name: &str,
    helper_oid: pg_sys::Oid,
    execution_oid: pg_sys::Oid,
) -> Result<pg_sys::Oid, MdmError> {
    let row = Spi::connect(|client| {
        let rows = client
            .select(
                "SELECT r.oid, r.rolname::text, r.rolsuper, r.rolcanlogin, r.rolbypassrls, pg_catalog.pg_has_role(r.oid, $2, 'MEMBER') OR pg_catalog.pg_has_role(r.oid, $3, 'MEMBER') FROM pg_catalog.pg_roles r WHERE r.rolname = $1",
                Some(1),
                &[name.into(), helper_oid.into(), execution_oid.into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if rows.is_empty() {
            return Err(MdmError::PolicyBinding(format!(
                "principal role {name} does not exist"
            )));
        }
        let row = rows.first();
        Ok((
            row.get::<pg_sys::Oid>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("principal role OID is NULL".into()))?,
            row.get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("principal role name is NULL".into()))?,
            row.get::<bool>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
            row.get::<bool>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
            row.get::<bool>(5)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
            row.get::<bool>(6)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(true),
        ))
    })?;
    if row.1 != name || row.2 || row.3 || row.4 || row.5 || row.0 == helper_oid {
        return Err(MdmError::PolicyBinding(
            "principal role must be NOLOGIN, NOSUPERUSER, NOBYPASSRLS, and unprivileged".into(),
        ));
    }
    let extension_owner = Spi::get_one_with_args::<bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'pg_mdm' AND extowner = $1)",
        &[row.0.into()],
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .unwrap_or(true);
    if extension_owner {
        return Err(MdmError::PolicyBinding(
            "principal role cannot own pg_mdm".into(),
        ));
    }
    Ok(row.0)
}

fn validate_actions(actions: &[String]) -> Result<(), MdmError> {
    let mut canonical = actions.to_vec();
    canonical.sort();
    canonical.dedup();
    if canonical != actions
        || canonical.is_empty()
        || canonical
            .iter()
            .any(|action| !matches!(action.as_str(), "ASSIGN_QUEUE" | "ESCALATE" | "SET_DUE_AT"))
    {
        return Err(MdmError::PolicyBinding(
            "allowed_actions must be sorted, distinct, and use only the three v0.14 actions".into(),
        ));
    }
    Ok(())
}

fn operation(
    client: &mut SpiClient<'_>,
    kind: &str,
    scope: Option<&str>,
    outcome: Value,
    actor: &str,
    selected: &str,
) -> Result<String, MdmError> {
    let row = client
        .update(
            "INSERT INTO mdm_internal.operations (operation_kind, entity_name, status, outcome, actor_name, actor_role_name) VALUES ($1, $2::pg_catalog.name, 'running', $3, $4, $5) RETURNING operation_id::text",
            Some(1),
            &[
                kind.into(),
                scope.map(str::to_owned).into(),
                JsonB(outcome).into(),
                actor.into(),
                selected.into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let id = row
        .first()
        .get::<String>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::OperationState("operation ID is NULL".into()))?;
    client
        .update(
            "UPDATE mdm_internal.operations SET status = 'succeeded', result_code = 'MDM_OK', completed_at = pg_catalog.statement_timestamp() WHERE operation_id = $1::pg_catalog.uuid AND status = 'running'",
            None,
            &[id.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    Ok(id)
}

fn database_oid() -> Result<pg_sys::Oid, MdmError> {
    Spi::get_one::<pg_sys::Oid>(
        "SELECT oid FROM pg_catalog.pg_database WHERE datname = pg_catalog.current_database()",
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .ok_or_else(|| MdmError::Spi("current database OID is NULL".into()))
}

fn interval_text(interval: Option<Interval>) -> Option<String> {
    interval.map(|interval| {
        let interval = interval.into_inner();
        format!(
            "{} mons {} days {} microseconds",
            interval.month, interval.day, interval.time
        )
    })
}

fn binding_result(value: &Value) -> Result<Uuid, MdmError> {
    parse_uuid(
        value["binding_id"]
            .as_str()
            .ok_or_else(|| MdmError::OperationState("binding ID is missing".into()))?,
    )
}

fn grant_binding_role(client: &mut SpiClient<'_>, principal_role: &str) -> Result<(), MdmError> {
    let role = quote_identifier(principal_role);
    for sql in [
        format!("GRANT USAGE ON SCHEMA mdm_steward TO {role}"),
        format!(
            "GRANT SELECT ON TABLE mdm_steward.policy_cases_v1, mdm_steward.policy_receipts_v1 TO {role}"
        ),
        format!(
            "GRANT EXECUTE ON FUNCTION mdm_steward.submit_policy_intent(uuid, bytea, bigint, text, jsonb, bigint, bigint, bigint, bigint, bytea, bigint, bytea, text, text, text) TO {role}"
        ),
    ] {
        client
            .update(&sql, None, &[])
            .map_err(|error| MdmError::Spi(error.to_string()))?;
    }
    Ok(())
}

fn validate_binding_request(
    request: &BindingRequest,
    execution_oid: pg_sys::Oid,
) -> Result<pg_sys::Oid, MdmError> {
    validate_actions(&request.allowed_actions)?;
    let mut queues = request.allowed_queues.clone();
    queues.sort();
    queues.dedup();
    if queues != request.allowed_queues || queues.iter().any(String::is_empty) {
        return Err(MdmError::PolicyBinding(
            "allowed_queues must be sorted, distinct, and nonempty".into(),
        ));
    }
    if request.allowed_actions.iter().any(|a| a == "ASSIGN_QUEUE") && queues.is_empty()
        || request.allowed_actions.iter().any(|a| a == "SET_DUE_AT")
            && request.max_due_interval.is_none()
        || request.allowed_actions.iter().any(|a| a == "ESCALATE")
            && request.max_escalation_level <= 0
        || request.max_escalation_level < 0
        || request.policy_digest.len() != 32
    {
        return Err(MdmError::PolicyBinding(
            "binding limits do not satisfy enabled actions".into(),
        ));
    }
    let helper = catalog::validate_helper_owner()?;
    validate_principal_role(&request.automation_role_name, helper.oid, execution_oid)
}

fn persist_binding(request: &BindingRequest) -> Result<Value, MdmError> {
    let (entity_id, selected, execution_oid) = validate_entity_admin(&request.entity_name)?;
    let automation_oid = validate_binding_request(request, execution_oid)?;
    let db_oid = database_oid()?;
    Spi::connect_mut(|client| {
        client
            .select(
                "SELECT entity_id FROM mdm_internal.entities WHERE entity_id = $1::uuid FOR UPDATE",
                Some(1),
                &[entity_id.clone().into()],
            )
            .map_err(|e| MdmError::Spi(e.to_string()))?;
        let (old_id, binding_id) = if let Some(id) = request.replace_binding_id {
            let old = client.select("SELECT binding_version, replaced_by::text FROM mdm_steward.policy_bindings_v1 WHERE binding_id = $1::uuid AND entity_id = $2::uuid FOR UPDATE", Some(1), &[id.into(), entity_id.clone().into()]).map_err(|e| MdmError::Spi(e.to_string()))?;
            let row = old.first();
            let version = row
                .get::<i64>(1)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| {
                    MdmError::PolicyBinding("binding does not exist for entity".into())
                })?;
            if row
                .get::<String>(2)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .is_some()
                || request.expected_version != Some(version)
            {
                return Err(MdmError::PolicyBinding(
                    "binding is replaced or expected version is stale".into(),
                ));
            }
            let new_id = Spi::get_one::<String>("SELECT pg_catalog.uuidv7()::text")
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| MdmError::Spi("replacement ID is NULL".into()))?;
            client.update("UPDATE mdm_steward.policy_bindings_v1 SET replaced_by = $2::uuid, replaced_at = pg_catalog.statement_timestamp() WHERE binding_id = $1::uuid", None, &[id.into(), new_id.clone().into()]).map_err(|e| MdmError::Spi(e.to_string()))?;
            client.update("UPDATE mdm_internal.policy_binding_runtime SET state = 'paused', runtime_version = runtime_version + 1, changed_at = pg_catalog.statement_timestamp() WHERE binding_id = $1::uuid", None, &[id.into()]).map_err(|e| MdmError::Spi(e.to_string()))?;
            (Some(id), new_id)
        } else {
            (None, String::new())
        };
        let inserted = client.update("INSERT INTO mdm_steward.policy_bindings_v1 (binding_id, entity_id, automation_role_name, policy_digest, allowed_actions, allowed_queues, max_due_interval, max_escalation_level, created_by_name, created_as_role_name) VALUES (COALESCE(NULLIF($1, '')::uuid, pg_catalog.uuidv7()), $2::uuid, $3::name, $4, $5, $6::name[], $7::interval, $8, $9, $10) RETURNING binding_id::text, binding_version", Some(1), &[binding_id.into(), entity_id.clone().into(), request.automation_role_name.clone().into(), request.policy_digest.clone().into(), request.allowed_actions.clone().into(), request.allowed_queues.clone().into(), request.max_due_interval.clone().into(), request.max_escalation_level.into(), request.actor.clone().into(), selected.clone().into()]).map_err(|e| MdmError::Spi(e.to_string()))?;
        let row = inserted.first();
        let id = row
            .get::<String>(1)
            .map_err(|e| MdmError::Spi(e.to_string()))?
            .ok_or_else(|| MdmError::Spi("binding ID is NULL".into()))?;
        let version = row
            .get::<i64>(2)
            .map_err(|e| MdmError::Spi(e.to_string()))?
            .ok_or_else(|| MdmError::Spi("binding version is NULL".into()))?;
        let state = if old_id.is_some() { "paused" } else { "active" };
        client.update("INSERT INTO mdm_internal.policy_binding_runtime (binding_id, database_oid, automation_role_oid, state, runtime_version) VALUES ($1::uuid, $2, $3, $4, 1)", None, &[id.clone().into(), db_oid.into(), automation_oid.into(), state.into()]).map_err(|e| MdmError::Spi(e.to_string()))?;
        grant_binding_role(client, &request.automation_role_name)?;
        operation(
            client,
            if old_id.is_some() {
                "policy_binding_replace"
            } else {
                "policy_binding_create"
            },
            Some(&request.entity_name),
            json!({"binding_id": id, "replaced_binding_id": old_id.map(|v| v.to_string()), "binding_version": version, "runtime_state": state}),
            &request.actor,
            &selected,
        )?;
        Ok(json!({"binding_id": id, "binding_version": version}))
    })
}

#[pg_extern(name = "create_policy_binding", requires = [persist_create_policy_binding], sql = "CREATE FUNCTION mdm_admin.create_policy_binding(entity_name text, automation_role_name text, policy_digest bytea, allowed_actions text[], allowed_queues text[], max_due_interval interval, max_escalation_level integer) RETURNS TABLE (binding_id uuid, binding_version bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'create_policy_binding_wrapper';")]
pub(crate) fn create_policy_binding(
    entity_name: String,
    automation_role_name: String,
    policy_digest: Vec<u8>,
    allowed_actions: Vec<String>,
    allowed_queues: Vec<String>,
    max_due_interval: Option<Interval>,
    max_escalation_level: i32,
) -> TableIterator<'static, (name!(binding_id, Uuid), name!(binding_version, i64))> {
    let result = (|| {
        let actor = current_user()?;
        let value = catalog::call_helper(
            "persist_create_policy_binding",
            BindingRequest {
                entity_name,
                automation_role_name,
                policy_digest,
                allowed_actions,
                allowed_queues,
                max_due_interval: interval_text(max_due_interval),
                max_escalation_level,
                actor,
                expected_version: None,
                replace_binding_id: None,
            },
        )?;
        Ok((
            binding_result(&value.0)?,
            value.0["binding_version"].as_i64().unwrap_or(1),
        ))
    })();
    match result {
        Ok(row) => TableIterator::new(vec![row]),
        Err(error) => crate::raise(error),
    }
}

#[pg_extern(
    name = "persist_create_policy_binding",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_create_policy_binding(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_create_policy_binding_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn persist_create_policy_binding(request: Internal) -> JsonB {
    let result = (|| {
        let request = unsafe { request.get::<BindingRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("binding request is required".into()))?;
        Ok(JsonB(persist_binding(request)?))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(name = "replace_policy_binding", requires = [persist_replace_policy_binding], sql = "CREATE FUNCTION mdm_admin.replace_policy_binding(binding_id uuid, expected_version bigint, policy_digest bytea, allowed_actions text[], allowed_queues text[], max_due_interval interval, max_escalation_level integer) RETURNS TABLE (binding_id uuid, binding_version bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'replace_policy_binding_wrapper';")]
pub(crate) fn replace_policy_binding(
    binding_id: Uuid,
    expected_version: i64,
    policy_digest: Vec<u8>,
    allowed_actions: Vec<String>,
    allowed_queues: Vec<String>,
    max_due_interval: Option<Interval>,
    max_escalation_level: i32,
) -> TableIterator<'static, (name!(binding_id, Uuid), name!(binding_version, i64))> {
    let result = (|| {
        let actor = current_user()?;
        let value = catalog::call_helper(
            "persist_replace_policy_binding",
            BindingRequest {
                entity_name: String::new(),
                automation_role_name: String::new(),
                policy_digest,
                allowed_actions,
                allowed_queues,
                max_due_interval: interval_text(max_due_interval),
                max_escalation_level,
                actor,
                expected_version: Some(expected_version),
                replace_binding_id: Some(binding_id),
            },
        )?;
        Ok((
            binding_result(&value.0)?,
            value.0["binding_version"].as_i64().unwrap_or(1),
        ))
    })();
    match result {
        Ok(row) => TableIterator::new(vec![row]),
        Err(error) => crate::raise(error),
    }
}

#[pg_extern(
    name = "persist_replace_policy_binding",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_replace_policy_binding(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_replace_policy_binding_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn persist_replace_policy_binding(request: Internal) -> JsonB {
    let result = (|| {
        let mut request = unsafe { request.get::<BindingRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("replacement request is required".into()))?
            .clone();
        let old_id = request
            .replace_binding_id
            .ok_or_else(|| MdmError::PolicyBinding("replacement binding ID is required".into()))?;
        (request.entity_name, request.automation_role_name) = Spi::connect(|client| {
            let rows = client.select(
                "SELECT e.entity_name::text, b.automation_role_name::text FROM mdm_steward.policy_bindings_v1 b JOIN mdm_internal.entities e USING (entity_id) WHERE b.binding_id = $1::uuid",
                Some(1), &[old_id.into()],
            ).map_err(|e| MdmError::Spi(e.to_string()))?;
            let row = rows.first();
            Ok((
                row.get::<String>(1)
                    .map_err(|e| MdmError::Spi(e.to_string()))?
                    .ok_or_else(|| MdmError::PolicyBinding("binding does not exist".into()))?,
                row.get::<String>(2)
                    .map_err(|e| MdmError::Spi(e.to_string()))?
                    .ok_or_else(|| MdmError::PolicyBinding("automation role is missing".into()))?,
            ))
        })?;
        Ok(JsonB(persist_binding(&request)?))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(name = "set_policy_binding_state", requires = [persist_set_policy_binding_state], sql = "CREATE FUNCTION mdm_admin.set_policy_binding_state(binding_id uuid, expected_runtime_version bigint, state text, reason text) RETURNS bigint LANGUAGE c AS 'MODULE_PATHNAME', 'set_policy_binding_state_wrapper';")]
pub(crate) fn set_policy_binding_state(
    binding_id: Uuid,
    expected_runtime_version: i64,
    state: String,
    reason: String,
) -> i64 {
    let result = catalog::call_helper(
        "persist_set_policy_binding_state",
        BindingStateRequest {
            binding_id,
            expected_runtime_version,
            state,
            reason,
            actor: current_user().unwrap_or_default(),
        },
    )
    .and_then(|value| {
        value.0["runtime_version"]
            .as_i64()
            .ok_or_else(|| MdmError::OperationState("runtime version missing".into()))
    });
    result.unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "persist_set_policy_binding_state",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_set_policy_binding_state(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_set_policy_binding_state_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn persist_set_policy_binding_state(request: Internal) -> JsonB {
    let result = (|| {
        let request = unsafe { request.get::<BindingStateRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("binding state request is required".into()))?;
        if !matches!(request.state.as_str(), "active" | "paused")
            || request.reason.trim().is_empty()
        {
            return Err(MdmError::PolicyBinding(
                "state must be active or paused and reason must be nonempty".into(),
            ));
        }
        let entity_name = Spi::get_one_with_args::<String>("SELECT e.entity_name::text FROM mdm_steward.policy_bindings_v1 b JOIN mdm_internal.entities e USING (entity_id) WHERE b.binding_id = $1::uuid", &[request.binding_id.into()]).map_err(|e| MdmError::Spi(e.to_string()))?.ok_or_else(|| MdmError::PolicyBinding("binding does not exist".into()))?;
        let (entity_id, selected_name, execution_oid) = validate_entity_admin(&entity_name)?;
        let automation_role = Spi::get_one_with_args::<String>(
            "SELECT automation_role_name::text FROM mdm_steward.policy_bindings_v1 WHERE binding_id = $1::uuid AND replaced_by IS NULL",
            &[request.binding_id.into()],
        )
        .map_err(|e| MdmError::Spi(e.to_string()))?
        .ok_or_else(|| MdmError::PolicyBinding("binding is replaced".into()))?;
        let helper = catalog::validate_helper_owner()?;
        let automation_oid = validate_principal_role(&automation_role, helper.oid, execution_oid)?;
        Spi::connect_mut(|client| {
            let row = client.update("INSERT INTO mdm_internal.policy_binding_runtime (binding_id, database_oid, automation_role_oid, state, runtime_version) SELECT $1::uuid, $2, $3, $4, 1 WHERE $5 >= 0 AND EXISTS (SELECT 1 FROM mdm_steward.policy_bindings_v1 WHERE binding_id = $1::uuid AND entity_id = $6::uuid AND replaced_by IS NULL) AND ($5 = 0 OR EXISTS (SELECT 1 FROM mdm_internal.policy_binding_runtime WHERE binding_id = $1::uuid AND runtime_version = $5)) ON CONFLICT (binding_id) DO UPDATE SET database_oid = EXCLUDED.database_oid, automation_role_oid = EXCLUDED.automation_role_oid, state = EXCLUDED.state, runtime_version = mdm_internal.policy_binding_runtime.runtime_version + 1, changed_at = pg_catalog.statement_timestamp() WHERE mdm_internal.policy_binding_runtime.runtime_version = $5 RETURNING runtime_version", Some(1), &[request.binding_id.into(), database_oid()?.into(), automation_oid.into(), request.state.clone().into(), request.expected_runtime_version.into(), entity_id.into()]).map_err(|e| MdmError::Spi(e.to_string()))?;
            let version = row
                .first()
                .get::<i64>(1)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| {
                    MdmError::PolicyBinding(
                        "runtime version is stale or binding is replaced".into(),
                    )
                })?;
            operation(
                client,
                "policy_binding_state",
                Some(&entity_name),
                json!({"binding_id": request.binding_id.to_string(), "state": request.state, "reason": request.reason, "runtime_version": version}),
                &request.actor,
                &selected_name,
            )?;
            Ok(JsonB(json!({"runtime_version": version})))
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "set_case_controls",
    requires = [persist_set_case_controls],
    sql = "CREATE FUNCTION mdm_steward.set_case_controls(case_key bigint, assigned_queue name, due_at timestamptz, escalation_level integer, manual_assignment_protected boolean, expected_action_revision bigint, reason text) RETURNS TABLE (operation_id uuid, action_revision bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'set_case_controls_wrapper';"
)]
pub(crate) fn set_case_controls(
    case_key: i64,
    assigned_queue: Option<String>,
    due_at: Option<TimestampWithTimeZone>,
    escalation_level: i32,
    manual_assignment_protected: bool,
    expected_action_revision: i64,
    reason: String,
) -> TableIterator<'static, (name!(operation_id, Uuid), name!(action_revision, i64))> {
    let result = (|| {
        let value = catalog::call_helper(
            "persist_set_case_controls",
            ControlRequest {
                case_key,
                assigned_queue,
                due_at,
                escalation_level,
                manual_assignment_protected,
                expected_action_revision,
                reason,
            },
        )?;
        Ok(vec![(
            parse_uuid(
                value.0["operation_id"]
                    .as_str()
                    .ok_or_else(|| MdmError::OperationState("operation ID is missing".into()))?,
            )?,
            value.0["action_revision"]
                .as_i64()
                .ok_or_else(|| MdmError::OperationState("action revision is missing".into()))?,
        )])
    })();
    match result {
        Ok(rows) => TableIterator::new(rows),
        Err(error) => crate::raise(error),
    }
}

#[pg_extern(
    name = "persist_set_case_controls",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_set_case_controls(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_set_case_controls_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn persist_set_case_controls(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only the public control wrapper constructs this request.
        let request = unsafe { request.get::<ControlRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("control request is required".into()))?;
        if request.case_key <= 0 || request.expected_action_revision <= 0 {
            return Err(MdmError::PolicyControl(
                "case_key and expected_action_revision must be positive".into(),
            ));
        }
        if request.escalation_level < 0 || request.reason.trim().is_empty() {
            return Err(MdmError::PolicyControl(
                "escalation level must be nonnegative and reason must not be empty".into(),
            ));
        }
        let helper = catalog::validate_helper_owner()?;
        let (session, selected) = catalog::validate_caller(&helper)?;
        Spi::connect_mut(|client| {
            let row = client
                .select(
                    "SELECT c.entity_name::text, c.status, c.assigned_queue::text, c.due_at::text, c.escalation_level, c.manual_assignment_protected, c.action_revision, e.execution_role_name, b.role_oid FROM mdm_steward.policy_cases_v1 c JOIN mdm_internal.entities e ON e.entity_name = c.entity_name LEFT JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id WHERE c.case_key = $1 FOR UPDATE OF c",
                    Some(1),
                    &[request.case_key.into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let row = row.first();
            let entity_name = row
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::PolicyControl("policy case does not exist".into()))?;
            let status = row
                .get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::PolicyControl("case status is NULL".into()))?;
            if status != "open" {
                return Err(MdmError::PolicyControl("policy case is resolved".into()));
            }
            let execution_role = row
                .get::<String>(8)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Unauthorized("execution role is NULL".into()))?;
            let bound_oid = row
                .get::<pg_sys::Oid>(9)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            if execution_role != selected.name || bound_oid != Some(catalog::outer_user_id()) {
                return Err(MdmError::Unauthorized(
                    "case is bound to another execution role".into(),
                ));
            }
            let old_queue = row
                .get::<String>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let old_due_at = row
                .get::<String>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let old_level = row
                .get::<i32>(5)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("escalation level is NULL".into()))?;
            let old_protected = row
                .get::<bool>(6)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("manual protection is NULL".into()))?;
            let old_revision = row
                .get::<i64>(7)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("action revision is NULL".into()))?;
            if old_revision != request.expected_action_revision {
                return Err(MdmError::PolicyControl(
                    "action revision changed concurrently".into(),
                ));
            }
            if request.assigned_queue.as_deref().is_some_and(str::is_empty) {
                return Err(MdmError::PolicyControl(
                    "assigned_queue must be a non-empty name".into(),
                ));
            }
            let due_text = request.due_at.as_ref().map(|value| value.to_iso_string());
            let changed = old_queue != request.assigned_queue
                || old_due_at != due_text
                || old_level != request.escalation_level
                || old_protected != request.manual_assignment_protected;
            let revision = if changed {
                old_revision
                    .checked_add(1)
                    .ok_or_else(|| MdmError::PolicyControl("action revision exhausted".into()))?
            } else {
                old_revision
            };
            let operation_id = operation(
                client,
                "policy_case_controls",
                Some(&entity_name),
                json!({"case_key": request.case_key, "changed": changed, "reason": request.reason}),
                &session.name,
                &selected.name,
            )?;
            if changed {
                client
                    .update(
                        "UPDATE mdm_steward.policy_cases_v1 SET assigned_queue = $2::pg_catalog.name, due_at = $3::timestamptz, escalation_level = $4, manual_assignment_protected = $5, action_revision = $6 WHERE case_key = $1",
                        None,
                        &[
                            request.case_key.into(),
                            request.assigned_queue.clone().into(),
                            request.due_at.into(),
                            request.escalation_level.into(),
                            request.manual_assignment_protected.into(),
                            revision.into(),
                        ],
                    )
                    .map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            Ok(JsonB(
                json!({"operation_id": operation_id, "action_revision": revision}),
            ))
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

fn receipt_from_row(row: &pgrx::spi::SpiTupleTable<'_>) -> Result<Receipt, MdmError> {
    Ok(Receipt {
        receipt_id: row
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("receipt ID is NULL".into()))?,
        outcome: row
            .get::<String>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("receipt outcome is NULL".into()))?,
        reason_code: row
            .get::<String>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or_default(),
        case_key: row
            .get::<i64>(4)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("receipt case key is NULL".into()))?,
        action_revision: row
            .get::<i64>(5)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("receipt action revision is NULL".into()))?,
        control: row
            .get::<JsonB>(6)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .map(|value| value.0),
        resulting_publication_revision: row
            .get::<i64>(7)
            .map_err(|error| MdmError::Spi(error.to_string()))?,
    })
}

fn receipt_select(
    client: &mut SpiClient<'_>,
    binding_id: &Uuid,
    request_key: &[u8],
) -> Result<Option<Receipt>, MdmError> {
    let rows = client
        .select(
            "SELECT receipt_id::text, outcome, reason_code, case_key, action_revision, control, resulting_publication_revision FROM mdm_steward.policy_receipts_v1 WHERE binding_id = $1::pg_catalog.uuid AND request_key = $2",
            Some(1),
            &[binding_id.to_string().into(), request_key.to_vec().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if rows.is_empty() {
        Ok(None)
    } else {
        Ok(Some(receipt_from_row(&rows.first())?))
    }
}

fn output_receipt(receipt: Receipt) -> Result<Value, MdmError> {
    Ok(json!({
        "receipt_id": receipt.receipt_id,
        "outcome": receipt.outcome,
        "reason_code": receipt.reason_code,
        "case_key": receipt.case_key,
        "action_revision": receipt.action_revision,
        "control": receipt.control,
        "resulting_publication_revision": receipt.resulting_publication_revision
    }))
}

fn replay_receipt(
    client: &mut SpiClient<'_>,
    request: &IntentRequest,
    digest: &[u8],
) -> Result<Option<Value>, MdmError> {
    let Some(receipt) = receipt_select(client, &request.binding_id, &request.request_key)? else {
        return Ok(None);
    };
    let existing_digest = client
        .select(
            "SELECT request_digest FROM mdm_steward.policy_receipts_v1 WHERE binding_id = $1::pg_catalog.uuid AND request_key = $2",
            Some(1),
            &[request.binding_id.to_string().into(), request.request_key.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .first()
        .get::<Vec<u8>>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("receipt digest is NULL".into()))?;
    if existing_digest == digest {
        Ok(Some(output_receipt(receipt)?))
    } else {
        Ok(Some(json!({
            "receipt_id": Value::Null,
            "outcome": "IDEMPOTENCY_CONFLICT",
            "reason_code": "REQUEST_KEY_BODY_MISMATCH",
            "case_key": request.case_key,
            "action_revision": request.expected_action_revision,
            "control": Value::Null,
            "resulting_publication_revision": Value::Null
        })))
    }
}

#[allow(clippy::too_many_arguments)]
fn terminal(
    client: &mut SpiClient<'_>,
    request: &IntentRequest,
    binding_version: i64,
    digest: &[u8],
    body: &Value,
    action_revision: i64,
    outcome: &str,
    reason_code: &str,
    control: Value,
) -> Result<Receipt, MdmError> {
    let inserted = client
        .update(
            "INSERT INTO mdm_steward.policy_receipts_v1 (binding_id, request_key, request_digest, request_body, actor, session_role_name, selected_role_name, binding_version, case_key, action, action_revision, outcome, reason_code, control) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5::pg_catalog.name, $6, $7, $8, $9, $10, $11, $12, $13, $14) ON CONFLICT (binding_id, request_key) DO NOTHING RETURNING receipt_id::text, outcome, reason_code, case_key, action_revision, control, resulting_publication_revision",
            Some(1),
            &[
                request.binding_id.to_string().into(),
                request.request_key.clone().into(),
                digest.to_vec().into(),
                JsonB(body.clone()).into(),
            request.actor.clone().into(),
            request.session_role_name.clone().into(),
            request.selected_role_name.clone().into(),
                binding_version.into(),
                request.case_key.into(),
                request.action.clone().into(),
                action_revision.into(),
                outcome.into(),
                reason_code.into(),
                JsonB(control).into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if inserted.is_empty() {
        receipt_select(client, &request.binding_id, &request.request_key)?.ok_or_else(|| {
            MdmError::PolicyIdempotency("receipt conflict did not expose an existing row".into())
        })
    } else {
        receipt_from_row(&inserted.first())
    }
}

fn submit_intent(request: &IntentRequest) -> Result<Value, MdmError> {
    if request.request_key.len() != 32
        || request.expected_evidence_basis_digest.len() != 32
        || request.expected_policy_digest.len() != 32
    {
        return Err(MdmError::PolicyIntent(
            "request_key and expected digests must be exactly 32 bytes".into(),
        ));
    }
    if request.case_key <= 0
        || request.expected_review_version <= 0
        || request.expected_definition_version <= 0
        || request.expected_publication_revision < 0
        || request.expected_stewardship_epoch < 0
        || request.expected_action_revision <= 0
    {
        return Err(MdmError::PolicyIntent(
            "freshness values are out of range".into(),
        ));
    }
    validate_reference(&request.policy_revision, "policy_revision")
        .map_err(MdmError::PolicyIntent)?;
    validate_reference(&request.evaluation_ref, "evaluation_ref")
        .map_err(MdmError::PolicyIntent)?;
    validate_reference(&request.work_ref, "work_ref").map_err(MdmError::PolicyIntent)?;
    let parsed_arguments = parse_intent_arguments(&request.action, &request.arguments.0)
        .map_err(MdmError::PolicyIntent)?;
    let body = PolicyIntentBody {
        action: request.action.clone(),
        arguments: request.arguments.0.clone(),
        binding_id: request.binding_id.to_string(),
        case_key: request.case_key,
        evaluation_ref: request.evaluation_ref.clone(),
        expected_action_revision: request.expected_action_revision,
        expected_definition_version: request.expected_definition_version,
        expected_evidence_basis_digest: request
            .expected_evidence_basis_digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        expected_policy_digest: request
            .expected_policy_digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        expected_publication_revision: request.expected_publication_revision,
        expected_review_version: request.expected_review_version,
        expected_stewardship_epoch: request.expected_stewardship_epoch,
        policy_revision: request.policy_revision.clone(),
        work_ref: request.work_ref.clone(),
    };
    let digest = intent_digest(&body);
    let body_value =
        serde_json::to_value(&body).map_err(|error| MdmError::PolicyIntent(error.to_string()))?;
    let helper = catalog::validate_helper_owner()?;
    let (_session, selected) = catalog::validate_caller(&helper)?;
    Spi::connect_mut(|client| {
        let binding = client
            .select(
                "SELECT b.entity_id::text, b.automation_role_name::text, COALESCE(r.automation_role_oid, 0), b.policy_digest, b.allowed_actions, b.allowed_queues::text[], b.max_due_interval::text, b.max_escalation_level, b.binding_version, CASE WHEN b.replaced_by IS NOT NULL THEN 'replaced' ELSE COALESCE(r.state, 'paused') END, COALESCE(r.database_oid, 0), COALESCE(r.automation_role_oid, 0), COALESCE(r.state, 'paused') FROM mdm_steward.policy_bindings_v1 b LEFT JOIN mdm_internal.policy_binding_runtime r ON r.binding_id = b.binding_id WHERE b.binding_id = $1::pg_catalog.uuid",
                Some(1),
                &[request.binding_id.to_string().into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if binding.is_empty() {
            return Err(MdmError::PolicyBinding("binding does not exist".into()));
        }
        let binding = binding.first();
        let principal_role = binding
            .get::<String>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("principal role is NULL".into()))?;
        let principal_oid = binding
            .get::<pg_sys::Oid>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("principal role OID is NULL".into()))?;
        if principal_role != selected.name || principal_oid != catalog::outer_user_id() {
            return Err(MdmError::Unauthorized(
                "caller does not match the bound automation role".into(),
            ));
        }
        if let Some(result) = replay_receipt(client, request, &digest)? {
            return Ok(result);
        }
        let binding = client
            .select(
                "SELECT b.entity_id::text, b.policy_digest, b.allowed_actions, b.allowed_queues::text[], b.max_due_interval::text, b.max_escalation_level, b.binding_version, CASE WHEN b.replaced_by IS NOT NULL THEN 'replaced' ELSE COALESCE(r.state, 'paused') END, COALESCE(r.database_oid, 0), COALESCE(r.automation_role_oid, 0), COALESCE(r.state, 'paused'), e.entity_name::text FROM mdm_steward.policy_bindings_v1 b JOIN mdm_internal.entities e USING (entity_id) LEFT JOIN mdm_internal.policy_binding_runtime r ON r.binding_id = b.binding_id WHERE b.binding_id = $1::pg_catalog.uuid FOR UPDATE OF b",
                Some(1),
                &[request.binding_id.to_string().into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let binding = binding.first();
        let _entity_id = binding
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("binding entity ID is NULL".into()))?;
        let policy_digest = binding
            .get::<Vec<u8>>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("binding policy digest is NULL".into()))?;
        let allowed_actions = binding
            .get::<Vec<String>>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or_default();
        let allowed_queues = binding
            .get::<Vec<String>>(4)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or_default();
        let max_due_interval = binding
            .get::<String>(5)
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let max_escalation_level = binding
            .get::<i32>(6)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("maximum escalation level is NULL".into()))?;
        let binding_version = binding
            .get::<i64>(7)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("binding version is NULL".into()))?;
        let state = binding
            .get::<String>(8)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("binding state is NULL".into()))?;
        let runtime_database = binding
            .get::<pg_sys::Oid>(9)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or(pg_sys::Oid::from(0));
        let runtime_role = binding
            .get::<pg_sys::Oid>(10)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or(pg_sys::Oid::from(0));
        let runtime_state = binding
            .get::<String>(11)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or_else(|| "paused".into());
        let binding_scope = binding
            .get::<String>(12)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("binding scope is NULL".into()))?;
        if let Some(result) = replay_receipt(client, request, &digest)? {
            return Ok(result);
        }
        let case = client
            .select(
                "SELECT entity_name::text, status, assigned_queue::text, due_at::text, escalation_level, manual_assignment_protected, opened_at::text, review_version, definition_version, publication_revision, stewardship_epoch, evidence_basis_digest, action_revision, pending_stewardship, permitted_actions FROM mdm_steward.policy_cases_v1 WHERE case_key = $1 FOR UPDATE",
                Some(1),
                &[request.case_key.into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let case = case.first();
        let case_entity = case
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyIntent("policy case does not exist".into()))?;
        let status = case
            .get::<String>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("case status is NULL".into()))?;
        let control = |queue: Option<String>, due: Option<String>, level: i32, protected: bool| json!({"assigned_queue": queue, "due_at": due, "escalation_level": level, "manual_assignment_protected": protected});
        let current_queue = case
            .get::<String>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let current_due = case
            .get::<String>(4)
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let current_level = case
            .get::<i32>(5)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("case escalation level is NULL".into()))?;
        let protected = case
            .get::<bool>(6)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("manual protection is NULL".into()))?;
        let opened_at = case
            .get::<String>(7)
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let review_version = case
            .get::<i64>(8)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("review version is NULL".into()))?;
        let definition_version = case
            .get::<i64>(9)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("definition version is NULL".into()))?;
        let publication_revision = case
            .get::<i64>(10)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("publication revision is NULL".into()))?;
        let stewardship_epoch = case
            .get::<i64>(11)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("stewardship epoch is NULL".into()))?;
        let evidence_digest = case
            .get::<Vec<u8>>(12)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("evidence digest is NULL".into()))?;
        let action_revision = case
            .get::<i64>(13)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("action revision is NULL".into()))?;
        let pending = case
            .get::<bool>(14)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("pending state is NULL".into()))?;
        let permitted = case
            .get::<Vec<String>>(15)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or_default();
        let current_control = control(
            current_queue.clone(),
            current_due.clone(),
            current_level,
            protected,
        );
        macro_rules! finish {
            ($outcome:expr, $reason:expr, $revision:expr, $control:expr $(,)?) => {
                terminal(
                    client,
                    &request,
                    binding_version,
                    &digest,
                    &body_value,
                    $revision,
                    $outcome,
                    $reason,
                    $control,
                )
            };
        }
        if case_entity != binding_scope {
            return Err(MdmError::PolicyIntent(
                "case does not belong to binding scope".into(),
            ));
        }
        if state == "replaced" {
            return output_receipt(finish!(
                "BINDING_REPLACED",
                "BINDING_REPLACED",
                action_revision,
                current_control,
            )?);
        }
        if state == "paused"
            || runtime_state != "active"
            || runtime_database != database_oid()?
            || runtime_role != principal_oid
        {
            return output_receipt(finish!(
                "BINDING_PAUSED",
                "BINDING_PAUSED",
                action_revision,
                current_control,
            )?);
        }
        if policy_digest != request.expected_policy_digest {
            return output_receipt(finish!(
                "POLICY_MISMATCH",
                "POLICY_DIGEST_MISMATCH",
                action_revision,
                current_control,
            )?);
        }
        if pending {
            return output_receipt(finish!(
                "PENDING_STEWARDSHIP",
                "PENDING_STEWARDSHIP",
                action_revision,
                current_control,
            )?);
        }
        if review_version != request.expected_review_version
            || definition_version != request.expected_definition_version
            || publication_revision != request.expected_publication_revision
            || stewardship_epoch != request.expected_stewardship_epoch
            || evidence_digest != request.expected_evidence_basis_digest
            || action_revision != request.expected_action_revision
        {
            return output_receipt(finish!(
                "STALE_CASE",
                "FRESHNESS_TOKEN_MISMATCH",
                action_revision,
                current_control,
            )?);
        }
        if status != "open" {
            return output_receipt(finish!(
                "CASE_CLOSED",
                "CASE_CLOSED",
                action_revision,
                current_control,
            )?);
        }
        if !permitted.contains(&request.action) || !allowed_actions.contains(&request.action) {
            return output_receipt(finish!(
                "ACTION_DENIED",
                "ACTION_NOT_ALLOWED",
                action_revision,
                current_control,
            )?);
        }
        let (next_queue, next_due, next_level) = match parsed_arguments {
            IntentArguments::AssignQueue(queue) => {
                if !queue_is_allowed(&queue, &allowed_queues) {
                    return output_receipt(finish!(
                        "ACTION_DENIED",
                        "QUEUE_NOT_ALLOWED",
                        action_revision,
                        current_control,
                    )?);
                }
                if protected {
                    return output_receipt(finish!(
                        "MANUAL_PROTECTION",
                        "MANUAL_ASSIGNMENT_PROTECTED",
                        action_revision,
                        current_control,
                    )?);
                }
                (Some(queue), current_due.clone(), current_level)
            }
            IntentArguments::SetDueAt(due) => {
                if opened_at.is_none() {
                    return output_receipt(finish!(
                        "OPENED_AT_UNKNOWN",
                        "OPENED_AT_UNKNOWN",
                        action_revision,
                        current_control,
                    )?);
                }
                let valid = client
                    .select(
                        "SELECT $1::timestamptz >= $2::timestamptz AND ($3::text IS NULL OR $1::timestamptz <= $2::timestamptz + $3::interval)",
                        Some(1),
                        &[due.clone().into(), opened_at.clone().into(), max_due_interval.clone().into()],
                    )
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .first()
                    .get::<bool>(1)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .unwrap_or(false);
                if !valid {
                    return output_receipt(finish!(
                        "LIMIT_EXCEEDED",
                        "DUE_AT_OUT_OF_BOUNDS",
                        action_revision,
                        current_control,
                    )?);
                }
                (current_queue.clone(), Some(due), current_level)
            }
            IntentArguments::Escalate(level) => {
                if current_due.is_none() {
                    return output_receipt(finish!(
                        "ACTION_DENIED",
                        "ESCALATION_NOT_DUE",
                        action_revision,
                        current_control,
                    )?);
                }
                let due = current_due.clone().unwrap_or_default();
                let due = client
                    .select(
                        "SELECT $1::timestamptz <= pg_catalog.statement_timestamp()",
                        Some(1),
                        &[due.into()],
                    )
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .first()
                    .get::<bool>(1)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .unwrap_or(false);
                if !due {
                    return output_receipt(finish!(
                        "ACTION_DENIED",
                        "ESCALATION_NOT_DUE",
                        action_revision,
                        current_control,
                    )?);
                }
                if level != current_level + 1 {
                    return output_receipt(finish!(
                        "STALE_CASE",
                        "ESCALATION_NOT_NEXT",
                        action_revision,
                        current_control,
                    )?);
                }
                if level > max_escalation_level {
                    return output_receipt(finish!(
                        "LIMIT_EXCEEDED",
                        "ESCALATION_LIMIT",
                        action_revision,
                        current_control,
                    )?);
                }
                (current_queue.clone(), current_due.clone(), level)
            }
        };
        let changed =
            next_queue != current_queue || next_due != current_due || next_level != current_level;
        let new_revision = if changed {
            action_revision
                .checked_add(1)
                .ok_or_else(|| MdmError::PolicyIntent("action revision exhausted".into()))?
        } else {
            action_revision
        };
        if changed {
            client
                .update(
                    "UPDATE mdm_steward.policy_cases_v1 SET assigned_queue = $2::pg_catalog.name, due_at = $3::timestamptz, escalation_level = $4, action_revision = $5 WHERE case_key = $1",
                    None,
                    &[request.case_key.into(), next_queue.clone().into(), next_due.clone().into(), next_level.into(), new_revision.into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
        }
        let outcome = if changed {
            "APPLIED_CONTROL"
        } else {
            "NO_CHANGE"
        };
        let reason = if changed {
            "CONTROL_APPLIED"
        } else {
            "CONTROL_UNCHANGED"
        };
        let final_control = control(next_queue, next_due, next_level, protected);
        output_receipt(finish!(outcome, reason, new_revision, final_control)?)
    })
}

#[pg_extern(
    name = "submit_policy_intent",
    requires = [persist_policy_intent],
    sql = "CREATE FUNCTION mdm_steward.submit_policy_intent(binding_id uuid, request_key bytea, case_key bigint, action text, arguments jsonb, expected_review_version bigint, expected_definition_version bigint, expected_publication_revision bigint, expected_stewardship_epoch bigint, expected_evidence_basis_digest bytea, expected_action_revision bigint, expected_policy_digest bytea, policy_revision text, evaluation_ref text, work_ref text) RETURNS TABLE (receipt_id uuid, outcome text, reason_code text, case_key bigint, action_revision bigint, control jsonb, resulting_publication_revision bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'submit_policy_intent_wrapper';"
)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub(crate) fn submit_policy_intent(
    binding_id: Uuid,
    request_key: Vec<u8>,
    case_key: i64,
    action: String,
    arguments: JsonB,
    expected_review_version: i64,
    expected_definition_version: i64,
    expected_publication_revision: i64,
    expected_stewardship_epoch: i64,
    expected_evidence_basis_digest: Vec<u8>,
    expected_action_revision: i64,
    expected_policy_digest: Vec<u8>,
    policy_revision: String,
    evaluation_ref: String,
    work_ref: String,
) -> TableIterator<
    'static,
    (
        name!(receipt_id, Option<Uuid>),
        name!(outcome, String),
        name!(reason_code, String),
        name!(case_key, i64),
        name!(action_revision, i64),
        name!(control, Option<JsonB>),
        name!(resulting_publication_revision, Option<i64>),
    ),
> {
    let result = (|| {
        let value = catalog::call_helper(
            "persist_policy_intent",
            IntentRequest {
                binding_id,
                request_key,
                case_key,
                action,
                arguments,
                expected_review_version,
                expected_definition_version,
                expected_publication_revision,
                expected_stewardship_epoch,
                expected_evidence_basis_digest,
                expected_action_revision,
                expected_policy_digest,
                policy_revision,
                evaluation_ref,
                work_ref,
                actor: current_user()?,
                session_role_name: session_user()?,
                selected_role_name: current_user()?,
            },
        )?;
        let control = value.0["control"].clone();
        Ok(vec![(
            value.0["receipt_id"].as_str().map(parse_uuid).transpose()?,
            value.0["outcome"].as_str().unwrap_or_default().to_owned(),
            value.0["reason_code"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            value.0["case_key"].as_i64().unwrap_or(case_key),
            value.0["action_revision"]
                .as_i64()
                .unwrap_or(expected_action_revision),
            (!control.is_null()).then_some(JsonB(control)),
            value.0["resulting_publication_revision"].as_i64(),
        )])
    })();
    match result {
        Ok(rows) => TableIterator::new(rows),
        Err(error) => crate::raise(error),
    }
}

#[pg_extern(
    name = "persist_policy_intent",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_policy_intent(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_policy_intent_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn persist_policy_intent(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only the public intent wrapper constructs this request.
        let request = unsafe { request.get::<IntentRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("intent request is required".into()))?;
        Ok(JsonB(submit_intent(request)?))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
