use pgrx::prelude::*;
use pgrx::{Internal, JsonB, Uuid};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::catalog;
use crate::constraint::{DecisionEdge, DecisionKind, validate_proposed_decision};
use crate::decision::{edge, next_version, validate_reason};
use crate::error::MdmError;

#[derive(Debug, Deserialize, Serialize)]
struct PersistedDecision {
    operation_id: String,
    decision_id: String,
    decision_version: i64,
    decision_epoch: i64,
}

struct DecideRequest {
    entity_name: String,
    left_source_record_id: Uuid,
    right_source_record_id: Uuid,
    decision: String,
    expected_version: i64,
    reason: String,
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

fn load_edges(
    client: &mut pgrx::spi::SpiClient<'_>,
    entity_id: &str,
) -> Result<Vec<DecisionEdge>, MdmError> {
    let rows = client
        .select(
            "SELECT decision_id::text, left_source_record_id::text, right_source_record_id::text, decision FROM mdm_internal.steward_decisions WHERE entity_id = $1::pg_catalog.uuid AND is_current ORDER BY decision_id",
            None,
            &[entity_id.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    rows.into_iter()
        .map(|row| {
            let decision_id = row
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("decision ID is NULL".into()))?;
            let left = row
                .get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("left source record ID is NULL".into()))?;
            let right = row
                .get::<String>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("right source record ID is NULL".into()))?;
            let decision = row
                .get::<String>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("decision is NULL".into()))?;
            Ok(edge(
                parse_uuid(&decision_id)?,
                parse_uuid(&left)?,
                parse_uuid(&right)?,
                DecisionKind::parse(&decision)?,
            ))
        })
        .collect()
}

#[pg_extern(
    name = "decide",
    requires = [persist_decision],
    sql = "CREATE FUNCTION mdm_steward.decide(entity_name text, left_source_record_id uuid, right_source_record_id uuid, decision text, expected_version bigint, reason text) RETURNS TABLE (operation_id uuid, decision_id uuid, decision_version bigint, decision_epoch bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'decide_wrapper';"
)]
pub(crate) fn decide(
    entity_name: String,
    left_source_record_id: Uuid,
    right_source_record_id: Uuid,
    decision: String,
    expected_version: i64,
    reason: String,
) -> TableIterator<
    'static,
    (
        name!(operation_id, Uuid),
        name!(decision_id, Uuid),
        name!(decision_version, i64),
        name!(decision_epoch, i64),
    ),
> {
    let result = (|| {
        let value = catalog::call_helper(
            "persist_decision",
            DecideRequest {
                entity_name,
                left_source_record_id,
                right_source_record_id,
                decision,
                expected_version,
                reason,
            },
        )?;
        let result: PersistedDecision = serde_json::from_value(value.0)
            .map_err(|error| MdmError::OperationState(error.to_string()))?;
        Ok(vec![(
            parse_uuid(&result.operation_id)?,
            parse_uuid(&result.decision_id)?,
            result.decision_version,
            result.decision_epoch,
        )])
    })();
    match result {
        Ok(rows) => TableIterator::new(rows),
        Err(error) => crate::raise(error),
    }
}

#[pg_extern(
    name = "persist_decision",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_decision(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_decision_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn persist_decision(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only the public decide wrapper constructs this request.
        let request = unsafe { request.get::<DecideRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("decision request is NULL".into()))?;
        validate_reason(&request.reason)?;
        let decision = DecisionKind::parse(&request.decision)?;
        let helper_owner = catalog::validate_helper_owner()?;
        let (session, selected) = catalog::validate_caller(&helper_owner)?;

        Spi::connect_mut(|client| {
            let entity = client
                .select(
                    "SELECT e.entity_id::text, e.execution_role_name, e.decision_epoch, e.publication_revision, b.role_oid FROM mdm_internal.entities e LEFT JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id WHERE e.entity_name = $1::pg_catalog.name FOR UPDATE OF e",
                    Some(1),
                    &[request.entity_name.clone().into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            if entity.is_empty() {
                return Err(MdmError::DefinitionInvalid(format!(
                    "entity {} does not exist",
                    request.entity_name
                )));
            }
            let row = entity.first();
            let entity_id = row
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("entity ID is NULL".into()))?;
            let execution_role = row
                .get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("execution role is NULL".into()))?;
            let decision_epoch = row
                .get::<i64>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("decision epoch is NULL".into()))?;
            let publication_revision = row
                .get::<i64>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("publication revision is NULL".into()))?;
            let bound_oid = row
                .get::<pg_sys::Oid>(5)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            if execution_role != selected.name || bound_oid != Some(catalog::outer_user_id()) {
                return Err(MdmError::Unauthorized(
                    "entity is bound to another execution role".into(),
                ));
            }
            let closure_limit = client
                .select(
                    "SELECT COALESCE((d.expanded_definition->'limits'->>'max_decision_closure')::bigint, 10000) FROM mdm_internal.definitions d WHERE d.entity_id = $1::pg_catalog.uuid AND d.definition_version = (SELECT desired_version FROM mdm_internal.entities WHERE entity_id = $1::pg_catalog.uuid)",
                    Some(1),
                    &[entity_id.clone().into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .first()
                .get::<i64>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(10_000);

            let records = client
                .select(
                    "SELECT source_record_id::text, source_record_key, active FROM mdm_internal.source_records WHERE entity_id = $1::pg_catalog.uuid AND source_record_id IN ($2::pg_catalog.uuid, $3::pg_catalog.uuid) FOR SHARE",
                    Some(2),
                    &[
                        entity_id.clone().into(),
                        request.left_source_record_id.into(),
                        request.right_source_record_id.into(),
                    ],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            if records.len() != 2 {
                return Err(MdmError::DecisionInvalid(
                    "both source records must belong to the entity".into(),
                ));
            }
            let records = records.into_iter().collect::<Vec<_>>();
            let mut endpoints = records
                .iter()
                .map(|record| {
                    Ok::<_, MdmError>((
                        parse_uuid(
                            &record
                                .get::<String>(1)
                                .map_err(|error| MdmError::Spi(error.to_string()))?
                                .ok_or_else(|| MdmError::Spi("source record ID is NULL".into()))?,
                        )?,
                        record
                            .get::<Vec<u8>>(2)
                            .map_err(|error| MdmError::Spi(error.to_string()))?
                            .ok_or_else(|| MdmError::Spi("source record key is NULL".into()))?,
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            endpoints
                .sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
            if endpoints[0].0 == endpoints[1].0 {
                return Err(MdmError::DecisionInvalid(
                    "source records must be distinct".into(),
                ));
            }
            let left = endpoints[0].0;
            let right = endpoints[1].0;

            let current = client
                .select(
                    "SELECT decision_id::text, decision_version FROM mdm_internal.steward_decisions WHERE entity_id = $1::pg_catalog.uuid AND left_source_record_id = $2::pg_catalog.uuid AND right_source_record_id = $3::pg_catalog.uuid AND is_current FOR UPDATE",
                    Some(1),
                    &[entity_id.clone().into(), left.into(), right.into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let (old_id, old_version) = if current.is_empty() {
                (None, None)
            } else {
                let current_row = current.first();
                (
                    Some(parse_uuid(
                        &current_row
                            .get::<String>(1)
                            .map_err(|error| MdmError::Spi(error.to_string()))?
                            .ok_or_else(|| MdmError::Spi("decision ID is NULL".into()))?,
                    )?),
                    current_row
                        .get::<i64>(2)
                        .map_err(|error| MdmError::Spi(error.to_string()))?,
                )
            };
            let version = next_version(old_version, request.expected_version)?;
            let mut existing = load_edges(client, &entity_id)?;
            let proposed_id = old_id.unwrap_or_else(|| Uuid::from_bytes([0; 16]));
            let proposed = edge(proposed_id, left, right, decision);
            validate_proposed_decision(&existing, &proposed, closure_limit)?;

            let operation = client
                .update(
                    "INSERT INTO mdm_internal.operations (operation_kind, entity_name, status, outcome, actor_name, actor_role_name) VALUES ('steward_decide', $1::pg_catalog.name, 'running', $2, $3, $4) RETURNING operation_id::text",
                    Some(1),
                    &[
                        request.entity_name.clone().into(),
                        JsonB(json!({
                            "decision": decision.as_str(),
                            "decision_version": version,
                            "base_publication_revision": publication_revision
                        }))
                        .into(),
                        session.name.clone().into(),
                        selected.name.clone().into(),
                    ],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let operation_id = operation
                .first()
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::OperationState("operation ID is NULL".into()))?;
            let new_epoch = decision_epoch.checked_add(1).ok_or_else(|| {
                MdmError::DecisionVersionConflict("decision epoch exhausted".into())
            })?;
            if let Some(old_id) = old_id {
                client
                    .update(
                        "UPDATE mdm_internal.steward_decisions SET is_current = false WHERE decision_id = $1::pg_catalog.uuid AND is_current",
                        None,
                        &[old_id.into()],
                    )
                    .map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            let inserted = client
                .update(
                    "INSERT INTO mdm_internal.steward_decisions (entity_id, left_source_record_id, right_source_record_id, decision, decision_version, reason, created_by_name, created_as_role_name, operation_id, base_publication_revision, decision_epoch, supersedes) VALUES ($1::pg_catalog.uuid, $2::pg_catalog.uuid, $3::pg_catalog.uuid, $4, $5, $6, $7, $8, $9::pg_catalog.uuid, $10, $11, $12::pg_catalog.uuid) RETURNING decision_id::text",
                    Some(1),
                    &[
                        entity_id.clone().into(),
                        left.into(),
                        right.into(),
                        decision.as_str().into(),
                        version.into(),
                        request.reason.clone().into(),
                        session.name.clone().into(),
                        selected.name.clone().into(),
                        operation_id.clone().into(),
                        publication_revision.into(),
                        new_epoch.into(),
                        old_id.into(),
                    ],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let decision_id = inserted
                .first()
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::OperationState("decision ID is NULL".into()))?;
            client
                .update(
                    "UPDATE mdm_internal.entities SET decision_epoch = $2 WHERE entity_id = $1::pg_catalog.uuid",
                    None,
                    &[entity_id.into(), new_epoch.into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            client
                .update(
                    "UPDATE mdm_internal.operations SET status = 'succeeded', result_code = 'MDM_OK', outcome = outcome || $2, completed_at = pg_catalog.statement_timestamp() WHERE operation_id = $1::pg_catalog.uuid AND status = 'running'",
                    None,
                    &[
                        operation_id.clone().into(),
                        JsonB(json!({"decision_id": decision_id, "decision_epoch": new_epoch})).into(),
                    ],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            existing.clear();
            Ok(JsonB(json!({
                "operation_id": operation_id,
                "decision_id": decision_id,
                "decision_version": version,
                "decision_epoch": new_epoch
            })))
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[derive(Debug, Deserialize, Serialize)]
struct PersistedOverride {
    operation_id: String,
    override_id: String,
    override_version: i64,
    decision_epoch: i64,
}

struct OverrideRequest {
    entity_name: String,
    anchor_source_record_id: Uuid,
    field_name: String,
    value: Option<JsonB>,
    expected_version: i64,
    reason: String,
}

fn call_override(request: OverrideRequest) -> Result<PersistedOverride, MdmError> {
    let value = catalog::call_helper("persist_golden_override", request)?;
    serde_json::from_value(value.0).map_err(|error| MdmError::OperationState(error.to_string()))
}

#[pg_extern(
    name = "override_golden",
    requires = [persist_golden_override],
    sql = "CREATE FUNCTION mdm_steward.override_golden(entity_name text, anchor_source_record_id uuid, field_name text, value jsonb, expected_version bigint, reason text) RETURNS TABLE (operation_id uuid, override_id uuid, override_version bigint, decision_epoch bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'override_golden_wrapper';"
)]
pub(crate) fn override_golden(
    entity_name: String,
    anchor_source_record_id: Uuid,
    field_name: String,
    value: JsonB,
    expected_version: i64,
    reason: String,
) -> TableIterator<
    'static,
    (
        name!(operation_id, Uuid),
        name!(override_id, Uuid),
        name!(override_version, i64),
        name!(decision_epoch, i64),
    ),
> {
    let result = (|| {
        let result = call_override(OverrideRequest {
            entity_name,
            anchor_source_record_id,
            field_name,
            value: Some(value),
            expected_version,
            reason,
        })?;
        Ok(vec![(
            parse_uuid(&result.operation_id)?,
            parse_uuid(&result.override_id)?,
            result.override_version,
            result.decision_epoch,
        )])
    })();
    match result {
        Ok(rows) => TableIterator::new(rows),
        Err(error) => crate::raise(error),
    }
}

#[pg_extern(
    name = "clear_golden_override",
    requires = [persist_golden_override, override_golden],
    sql = "CREATE FUNCTION mdm_steward.clear_golden_override(entity_name text, anchor_source_record_id uuid, field_name text, expected_version bigint, reason text) RETURNS TABLE (operation_id uuid, override_id uuid, override_version bigint, decision_epoch bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'clear_golden_override_wrapper';"
)]
pub(crate) fn clear_golden_override(
    entity_name: String,
    anchor_source_record_id: Uuid,
    field_name: String,
    expected_version: i64,
    reason: String,
) -> TableIterator<
    'static,
    (
        name!(operation_id, Uuid),
        name!(override_id, Uuid),
        name!(override_version, i64),
        name!(decision_epoch, i64),
    ),
> {
    let result = (|| {
        let result = call_override(OverrideRequest {
            entity_name,
            anchor_source_record_id,
            field_name,
            value: None,
            expected_version,
            reason,
        })?;
        Ok(vec![(
            parse_uuid(&result.operation_id)?,
            parse_uuid(&result.override_id)?,
            result.override_version,
            result.decision_epoch,
        )])
    })();
    match result {
        Ok(rows) => TableIterator::new(rows),
        Err(error) => crate::raise(error),
    }
}

#[pg_extern(
    name = "persist_golden_override",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_golden_override(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_golden_override_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn persist_golden_override(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only the public override wrappers construct this request.
        let request = unsafe { request.get::<OverrideRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("golden override request is NULL".into()))?;
        validate_reason(&request.reason)?;
        if request.expected_version < 0 {
            return Err(MdmError::GoldenInvalid(
                "expected_version must be non-negative".into(),
            ));
        }
        if request.field_name.is_empty() {
            return Err(MdmError::GoldenInvalid(
                "field_name must not be empty".into(),
            ));
        }
        if request
            .value
            .as_ref()
            .is_some_and(|value| value.0.is_null())
        {
            return Err(MdmError::GoldenInvalid(
                "JSON null is not a golden value".into(),
            ));
        }
        let helper_owner = catalog::validate_helper_owner()?;
        let (session, selected) = catalog::validate_caller(&helper_owner)?;
        Spi::connect_mut(|client| {
            let entity = client
                .select(
                    "SELECT e.entity_id::text, e.execution_role_name, e.decision_epoch, e.publication_revision, b.role_oid FROM mdm_internal.entities e LEFT JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id WHERE e.entity_name = $1::pg_catalog.name FOR UPDATE OF e",
                    Some(1),
                    &[request.entity_name.clone().into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            if entity.is_empty() {
                return Err(MdmError::DefinitionInvalid(format!(
                    "entity {} does not exist",
                    request.entity_name
                )));
            }
            let row = entity.first();
            let entity_id = row
                .get::<String>(1)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| MdmError::Spi("entity ID is NULL".into()))?;
            let execution_role = row
                .get::<String>(2)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| MdmError::Spi("execution role is NULL".into()))?;
            let decision_epoch = row
                .get::<i64>(3)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| MdmError::Spi("decision epoch is NULL".into()))?;
            let publication_revision = row
                .get::<i64>(4)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| MdmError::Spi("publication revision is NULL".into()))?;
            let bound_oid = row
                .get::<pg_sys::Oid>(5)
                .map_err(|e| MdmError::Spi(e.to_string()))?;
            if execution_role != selected.name || bound_oid != Some(catalog::outer_user_id()) {
                return Err(MdmError::Unauthorized(
                    "entity is bound to another execution role".into(),
                ));
            }
            let field = client.select(
                "SELECT f->>'type' FROM mdm_internal.definitions d, pg_catalog.jsonb_array_elements(d.expanded_definition->'fields') f WHERE d.entity_id = $1::pg_catalog.uuid AND d.definition_version = (SELECT desired_version FROM mdm_internal.entities WHERE entity_id = $1::pg_catalog.uuid) AND f->>'name' = $2",
                Some(1), &[entity_id.clone().into(), request.field_name.clone().into()]
            ).map_err(|e| MdmError::Spi(e.to_string()))?;
            let value_type_name = field
                .first()
                .get::<String>(1)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| {
                    MdmError::GoldenInvalid(format!("unknown golden field {}", request.field_name))
                })?;
            let anchor = client.select(
                "SELECT 1 FROM mdm_internal.source_records WHERE entity_id = $1::pg_catalog.uuid AND source_record_id = $2::pg_catalog.uuid",
                Some(1), &[entity_id.clone().into(), request.anchor_source_record_id.into()]
            ).map_err(|e| MdmError::Spi(e.to_string()))?;
            if anchor.is_empty() {
                return Err(MdmError::GoldenInvalid(
                    "anchor source record does not belong to the entity".into(),
                ));
            }
            let current = client.select(
                "SELECT override_id::text, override_version FROM mdm_internal.golden_override_directives WHERE entity_id = $1::pg_catalog.uuid AND field_name = $2::pg_catalog.name AND anchor_source_record_id = $3::pg_catalog.uuid AND is_current FOR UPDATE",
                Some(1), &[entity_id.clone().into(), request.field_name.clone().into(), request.anchor_source_record_id.into()]
            ).map_err(|e| MdmError::Spi(e.to_string()))?;
            let (old_id, old_version) = if current.is_empty() {
                (None, 0)
            } else {
                let row = current.first();
                (
                    Some(parse_uuid(
                        &row.get::<String>(1)
                            .map_err(|e| MdmError::Spi(e.to_string()))?
                            .ok_or_else(|| MdmError::Spi("override ID is NULL".into()))?,
                    )?),
                    row.get::<i64>(2)
                        .map_err(|e| MdmError::Spi(e.to_string()))?
                        .ok_or_else(|| MdmError::Spi("override version is NULL".into()))?,
                )
            };
            if old_version != request.expected_version {
                return Err(MdmError::GoldenConflict(format!(
                    "expected override version {}, current version {}",
                    request.expected_version, old_version
                )));
            }
            let version = old_version
                .checked_add(1)
                .ok_or_else(|| MdmError::GoldenInvalid("override version exhausted".into()))?;
            let operation = client.update(
                "INSERT INTO mdm_internal.operations (operation_kind, entity_name, status, outcome, actor_name, actor_role_name) VALUES ('golden_override', $1::pg_catalog.name, 'running', $2, $3, $4) RETURNING operation_id::text",
                Some(1), &[request.entity_name.clone().into(), JsonB(json!({"field": request.field_name, "action": if request.value.is_some() {"SET"} else {"CLEAR"}, "base_publication_revision": publication_revision})).into(), session.name.clone().into(), selected.name.clone().into()]
            ).map_err(|e| MdmError::Spi(e.to_string()))?;
            let operation_id = operation
                .first()
                .get::<String>(1)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| MdmError::OperationState("operation ID is NULL".into()))?;
            let new_epoch = decision_epoch
                .checked_add(1)
                .ok_or_else(|| MdmError::GoldenInvalid("decision epoch exhausted".into()))?;
            if let Some(old_id) = old_id {
                client.update("UPDATE mdm_internal.golden_override_directives SET is_current = false WHERE override_id = $1::pg_catalog.uuid AND is_current", None, &[old_id.into()]).map_err(|e| MdmError::Spi(e.to_string()))?;
            }
            let action = if request.value.is_some() {
                "SET"
            } else {
                "CLEAR"
            };
            let value = request.value.as_ref().map(|value| JsonB(value.0.clone()));
            let inserted = client.update(
                "INSERT INTO mdm_internal.golden_override_directives (entity_id, field_name, anchor_source_record_id, action, value, value_type_name, override_version, reason, created_by_name, created_as_role_name, operation_id, base_publication_revision, decision_epoch, supersedes) VALUES ($1::pg_catalog.uuid, $2::pg_catalog.name, $3::pg_catalog.uuid, $4, $5, $6, $7, $8, $9, $10, $11::pg_catalog.uuid, $12, $13, $14::pg_catalog.uuid) RETURNING override_id::text",
                Some(1), &[entity_id.clone().into(), request.field_name.clone().into(), request.anchor_source_record_id.into(), action.into(), value.into(), if request.value.is_some() { Some(value_type_name).into() } else { Option::<String>::None.into() }, version.into(), request.reason.clone().into(), session.name.clone().into(), selected.name.clone().into(), operation_id.clone().into(), publication_revision.into(), new_epoch.into(), old_id.into()]
            ).map_err(|e| MdmError::Spi(e.to_string()))?;
            let override_id = inserted
                .first()
                .get::<String>(1)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| MdmError::OperationState("override ID is NULL".into()))?;
            client.update("UPDATE mdm_internal.entities SET decision_epoch = $2 WHERE entity_id = $1::pg_catalog.uuid", None, &[entity_id.into(), new_epoch.into()]).map_err(|e| MdmError::Spi(e.to_string()))?;
            client.update("UPDATE mdm_internal.operations SET status = 'succeeded', result_code = 'MDM_OK', outcome = outcome || $2, completed_at = pg_catalog.statement_timestamp() WHERE operation_id = $1::pg_catalog.uuid AND status = 'running'", None, &[operation_id.clone().into(), JsonB(json!({"override_id": override_id, "decision_epoch": new_epoch})).into()]).map_err(|e| MdmError::Spi(e.to_string()))?;
            Ok(JsonB(
                json!({"operation_id": operation_id, "override_id": override_id, "override_version": version, "decision_epoch": new_epoch}),
            ))
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
