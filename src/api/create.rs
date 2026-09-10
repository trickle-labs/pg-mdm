use pgrx::prelude::*;
use pgrx::spi::SpiClient;
use pgrx::{Internal, JsonB, Uuid};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog;
use crate::definition::canonical::canonical_definition;
use crate::definition::parse_entity;
use crate::definition::validate::{PreparedDefinition, digest_hex, prepare};
use crate::error::MdmError;
use crate::graph_spec;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CreateResult {
    operation_id: String,
    entity_name: String,
    desired_version: i64,
    changed: bool,
    definition_digest: String,
    artifact_digest: String,
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

struct CreateRequest {
    prepared: PreparedDefinition,
    expected_version: Option<i64>,
    comment: Option<String>,
}

fn call_persist(request: CreateRequest) -> Result<CreateResult, MdmError> {
    let result = catalog::call_helper("persist_entity", request)?;
    serde_json::from_value(result.0).map_err(|error| MdmError::OperationState(error.to_string()))
}

#[allow(clippy::type_complexity)]
#[pg_extern(
    name = "create",
    requires = [persist_entity],
    sql = "CREATE FUNCTION mdm.create(definition jsonb, expected_version bigint DEFAULT NULL, comment text DEFAULT NULL) RETURNS TABLE (operation_id uuid, entity_name text, desired_version bigint, changed boolean, definition_digest bytea, artifact_digest bytea) LANGUAGE c AS 'MODULE_PATHNAME', 'create_wrapper';"
)]
pub(crate) fn create(
    definition: JsonB,
    expected_version: Option<i64>,
    comment: Option<String>,
) -> TableIterator<
    'static,
    (
        name!(operation_id, Uuid),
        name!(entity_name, String),
        name!(desired_version, i64),
        name!(changed, bool),
        name!(definition_digest, Vec<u8>),
        name!(artifact_digest, Vec<u8>),
    ),
> {
    let result = (|| {
        let user_definition = canonical_definition(definition.0);
        let prepared = prepare(user_definition.clone())?;
        let result = call_persist(CreateRequest {
            prepared,
            expected_version,
            comment,
        })?;
        Ok(vec![(
            parse_uuid(&result.operation_id)?,
            result.entity_name,
            result.desired_version,
            result.changed,
            decode_hex(&result.definition_digest)?,
            decode_hex(&result.artifact_digest)?,
        )])
    })();
    match result {
        Ok(rows) => TableIterator::new(rows),
        Err(error) => crate::raise(error),
    }
}

fn decode_hex(value: &str) -> Result<Vec<u8>, MdmError> {
    if !value.len().is_multiple_of(2) {
        return Err(MdmError::OperationState("digest has odd length".into()));
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|error| MdmError::OperationState(error.to_string()))
        })
        .collect()
}

fn helper_result(
    operation_id: String,
    entity_name: String,
    desired_version: i64,
    changed: bool,
    prepared: &PreparedDefinition,
) -> JsonB {
    JsonB(json!({
        "operation_id": operation_id,
        "entity_name": entity_name,
        "desired_version": desired_version,
        "changed": changed,
        "definition_digest": digest_hex(&prepared.definition_digest),
        "artifact_digest": digest_hex(&prepared.artifact_digest)
    }))
}

fn fetch_operation_id(
    client: &mut SpiClient<'_>,
    entity_name: &str,
    outcome: JsonB,
    actor_name: &str,
    actor_role: &str,
) -> Result<String, MdmError> {
    let rows = client
        .update(
            "INSERT INTO mdm_internal.operations (operation_kind, entity_name, status, outcome, actor_name, actor_role_name) VALUES ('create', $1, 'running', $2, $3, $4) RETURNING operation_id::text",
            Some(1),
            &[entity_name.into(), outcome.into(), actor_name.into(), actor_role.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    rows.first()
        .get::<String>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::OperationState("create operation ID is NULL".into()))
}

fn complete_operation(client: &mut SpiClient<'_>, id: &str) -> Result<(), MdmError> {
    let rows = client
        .update(
            "UPDATE mdm_internal.operations SET status = 'succeeded', result_code = 'MDM_OK', completed_at = pg_catalog.statement_timestamp() WHERE operation_id = $1::pg_catalog.uuid AND status = 'running' RETURNING operation_id",
            Some(1),
            &[id.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if rows.is_empty() {
        return Err(MdmError::OperationState(
            "create operation did not complete".into(),
        ));
    }
    Ok(())
}

fn persist(
    prepared: PreparedDefinition,
    entity: crate::definition::Entity,
    expected_version: Option<i64>,
    comment: Option<String>,
    session_name: String,
    selected: String,
) -> Result<(String, i64, bool), MdmError> {
    let name = entity.name.clone();
    let role = entity
        .execution_role
        .clone()
        .ok_or_else(|| MdmError::Unauthorized("execution role is missing".into()))?;
    if role != selected {
        return Err(MdmError::Unauthorized(
            "execution role is not the selected role".into(),
        ));
    }
    let capabilities = crate::integration::integration_capabilities()?;
    let outcome = JsonB(
        json!({"definition_digest": digest_hex(&prepared.definition_digest), "artifact_digest": digest_hex(&prepared.artifact_digest), "graph_executable": false, "capabilities": capabilities}),
    );
    let mut operation_id = String::new();
    let mut version = 1_i64;
    let mut changed = true;
    Spi::connect_mut(|client| {
        let existing = client.update(
            "SELECT e.entity_id::text, e.desired_version, e.execution_role_name, b.role_oid FROM mdm_internal.entities e LEFT JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id WHERE e.entity_name = $1::pg_catalog.name FOR UPDATE OF e",
            Some(1), &[name.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
        let entity_id: String;
        if !existing.is_empty() {
            let row = existing.first();
            entity_id = row
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("entity ID is NULL".into()))?;
            let current_version = row
                .get::<i64>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("desired version is NULL".into()))?;
            let current_role = row
                .get::<String>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("execution role is NULL".into()))?;
            let bound_oid = row
                .get::<pg_sys::Oid>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            if current_role != selected || bound_oid != Some(selected_oid()) {
                return Err(MdmError::Unauthorized(
                    "existing entity is bound to another execution role".into(),
                ));
            }
            if expected_version != Some(current_version) {
                return Err(MdmError::VersionConflict(format!(
                    "expected version {:?}, current version {current_version}",
                    expected_version
                )));
            }
            let same = client.select(
                "SELECT definition_digest, (SELECT artifact_digest FROM mdm_internal.definition_artifacts a WHERE a.entity_id = d.entity_id AND a.definition_version = d.definition_version ORDER BY a.artifact_id DESC LIMIT 1) FROM mdm_internal.definitions d WHERE d.entity_id = $1::pg_catalog.uuid AND d.definition_version = $2",
                Some(1), &[entity_id.clone().into(), current_version.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            let digest_current = if same.is_empty() {
                None
            } else {
                same.first().get::<Vec<u8>>(1).ok().flatten()
            };
            if digest_current.as_deref() == Some(prepared.definition_digest.as_slice()) {
                changed = false;
                version = current_version;
            } else {
                version = current_version + 1;
            }
        } else {
            if expected_version.is_some() {
                return Err(MdmError::VersionConflict(
                    "new entities require expected_version NULL".into(),
                ));
            }
            let rows = client.update(
                "INSERT INTO mdm_internal.entities (entity_name, execution_role_name, desired_version, created_by_name) VALUES ($1, $2, 1, $3) RETURNING entity_id::text",
                Some(1), &[name.clone().into(), selected.clone().into(), session_name.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            entity_id = rows
                .first()
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("entity ID is NULL".into()))?;
            client.update("INSERT INTO mdm_internal.execution_role_bindings (entity_id, role_oid) VALUES ($1::pg_catalog.uuid, $2)", None, &[entity_id.clone().into(), selected_oid().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
        }

        operation_id = fetch_operation_id(
            client,
            &name,
            JsonB(outcome.0.clone()),
            &session_name,
            &selected,
        )?;
        if !changed {
            for source in &prepared.sources {
                let current = client
                    .select("SELECT source_identity_id::text, relation_name, key_contract, identity_digest FROM mdm_internal.source_identities WHERE entity_id = $1::pg_catalog.uuid AND source_name = $2::pg_catalog.name", Some(1), &[entity_id.clone().into(), source.name.clone().into()])
                    .map_err(|error| MdmError::Spi(error.to_string()))?;
                if current.is_empty() {
                    return Err(MdmError::SourceInvalid(format!(
                        "source identity {} is missing",
                        source.name
                    )));
                }
                let row = current.first();
                let identity_id = row
                    .get::<String>(1)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("source identity ID is NULL".into()))?;
                let existing_relation = row
                    .get::<String>(2)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("source relation is NULL".into()))?;
                let existing_contract = row
                    .get::<JsonB>(3)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .map(|value| value.0)
                    .ok_or_else(|| MdmError::Spi("source contract is NULL".into()))?;
                let existing_digest = row
                    .get::<Vec<u8>>(4)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("source digest is NULL".into()))?;
                if Value::String(existing_relation) != source.key_contract["relation_name"]
                    || existing_contract != source.key_contract
                    || existing_digest != source.identity_digest
                {
                    return Err(MdmError::SourceInvalid(format!(
                        "source identity {} changed",
                        source.name
                    )));
                }
                let binding = client
                    .select("SELECT relation_oid, binding_fingerprint FROM mdm_internal.source_bindings WHERE source_identity_id = $1::pg_catalog.uuid", Some(1), &[identity_id.into()])
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .first();
                if binding.is_empty()
                    || binding.get::<pg_sys::Oid>(1).ok().flatten()
                        != Some(pg_sys::Oid::from(source.relation_oid))
                    || binding
                        .get::<JsonB>(2)
                        .ok()
                        .flatten()
                        .map(|value| value.0)
                        .as_ref()
                        != Some(&source.binding_fingerprint)
                {
                    return Err(MdmError::SourceInvalid(format!(
                        "source {} binding changed; administrator rebind is required",
                        source.name
                    )));
                }
            }
        }
        if changed {
            for output in &prepared.output_names {
                let objects = client
                    .select(
                        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'mdm_out' AND c.relname = $1)",
                        Some(1),
                        &[output.name.clone().into()],
                    )
                    .map_err(|error| MdmError::Spi(error.to_string()))?;
                let exists = objects
                    .first()
                    .get::<bool>(1)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .unwrap_or(false);
                if exists {
                    return Err(MdmError::OutputNameConflict(output.name.clone()));
                }
                let reserved = client
                    .select(
                        "SELECT entity_id::text FROM mdm_internal.output_names WHERE lower(output_name::text) = lower($1)",
                        Some(1),
                        &[output.name.clone().into()],
                    )
                    .map_err(|error| MdmError::Spi(error.to_string()))?;
                if !reserved.is_empty() {
                    let owner = reserved
                        .first()
                        .get::<String>(1)
                        .map_err(|error| MdmError::Spi(error.to_string()))?;
                    if owner.as_deref() != Some(entity_id.as_str()) {
                        return Err(MdmError::OutputNameConflict(output.name.clone()));
                    }
                } else {
                    client
                        .update("INSERT INTO mdm_internal.output_names (output_name, entity_id, output_kind) VALUES ($1, $2::pg_catalog.uuid, $3)", None, &[output.name.clone().into(), entity_id.clone().into(), output.kind.clone().into()])
                        .map_err(|error| MdmError::Spi(error.to_string()))?;
                }
            }
            for source in &prepared.sources {
                let current = client.select("SELECT source_identity_id::text, relation_name, key_contract, identity_digest FROM mdm_internal.source_identities WHERE entity_id = $1::pg_catalog.uuid AND source_name = $2::pg_catalog.name FOR UPDATE", Some(1), &[entity_id.clone().into(), source.name.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
                let source_id = if !current.is_empty() {
                    let row = current.first();
                    let existing_relation = row
                        .get::<String>(2)
                        .map_err(|error| MdmError::Spi(error.to_string()))?
                        .ok_or_else(|| MdmError::Spi("source relation is NULL".into()))?;
                    let existing_contract = row
                        .get::<JsonB>(3)
                        .map_err(|error| MdmError::Spi(error.to_string()))?
                        .map(|value| value.0)
                        .ok_or_else(|| MdmError::Spi("source contract is NULL".into()))?;
                    let existing_digest = row
                        .get::<Vec<u8>>(4)
                        .map_err(|error| MdmError::Spi(error.to_string()))?
                        .ok_or_else(|| MdmError::Spi("source digest is NULL".into()))?;
                    if Value::String(existing_relation) != source.key_contract["relation_name"]
                        || existing_contract != source.key_contract
                        || existing_digest != source.identity_digest
                    {
                        return Err(MdmError::SourceInvalid(format!(
                            "source identity {} changed",
                            source.name
                        )));
                    }
                    let source_id = row
                        .get::<String>(1)
                        .map_err(|error| MdmError::Spi(error.to_string()))?
                        .ok_or_else(|| MdmError::Spi("source identity ID is NULL".into()))?;
                    let binding = client.select("SELECT relation_oid, binding_fingerprint FROM mdm_internal.source_bindings WHERE source_identity_id = $1::pg_catalog.uuid", Some(1), &[source_id.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
                    let binding = binding.first();
                    let bound_oid = if binding.is_empty() {
                        None
                    } else {
                        binding.get::<pg_sys::Oid>(1).ok().flatten()
                    };
                    if bound_oid != Some(pg_sys::Oid::from(source.relation_oid)) {
                        return Err(MdmError::SourceInvalid(format!(
                            "source {} relation binding changed; administrator rebind is required",
                            source.name
                        )));
                    }
                    let binding_fingerprint = binding
                        .get::<JsonB>(2)
                        .map_err(|error| MdmError::Spi(error.to_string()))?
                        .map(|value| value.0);
                    if binding_fingerprint.as_ref() != Some(&source.binding_fingerprint) {
                        return Err(MdmError::SourceInvalid(format!(
                            "source {} binding changed; administrator rebind is required",
                            source.name
                        )));
                    }
                    source_id
                } else {
                    let rows = client.update("INSERT INTO mdm_internal.source_identities (entity_id, source_name, relation_name, key_contract, identity_digest) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5) RETURNING source_identity_id::text", Some(1), &[entity_id.clone().into(), source.name.clone().into(), source.relation_name.clone().into(), JsonB(source.key_contract.clone()).into(), source.identity_digest.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
                    let source_id = rows
                        .first()
                        .get::<String>(1)
                        .map_err(|error| MdmError::Spi(error.to_string()))?
                        .ok_or_else(|| MdmError::Spi("source identity ID is NULL".into()))?;
                    client.update("INSERT INTO mdm_internal.source_bindings (source_identity_id, relation_oid, binding_fingerprint) VALUES ($1::pg_catalog.uuid, $2, $3)", None, &[source_id.clone().into(), pg_sys::Oid::from(source.relation_oid).into(), JsonB(source.binding_fingerprint.clone()).into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
                    source_id
                };
                let _ = source_id;
            }
            client.update("INSERT INTO mdm_internal.definitions (entity_id, definition_version, parent_version, user_definition, expanded_definition, logical_candidate_plan, semantic_manifest, definition_digest, comment, created_by_name) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10)", None, &[entity_id.clone().into(), version.into(), if version > 1 { Some(version - 1) } else { None }.into(), JsonB(prepared.user_definition.clone()).into(), JsonB(prepared.expanded_definition.clone()).into(), JsonB(prepared.logical_candidate_plan.clone()).into(), JsonB(prepared.semantic_manifest.clone()).into(), prepared.definition_digest.clone().into(), comment.into(), session_name.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            client.update("INSERT INTO mdm_internal.definition_artifacts (entity_id, definition_version, compiler_version, artifact_format_version, artifact_bytes, artifact_digest, created_by_name) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5, $6, $7)", None, &[entity_id.clone().into(), version.into(), graph_spec::COMPILER_VERSION.into(), graph_spec::ARTIFACT_FORMAT_VERSION.into(), prepared.artifact_bytes.clone().into(), prepared.artifact_digest.clone().into(), session_name.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            client.update("UPDATE mdm_internal.entities SET desired_version = $2 WHERE entity_id = $1::pg_catalog.uuid", None, &[entity_id.into(), version.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
        }
        complete_operation(client, &operation_id)
    })?;
    Ok((operation_id, version, changed))
}

fn selected_oid() -> pg_sys::Oid {
    catalog::outer_user_id()
}

#[pg_extern(
    name = "persist_entity",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_entity(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_entity_wrapper';"
)]
pub(crate) fn persist_entity(request: Internal) -> JsonB {
    let result = (|| {
        let helper_owner = catalog::validate_helper_owner()?;
        let (session, selected) = catalog::validate_caller(&helper_owner)?;
        // SAFETY: only call_persist supplies this internal request. PostgreSQL
        // prevents SQL callers from constructing it; reject NULL explicitly.
        let request = unsafe { request.get::<CreateRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("validated create request is required".into()))?;
        let prepared = &request.prepared;
        let entity = parse_entity(prepared.expanded_definition.clone())
            .map_err(MdmError::DefinitionInvalid)?;
        let (operation_id, version, changed) = persist(
            prepared.clone(),
            entity.clone(),
            request.expected_version,
            request.comment.clone(),
            session.name,
            selected.name,
        )?;
        Ok(helper_result(
            operation_id,
            entity.name,
            version,
            changed,
            prepared,
        ))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
