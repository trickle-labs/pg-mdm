use std::collections::BTreeMap;

use pgrx::prelude::*;
use pgrx::spi::SpiClient;
use pgrx::{Internal, JsonB, Uuid};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog;
use crate::definition::canonical::{canonical_definition, digest, hex, json_bytes};
use crate::definition::parse_entity;
use crate::definition::validate::{PreparedDefinition, digest_hex, prepare};
use crate::error::MdmError;
use crate::graph_spec;
use crate::source_record::quote_identifier;

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

struct InstalledMember {
    logical_id: String,
    ordinal: i32,
    relation_oid: pg_sys::Oid,
    relation_name: String,
    contract_generation: i64,
    contract_digest: Vec<u8>,
    contract: Value,
}

fn source_binding_digest(prepared: &PreparedDefinition) -> Vec<u8> {
    let sources = prepared
        .sources
        .iter()
        .map(|source| {
            json!({
                "name": source.name,
                "relation_name": source.relation_name,
                "relation_oid": source.relation_oid,
                "identity_digest": hex(&source.identity_digest),
                "binding_fingerprint": source.binding_fingerprint
            })
        })
        .collect::<Vec<_>>();
    digest(
        "pg_mdm/source-binding/v1",
        &[&json_bytes(&canonical_definition(json!(sources)))],
    )
}

fn physical_name(binding_id: &str, ordinal: i32) -> String {
    format!("mdm_g_{}_{}", binding_id.replace('-', ""), ordinal)
}

fn grant_graph_access(client: &mut SpiClient<'_>, role: &str) -> Result<(), MdmError> {
    let role = quote_identifier(role);
    client
        .update(
            &format!("GRANT USAGE, CREATE ON SCHEMA mdm_graph TO {role}"),
            None,
            &[],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?;
    client
        .update(
            &format!(
                "GRANT SELECT, MAINTAIN ON mdm_graph.source_identity_map, mdm_graph.source_records, mdm_graph.definition_limits TO {role}"
            ),
            None,
            &[],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?;
    client
        .update(
            &format!(
                "GRANT EXECUTE ON FUNCTION mdm_graph.normalize_text(text, text, integer, text, jsonb), mdm_graph.normalize_date(date, text, integer, text, jsonb), mdm_graph.normalized_levenshtein_score(text, text, bigint), mdm_graph.evidence_digest(text) TO {role}"
            ),
            None,
            &[],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?;
    Ok(())
}

fn revoke_graph_create(client: &mut SpiClient<'_>, role: &str) -> Result<(), MdmError> {
    let role = quote_identifier(role);
    client
        .update(
            &format!("REVOKE CREATE ON SCHEMA mdm_graph FROM {role}"),
            None,
            &[],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?;
    Ok(())
}

fn member_contract(
    client: &mut SpiClient<'_>,
    relation_name: &str,
    expected_query: &str,
    role: &str,
) -> Result<InstalledMember, MdmError> {
    let relation_oid = client
        .select(
            "SELECT pg_catalog.to_regclass($1)::pg_catalog.oid",
            Some(1),
            &[relation_name.into()],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
        .first()
        .get::<pg_sys::Oid>(1)
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
        .ok_or_else(|| {
            MdmError::GraphContract(format!("created member {relation_name} is missing"))
        })?;
    let table = client
        .select(
            "SELECT contract_version, contract_generation, contract_digest, contract FROM pgtrickle.stream_table_contract($1::regclass)",
            Some(1),
            &[relation_name.into()],
        )
        .map_err(|error| MdmError::GraphContract(error.to_string()))?;
    if table.is_empty() {
        return Err(MdmError::GraphContract(format!(
            "contract for {relation_name} is missing"
        )));
    }
    let row = table.first();
    let version = row
        .get::<i16>(1)
        .map_err(|error| MdmError::GraphContract(error.to_string()))?
        .ok_or_else(|| MdmError::GraphContract("contract version is NULL".into()))?;
    let generation = row
        .get::<i64>(2)
        .map_err(|error| MdmError::GraphContract(error.to_string()))?
        .ok_or_else(|| MdmError::GraphContract("contract generation is NULL".into()))?;
    let contract_digest = row
        .get::<Vec<u8>>(3)
        .map_err(|error| MdmError::GraphContract(error.to_string()))?
        .ok_or_else(|| MdmError::GraphContract("contract digest is NULL".into()))?;
    let contract = row
        .get::<JsonB>(4)
        .map_err(|error| MdmError::GraphContract(error.to_string()))?
        .ok_or_else(|| MdmError::GraphContract("contract is NULL".into()))?
        .0;
    let owner = client
        .select(
            "SELECT pg_catalog.pg_get_userbyid(c.relowner)::text FROM pg_catalog.pg_class c WHERE c.oid = $1",
            Some(1),
            &[relation_oid.into()],
        )
        .map_err(|error| MdmError::GraphContract(error.to_string()))?
        .first()
        .get::<String>(1)
        .map_err(|error| MdmError::GraphContract(error.to_string()))?
        .ok_or_else(|| MdmError::GraphContract("member owner is NULL".into()))?;
    if version != 1
        || contract_digest.len() != 32
        || owner != role
        || contract.get("orchestration_mode").and_then(Value::as_str) != Some("EXTERNAL")
        || contract
            .get("relation")
            .and_then(|value| value.get("owner"))
            .and_then(Value::as_str)
            != Some(role)
    {
        return Err(MdmError::GraphContract(format!(
            "member contract for {relation_name} does not match its binding"
        )));
    }
    if let Some(actual_query) = contract
        .get("defining_query")
        .or_else(|| contract.get("query"))
        .and_then(Value::as_str)
        && (actual_query.trim().is_empty() || expected_query.trim().is_empty())
    {
        return Err(MdmError::GraphContract(format!(
            "member contract for {relation_name} has no defining query"
        )));
    }
    Ok(InstalledMember {
        logical_id: String::new(),
        ordinal: 0,
        relation_oid,
        relation_name: relation_name.into(),
        contract_generation: generation,
        contract_digest,
        contract,
    })
}

fn install_graph(
    client: &mut SpiClient<'_>,
    entity_id: &str,
    definition_version: i64,
    prepared: &PreparedDefinition,
    role: &str,
) -> Result<(), MdmError> {
    let (nodes, roots) = graph_spec::artifact_nodes(&prepared.artifact_bytes)?;
    let artifact_id = client
        .select(
            "SELECT artifact_id::text FROM mdm_internal.definition_artifacts WHERE entity_id = $1::pg_catalog.uuid AND definition_version = $2 AND artifact_digest = $3",
            Some(1),
            &[
                entity_id.into(),
                definition_version.into(),
                prepared.artifact_digest.clone().into(),
            ],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
        .first()
        .get::<String>(1)
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
        .ok_or_else(|| MdmError::GraphInstallation("stored graph artifact is missing".into()))?;
    let source_digest = source_binding_digest(prepared);
    let existing = client
        .select(
            "SELECT graph_binding_id FROM mdm_internal.graph_bindings WHERE entity_id = $1::pg_catalog.uuid AND definition_version = $2 AND artifact_id = $3::pg_catalog.uuid AND execution_role_oid = $4 AND source_binding_digest = $5",
            Some(1),
            &[
                entity_id.into(),
                definition_version.into(),
                artifact_id.clone().into(),
                selected_oid().into(),
                source_digest.clone().into(),
            ],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?;
    if !existing.is_empty() {
        return Ok(());
    }

    let binding_id = client
        .select("SELECT pg_catalog.uuidv7()::text", Some(1), &[])
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
        .first()
        .get::<String>(1)
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
        .ok_or_else(|| MdmError::GraphInstallation("graph binding ID is NULL".into()))?;
    let generation = client
        .select(
            "SELECT COALESCE(pg_catalog.max(graph_generation), 0) + 1 FROM mdm_internal.graph_bindings WHERE entity_id = $1::pg_catalog.uuid",
            Some(1),
            &[entity_id.into()],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
        .first()
        .get::<i64>(1)
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
        .ok_or_else(|| MdmError::GraphInstallation("graph generation is NULL".into()))?;

    grant_graph_access(client, role)?;
    client
        .update(
            "DELETE FROM mdm_graph.source_identity_map WHERE entity_id = $1::pg_catalog.uuid; DELETE FROM mdm_graph.source_records WHERE entity_id = $1::pg_catalog.uuid; DELETE FROM mdm_graph.definition_limits WHERE entity_id = $1::pg_catalog.uuid",
            None,
            &[entity_id.into()],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?;
    client
        .update(
            "INSERT INTO mdm_graph.source_identity_map (entity_id, entity_name, source_identity_id, source_name) SELECT e.entity_id, e.entity_name::text, s.source_identity_id, s.source_name::text FROM mdm_internal.entities e JOIN mdm_internal.source_identities s ON s.entity_id = e.entity_id WHERE e.entity_id = $1::pg_catalog.uuid",
            None,
            &[entity_id.into()],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?;
    client
        .update(
            "INSERT INTO mdm_graph.source_records (entity_id, source_identity_id, source_record_key, source_record_id, active) SELECT entity_id, source_identity_id, source_record_key, source_record_id, active FROM mdm_internal.source_records WHERE entity_id = $1::pg_catalog.uuid",
            None,
            &[entity_id.into()],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?;
    client
        .update(
            "INSERT INTO mdm_graph.definition_limits (entity_id, entity_name, expanded_definition) SELECT e.entity_id, e.entity_name::text, d.expanded_definition FROM mdm_internal.entities e JOIN mdm_internal.definitions d ON d.entity_id = e.entity_id AND d.definition_version = $2 WHERE e.entity_id = $1::pg_catalog.uuid",
            None,
            &[entity_id.into(), definition_version.into()],
        )
        .map_err(|error| MdmError::GraphInstallation(error.to_string()))?;
    let mut relations = BTreeMap::new();
    let mut members = Vec::with_capacity(nodes.len());
    for (ordinal, node) in nodes.iter().enumerate() {
        let name = physical_name(&binding_id, ordinal as i32);
        let qualified = format!("mdm_graph.{}", quote_identifier(&name));
        if client
            .select(
                "SELECT pg_catalog.to_regclass($1)::pg_catalog.oid",
                Some(1),
                &[qualified.clone().into()],
            )
            .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
            .first()
            .get::<pg_sys::Oid>(1)
            .map_err(|error| MdmError::GraphInstallation(error.to_string()))?
            .is_some()
        {
            return Err(MdmError::GraphInstallation(format!(
                "graph member name collision: {qualified}"
            )));
        }
        let query = graph_spec::render_sql(&node.defining_sql, &relations)?;
        client
            .update(
                "SELECT pgtrickle.create_stream_table(name => $1::text, query => $2::text, schedule => 'calculated', refresh_mode => 'AUTO', initialize => false, cdc_mode => 'trigger', orchestration_mode => 'EXTERNAL')",
                Some(1),
                &[qualified.clone().into(), query.clone().into()],
            )
            .map_err(|error| {
                MdmError::GraphInstallation(format!("{}: {}", node.logical_id, error))
            })?;
        let mut member = member_contract(client, &qualified, &query, role)?;
        member.logical_id = node.logical_id.clone();
        member.ordinal = ordinal as i32;
        relations.insert(node.logical_id.clone(), qualified);
        members.push(member);
    }

    let root_names = roots
        .iter()
        .map(|root| {
            relations
                .get(root)
                .cloned()
                .ok_or_else(|| MdmError::GraphArtifact(format!("root {root} was not installed")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let contract_table = client
        .select(
            "SELECT contract_version, graph_digest, contract FROM pgtrickle.graph_contract(ARRAY[$1::regclass])",
            Some(1),
            &[root_names[0].clone().into()],
        )
        .map_err(|error| MdmError::GraphContract(error.to_string()))?;
    if contract_table.is_empty() {
        return Err(MdmError::GraphContract("graph contract is missing".into()));
    }
    let graph_row = contract_table.first();
    let graph_version = graph_row
        .get::<i16>(1)
        .map_err(|error| MdmError::GraphContract(error.to_string()))?
        .ok_or_else(|| MdmError::GraphContract("graph contract version is NULL".into()))?;
    let graph_digest = graph_row
        .get::<Vec<u8>>(2)
        .map_err(|error| MdmError::GraphContract(error.to_string()))?
        .ok_or_else(|| MdmError::GraphContract("graph digest is NULL".into()))?;
    let graph_contract = graph_row
        .get::<JsonB>(3)
        .map_err(|error| MdmError::GraphContract(error.to_string()))?
        .ok_or_else(|| MdmError::GraphContract("graph contract is NULL".into()))?
        .0;
    let contract_members = graph_contract
        .get("members")
        .and_then(Value::as_array)
        .ok_or_else(|| MdmError::GraphContract("graph members are missing".into()))?;
    if graph_version != 1
        || graph_digest.len() != 32
        || contract_members.len() != members.len()
        || contract_members.iter().any(|member| {
            member.get("orchestration_mode").and_then(Value::as_str) != Some("EXTERNAL")
        })
    {
        return Err(MdmError::GraphContract(
            "graph contract does not match installed members".into(),
        ));
    }

    let binding_input = json!({
        "artifact_digest": hex(&prepared.artifact_digest),
        "graph_generation": generation,
        "execution_role_oid": selected_oid().to_u32(),
        "source_binding_digest": hex(&source_digest),
        "members": members.iter().map(|member| json!({
            "logical_id": member.logical_id,
            "ordinal": member.ordinal,
            "relation_oid": member.relation_oid.to_u32(),
            "relation_name": member.relation_name,
            "contract_generation": member.contract_generation,
            "contract_digest": hex(&member.contract_digest),
            "contract": member.contract
        })).collect::<Vec<_>>(),
        "roots": members.iter().filter(|member| roots.contains(&member.logical_id)).map(|member| member.relation_oid.to_u32()).collect::<Vec<_>>(),
        "graph_digest": hex(&graph_digest),
        "graph_contract": graph_contract
    });
    let binding_digest = digest(
        "pg_mdm/graph-binding/v1",
        &[&json_bytes(&canonical_definition(binding_input))],
    );

    client
        .update(
            "INSERT INTO mdm_internal.graph_bindings (graph_binding_id, entity_id, definition_version, artifact_id, graph_generation, execution_role_oid, source_binding_digest, root_relation_oids, graph_contract_version, graph_digest, graph_contract, graph_binding_digest) VALUES ($1::pg_catalog.uuid, $2::pg_catalog.uuid, $3, $4::pg_catalog.uuid, $5, $6, $7, ARRAY[$8::oid], $9, $10, $11, $12)",
            None,
            &[
                binding_id.clone().into(),
                entity_id.into(),
                definition_version.into(),
                artifact_id.into(),
                generation.into(),
                selected_oid().into(),
                source_digest.into(),
                members
                    .iter()
                    .find(|member| roots.contains(&member.logical_id))
                    .ok_or_else(|| MdmError::GraphContract("graph root member is missing".into()))?
                    .relation_oid
                    .into(),
                graph_version.into(),
                graph_digest.clone().into(),
                JsonB(graph_contract.clone()).into(),
                binding_digest.clone().into(),
            ],
        )
        .map_err(|error| MdmError::GraphBinding(error.to_string()))?;
    for member in &members {
        client
            .update(
                "INSERT INTO mdm_internal.graph_members (graph_binding_id, logical_id, topological_ordinal, relation_oid, relation_name, contract_generation, contract_digest, contract) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5, $6, $7, $8)",
                None,
                &[
                    binding_id.clone().into(),
                    member.logical_id.clone().into(),
                    member.ordinal.into(),
                    member.relation_oid.into(),
                    member.relation_name.clone().into(),
                    member.contract_generation.into(),
                    member.contract_digest.clone().into(),
                    JsonB(member.contract.clone()).into(),
                ],
            )
            .map_err(|error| MdmError::GraphBinding(error.to_string()))?;
    }
    revoke_graph_create(client, role)?;
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
    let capabilities = crate::integration::require_graph_v1()?;
    let outcome = JsonB(
        json!({"definition_digest": digest_hex(&prepared.definition_digest), "artifact_digest": digest_hex(&prepared.artifact_digest), "graph_executable": true, "capabilities": capabilities}),
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
            client.update("UPDATE mdm_internal.entities SET desired_version = $2 WHERE entity_id = $1::pg_catalog.uuid", None, &[entity_id.clone().into(), version.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
        }
        install_graph(client, &entity_id, version, &prepared, &selected)?;
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
