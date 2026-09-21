use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use pgrx::prelude::*;
use pgrx::spi::SpiClient;
use pgrx::{Internal, JsonB, Uuid, default};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::policy::project_policy_cases;
use crate::candidate::CandidatePair;
use crate::catalog;
use crate::constraint::{DecisionEdge, DecisionKind};
use crate::definition::canonical::{digest, hex, json_bytes};
use crate::definition::{Entity, parse_entity};
use crate::error::MdmError;
use crate::evaluation::{self, EvaluationRecord, GoldenRow};
use crate::evidence::{EvidenceClass, EvidenceItem};
use crate::golden::{GoldenOverride, GoldenSelection};
use crate::identity::{self, IdentityState};
use crate::normalization::NormalizedState;
use crate::output::{self, OutputField};
use crate::pair::decide_pair_for_rules;
use crate::resolver::ResolverLimits;
use crate::review::{Review, ReviewStatus};
use crate::source_record::quote_identifier;

struct RefreshRequest {
    entity_name: String,
    full_policy: String,
    rebuild: bool,
    source_snapshots: Vec<SourceSnapshot>,
}

#[derive(Clone, Debug)]
struct SourceSnapshot {
    source_name: String,
    keys: Vec<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
struct SourceScanSpec {
    source_name: String,
    sql: String,
    max_active_records: usize,
}

#[derive(Debug, Deserialize)]
struct RefreshAccess {
    entity_id: String,
    relations: Vec<String>,
    source_scans: Vec<SourceScanSpec>,
}

struct RefreshAccessRequest {
    entity_name: String,
}

struct PreviewRequest {
    entity_name: String,
    mode: String,
    options: Value,
}

#[derive(Clone, Debug)]
struct Context {
    entity_id: String,
    graph_binding_id: String,
    entity: Entity,
    definition_version: i64,
    decision_epoch: i64,
    publication_revision: i64,
    artifact_id: String,
    artifact_digest: Vec<u8>,
    graph_digest: Vec<u8>,
    graph_contract: Value,
    graph_root: String,
    pair_stats_relation: Option<String>,
    evidence_relation: String,
    golden_relation: String,
}

#[derive(Clone, Debug)]
struct GraphRefresh {
    id: i64,
    boundary: Value,
    boundary_digest: Vec<u8>,
    node_results: Value,
}

#[derive(Clone, Debug)]
struct DeltaConsumer {
    logical_id: String,
    consumer_id: Uuid,
    delta_relation: String,
    contract_digest: Vec<u8>,
    row_identity_version: i16,
    state: String,
    acknowledged_token: i64,
    log_head: i64,
    resnapshot_token: Option<Uuid>,
    through_token: Option<i64>,
    batch_count: usize,
    row_count: usize,
    saw_full_invalidation: bool,
    affected_records: BTreeSet<Uuid>,
    lag: i64,
}

#[derive(Debug, PartialEq)]
struct DeltaBatch {
    token: i64,
    row_count: i64,
    rows_inserted: i64,
    rows_deleted: i64,
    mode: String,
    contract_digest: Vec<u8>,
    row_identity_version: i16,
}

#[derive(Debug, PartialEq)]
struct DeltaRow {
    action: String,
    row_identity: Vec<u8>,
    source_record_ids: Vec<Uuid>,
}

fn validate_delta_batch(
    batch: &DeltaBatch,
    expected_token: i64,
    contract_digest: &[u8],
    row_identity_version: i16,
) -> Result<(), MdmError> {
    let counts_match = batch
        .rows_inserted
        .checked_add(batch.rows_deleted)
        .is_some_and(|count| count == batch.row_count);
    if batch.token != expected_token
        || batch.row_count < 0
        || batch.rows_inserted < 0
        || batch.rows_deleted < 0
        || !counts_match
        || batch.contract_digest != contract_digest
        || batch.row_identity_version != row_identity_version
        || !matches!(batch.mode.as_str(), "EXACT" | "FULL_INVALIDATION")
        || (batch.mode == "FULL_INVALIDATION"
            && (batch.row_count != 0 || batch.rows_inserted != 0 || batch.rows_deleted != 0))
    {
        return Err(MdmError::DeltaProtocol(format!(
            "invalid batch metadata for token {}",
            batch.token
        )));
    }
    Ok(())
}

fn validate_delta_payload(payload: &[DeltaRow], batch: &DeltaBatch) -> Result<(), MdmError> {
    if payload.len() as i64 != batch.row_count {
        return Err(MdmError::DeltaProtocol(format!(
            "payload row count for batch {} does not match metadata",
            batch.token
        )));
    }
    let mut inserted = 0_i64;
    let mut deleted = 0_i64;
    for row in payload {
        if row.row_identity.is_empty() || row.source_record_ids.is_empty() {
            return Err(MdmError::DeltaProtocol(
                "payload row identity or source record IDs are empty".into(),
            ));
        }
        match row.action.as_str() {
            "INSERT" => inserted += 1,
            "DELETE" => deleted += 1,
            _ => return Err(MdmError::DeltaProtocol("payload action is invalid".into())),
        }
    }
    if inserted != batch.rows_inserted || deleted != batch.rows_deleted {
        return Err(MdmError::DeltaProtocol(format!(
            "payload action counts for batch {} do not match metadata",
            batch.token
        )));
    }
    Ok(())
}

fn node_mode_summary(node_results: &Value) -> (Value, Value) {
    let mut modes = BTreeMap::<String, usize>::new();
    let mut fallbacks = Vec::new();
    for result in node_results
        .as_object()
        .into_iter()
        .flat_map(|nodes| nodes.values())
    {
        let effective_mode = result
            .get("action")
            .or_else(|| result.get("effective_mode"))
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN");
        *modes.entry(effective_mode.into()).or_default() += 1;
        let requested_mode = result.get("requested_mode").and_then(Value::as_str);
        let fallback_reason = if effective_mode == "FULL" && requested_mode != Some("FULL") {
            result
                .get("fallback_reason")
                .or_else(|| result.get("reason"))
                .and_then(Value::as_str)
                .or_else(|| {
                    requested_mode
                        .filter(|requested| *requested != effective_mode)
                        .and_then(|_| result.get("result_class").and_then(Value::as_str))
                })
        } else {
            None
        };
        if let Some(reason) = fallback_reason {
            fallbacks.push(json!({
                "logical_id": result.get("identity").cloned().unwrap_or(Value::Null),
                "reason": reason,
            }));
        }
    }
    fallbacks.sort_by_key(|fallback| {
        fallback
            .get("logical_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    });
    (json!(modes), Value::Array(fallbacks))
}

#[derive(Clone, Debug)]
struct SourceRow {
    id: Uuid,
    source_name: String,
    sort_key: Vec<u8>,
}

type PairEvidence = (CandidatePair, Vec<EvidenceItem>);

#[derive(Clone, Debug, Serialize)]
struct RefreshResult {
    operation_id: String,
    entity_name: String,
    changed: bool,
    publication_revision: i64,
    graph_refresh_id: i64,
    source_boundary: Value,
    source_boundary_digest: String,
    node_results: Value,
    active_records: usize,
    identities: usize,
    open_reviews: usize,
    stage_timings_ms: Value,
    component_checks: usize,
    resolver_strategy: String,
    resolver_fallback_reason: Option<String>,
    delta_batch_count: usize,
    delta_row_count: usize,
    delta_acknowledged_token: Option<String>,
    delta_lag: Option<i64>,
    effective_node_modes: Value,
    unexpected_full_fallbacks: Value,
    affected_records: usize,
    affected_components: usize,
    shadow_comparison: Option<ShadowComparison>,
}

#[derive(Clone, Debug, Serialize)]
struct ShadowComparison {
    affected_records: usize,
    affected_components: usize,
    equivalent: bool,
    shadow_digest: Option<String>,
    full_digest: Option<String>,
}

#[derive(Default)]
struct ControlChanges {
    seeds: BTreeSet<Uuid>,
    reconstructable: bool,
    changed: bool,
}

fn normalized_state(value: &str) -> Result<NormalizedState, MdmError> {
    match value {
        "value" => Ok(NormalizedState::Value),
        "absent" => Ok(NormalizedState::Absent),
        "empty" => Ok(NormalizedState::Empty),
        "invalid" => Ok(NormalizedState::Invalid),
        "unknown" => Ok(NormalizedState::Unknown),
        "redacted" => Ok(NormalizedState::Redacted),
        "unsupported" => Ok(NormalizedState::Unsupported),
        other => Err(MdmError::RefreshFailed(format!(
            "unknown normalized state {other}"
        ))),
    }
}

fn sql_literal(value: &Value, type_name: &str) -> String {
    let value = match value {
        Value::Null => "NULL".into(),
        Value::String(value) => format!("'{}'", value.replace('\'', "''")),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => {
            format!("'{}'", value.to_string().replace('\'', "''"))
        }
    };
    format!("{value}::{type_name}")
}

fn stable_graph_contract(contract: &Value) -> Value {
    let mut stable = contract.clone();
    if let Some(object) = stable.as_object_mut() {
        object.remove("graph_digest");
    }
    if let Some(members) = stable.get_mut("members").and_then(Value::as_array_mut) {
        for member in members {
            if let Some(member) = member.as_object_mut() {
                member.remove("contract_generation");
            }
        }
    }
    stable
}

fn load_context(
    client: &SpiClient<'_>,
    entity_name: &str,
    selected: &catalog::Role,
    lock: bool,
) -> Result<Context, MdmError> {
    let suffix = if lock { " FOR UPDATE OF e" } else { "" };
    let query = format!(
        "SELECT e.entity_id::text, e.desired_version, e.decision_epoch, e.publication_revision, e.execution_role_name, b.role_oid, d.expanded_definition, a.artifact_id::text, a.artifact_digest FROM mdm_internal.entities e JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id JOIN mdm_internal.definitions d ON d.entity_id = e.entity_id AND d.definition_version = e.desired_version JOIN LATERAL (SELECT artifact_id, artifact_digest FROM mdm_internal.definition_artifacts x WHERE x.entity_id = d.entity_id AND x.definition_version = d.definition_version ORDER BY x.artifact_id DESC LIMIT 1) a ON true WHERE e.entity_name = $1::pg_catalog.name{suffix}"
    );
    let rows = client
        .select(&query, Some(1), &[entity_name.into()])
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if rows.is_empty() {
        return Err(MdmError::DefinitionInvalid(format!(
            "entity {entity_name} does not exist"
        )));
    }
    let row = rows.first();
    let entity_id = row
        .get::<String>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("entity ID is NULL".into()))?;
    let execution_role = row
        .get::<String>(5)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("execution role is NULL".into()))?;
    let bound_oid = row
        .get::<pg_sys::Oid>(6)
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if execution_role != selected.name || bound_oid != Some(selected.oid) {
        return Err(MdmError::Unauthorized(
            "entity is bound to another execution role".into(),
        ));
    }
    let definition = row
        .get::<JsonB>(7)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("expanded definition is NULL".into()))?;
    let entity = parse_entity(definition.0).map_err(MdmError::DefinitionInvalid)?;
    let definition_version = row
        .get::<i64>(2)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("definition version is NULL".into()))?;
    let decision_epoch = row
        .get::<i64>(3)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("decision epoch is NULL".into()))?;
    let publication_revision = row
        .get::<i64>(4)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("publication revision is NULL".into()))?;
    let artifact_id = row
        .get::<String>(8)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("artifact ID is NULL".into()))?;
    let artifact_digest = row
        .get::<Vec<u8>>(9)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("artifact digest is NULL".into()))?;
    let binding = client
        .select(
            "SELECT b.graph_binding_id::text, b.graph_digest, gm.relation_name, b.graph_contract FROM mdm_internal.graph_bindings b JOIN mdm_internal.graph_members gm ON gm.graph_binding_id = b.graph_binding_id AND gm.logical_id = $3 WHERE b.entity_id = $1::pg_catalog.uuid AND b.definition_version = $2 AND b.artifact_id = $4::pg_catalog.uuid ORDER BY b.graph_generation DESC LIMIT 1",
            Some(1),
            &[
                entity_id.clone().into(),
                definition_version.into(),
                format!("golden/{}", entity_name).into(),
                artifact_id.clone().into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if binding.is_empty() {
        return Err(MdmError::GraphBinding(
            "no graph binding exists for the desired definition".into(),
        ));
    }
    let binding_row = binding.first();
    let binding_id = binding_row
        .get::<String>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("graph binding ID is NULL".into()))?;
    let graph_digest = binding_row
        .get::<Vec<u8>>(2)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("graph digest is NULL".into()))?;
    let graph_root = binding_row
        .get::<String>(3)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("graph root relation is NULL".into()))?;
    let graph_contract = binding_row
        .get::<JsonB>(4)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("graph contract is NULL".into()))?
        .0;
    let members = client
        .select(
            "SELECT m.logical_id, m.relation_name, m.relation_oid, pg_catalog.to_regclass(m.relation_name)::pg_catalog.oid, c.relowner FROM mdm_internal.graph_members m LEFT JOIN pg_catalog.pg_class c ON c.oid = m.relation_oid WHERE m.graph_binding_id = $1::pg_catalog.uuid ORDER BY m.topological_ordinal",
            None,
            &[binding_id.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let mut relations = BTreeMap::new();
    for member in members {
        let logical_id = member
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("graph logical ID is NULL".into()))?;
        let relation_name = member
            .get::<String>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("graph relation name is NULL".into()))?;
        let relation_oid = member
            .get::<pg_sys::Oid>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("graph relation OID is NULL".into()))?;
        let current_oid = member
            .get::<pg_sys::Oid>(4)
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let owner = member
            .get::<pg_sys::Oid>(5)
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if current_oid != Some(relation_oid) || owner != Some(selected.oid) {
            return Err(MdmError::GraphBinding(format!(
                "graph member {relation_name} no longer matches its binding"
            )));
        }
        relations.entry(logical_id).or_insert(relation_name);
    }
    Ok(Context {
        entity_id,
        graph_binding_id: binding_id,
        entity,
        definition_version,
        decision_epoch,
        publication_revision,
        artifact_id,
        artifact_digest,
        graph_digest,
        graph_contract,
        graph_root,
        pair_stats_relation: relations.remove(&format!("pair-stats/{entity_name}")),
        evidence_relation: relations
            .remove(&format!("evidence/{entity_name}"))
            .ok_or_else(|| MdmError::GraphBinding("evidence relation is missing".into()))?,
        golden_relation: relations
            .remove(&format!("golden/{entity_name}"))
            .ok_or_else(|| MdmError::GraphBinding("golden relation is missing".into()))?,
    })
}

fn sync_source_records(
    client: &mut SpiClient<'_>,
    context: &Context,
    snapshots: &[SourceSnapshot],
) -> Result<(), MdmError> {
    for source in &context.entity.sources {
        let snapshot = snapshots
            .iter()
            .find(|snapshot| snapshot.source_name == source.name)
            .ok_or_else(|| MdmError::GraphBinding("source snapshot is missing".into()))?;
        let rows = client
            .select(
                "SELECT source_identity_id::text FROM mdm_internal.source_identities WHERE entity_id = $1::pg_catalog.uuid AND source_name = $2::pg_catalog.name",
                Some(1),
                &[context.entity_id.clone().into(), source.name.clone().into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let source_identity_id = rows
            .first()
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::GraphBinding("source identity is missing".into()))?;
        let keys = JsonB(json!(
            snapshot.keys.iter().map(|key| hex(key)).collect::<Vec<_>>()
        ));
        let sql = "WITH current_records AS MATERIALIZED (SELECT pg_catalog.decode(key, 'hex') AS source_record_key FROM pg_catalog.jsonb_array_elements_text($3::pg_catalog.jsonb) AS keys(key)), deactivated AS (UPDATE mdm_internal.source_records r SET active = false WHERE r.entity_id = $1::pg_catalog.uuid AND r.source_identity_id = $2::pg_catalog.uuid AND r.active AND NOT EXISTS (SELECT FROM current_records c WHERE c.source_record_key = r.source_record_key) RETURNING 1), registered AS (INSERT INTO mdm_internal.source_records AS target (entity_id, source_identity_id, source_record_key, active) SELECT $1::pg_catalog.uuid, $2::pg_catalog.uuid, source_record_key, true FROM current_records ON CONFLICT (entity_id, source_identity_id, source_record_key) DO UPDATE SET active = true, last_seen_at = pg_catalog.statement_timestamp() WHERE NOT target.active RETURNING 1) SELECT EXISTS (SELECT FROM deactivated) OR EXISTS (SELECT FROM registered)";
        let _ = client
            .select(
                sql,
                Some(1),
                &[
                    context.entity_id.clone().into(),
                    source_identity_id.into(),
                    keys.into(),
                ],
            )
            .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    }
    Ok(())
}

fn grant_refresh_access(entity_name: &str) -> Result<Vec<SourceSnapshot>, MdmError> {
    let helper_owner = catalog::validate_helper_owner()?;
    let access = serde_json::from_value::<RefreshAccess>(
        catalog::call_helper(
            "refresh_access",
            RefreshAccessRequest {
                entity_name: entity_name.to_owned(),
            },
        )?
        .0,
    )
    .map_err(|error| {
        MdmError::Spi(format!(
            "refresh access helper returned invalid data: {error}"
        ))
    })?;
    Spi::connect_mut(|client| {
        client
            .update(
                "SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended($1, 0))",
                None,
                &[access.entity_id.clone().into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        for relation_name in &access.relations {
            client
                .update(
                    &format!(
                        "GRANT SELECT ON {relation_name} TO {}",
                        quote_identifier(&helper_owner.name)
                    ),
                    None,
                    &[],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
        }
        Ok(())
    })?;
    Spi::connect(|client| {
        let mut active_records = 0usize;
        access
            .source_scans
            .into_iter()
            .map(|scan| {
                let remaining = scan.max_active_records.saturating_sub(active_records);
                let query = format!("{} LIMIT {}", scan.sql, remaining.saturating_add(1));
                let rows = client
                    .select(&query, None, &[])
                    .map_err(|error| MdmError::SourceInvalid(error.to_string()))?;
                if rows.len() > remaining {
                    return Err(MdmError::ResolverLimit {
                        resource: "max_active_records",
                        observed: active_records.saturating_add(rows.len()),
                        limit: scan.max_active_records,
                    });
                }
                active_records = active_records.saturating_add(rows.len());
                let keys = rows
                    .into_iter()
                    .map(|row| {
                        row.get::<Vec<u8>>(1)
                            .map_err(|error| MdmError::Spi(error.to_string()))?
                            .ok_or_else(|| {
                                MdmError::SourceRecord("source record key is NULL".into())
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(SourceSnapshot {
                    source_name: scan.source_name,
                    keys,
                })
            })
            .collect()
    })
}

#[pg_extern(
    name = "refresh_access",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.refresh_access(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'refresh_access_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn refresh_access(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only refresh constructs RefreshAccessRequest.
        let request = unsafe { request.get::<RefreshAccessRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("refresh access request is required".into()))?;
        let helper_owner = catalog::validate_helper_owner()?;
        let (_, selected) = catalog::validate_caller(&helper_owner)?;
        Spi::connect(|client| {
            let context = load_context(client, &request.entity_name, &selected, false)?;
            let rows = client
                .select(
                    "SELECT m.relation_name FROM mdm_internal.graph_members m JOIN mdm_internal.graph_bindings b ON b.graph_binding_id = m.graph_binding_id JOIN mdm_internal.entities e ON e.entity_id = b.entity_id WHERE e.entity_name = $1::pg_catalog.name AND b.definition_version = e.desired_version AND b.execution_role_oid = $2 AND b.graph_generation = (SELECT max(b2.graph_generation) FROM mdm_internal.graph_bindings b2 WHERE b2.entity_id = e.entity_id AND b2.definition_version = e.desired_version AND b2.execution_role_oid = $2) ORDER BY m.topological_ordinal",
                    None,
                    &[request.entity_name.clone().into(), selected.oid.into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let relations = rows
                .map(|member| {
                    member
                        .get::<String>(1)
                        .map_err(|error| MdmError::Spi(error.to_string()))?
                        .ok_or_else(|| MdmError::Spi("graph relation name is NULL".into()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let max_active_records = load_limits(&context).max_active_records;
            let source_scans = context
                .entity
                .sources
                .iter()
                .map(|source| {
                    let rows = client
                        .select(
                            "SELECT relation_name FROM mdm_internal.source_identities WHERE entity_id = $1::pg_catalog.uuid AND source_name = $2::pg_catalog.name",
                            Some(1),
                            &[context.entity_id.clone().into(), source.name.clone().into()],
                        )
                        .map_err(|error| MdmError::Spi(error.to_string()))?;
                    let relation_name = rows
                        .first()
                        .get::<String>(1)
                        .map_err(|error| MdmError::Spi(error.to_string()))?
                        .ok_or_else(|| MdmError::GraphBinding("source relation is missing".into()))?;
                    let mut scan_source = source.clone();
                    scan_source.relation = relation_name;
                    let query = crate::graph_spec::source_record_sql(
                        &context.entity.name,
                        &scan_source,
                    );
                    Ok(json!({
                        "source_name": source.name,
                        "sql": format!(
                            "SELECT source_record_key FROM ({query}) AS source_rows"
                        ),
                        "max_active_records": max_active_records
                    }))
                })
                .collect::<Result<Vec<_>, MdmError>>()?;
            Ok(JsonB(json!({
                "entity_id": context.entity_id,
                "relations": relations,
                "source_scans": source_scans
            })))
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

fn refresh_graph(
    client: &mut SpiClient<'_>,
    context: &Context,
    full_policy: &str,
) -> Result<GraphRefresh, MdmError> {
    if !matches!(full_policy, "ALLOW" | "ERROR") {
        return Err(MdmError::RefreshBoundary(
            "full_policy must be ALLOW or ERROR".into(),
        ));
    }
    client
        .update(
            "DELETE FROM mdm_graph.source_identity_map target WHERE target.entity_id = $1::pg_catalog.uuid AND NOT EXISTS (SELECT FROM mdm_internal.source_identities source WHERE source.entity_id = target.entity_id AND source.source_name::text = target.source_name); DELETE FROM mdm_graph.source_records target WHERE target.entity_id = $1::pg_catalog.uuid AND NOT EXISTS (SELECT FROM mdm_internal.source_records source WHERE source.entity_id = target.entity_id AND source.source_record_id = target.source_record_id); DELETE FROM mdm_graph.definition_limits target WHERE target.entity_id = $1::pg_catalog.uuid AND NOT EXISTS (SELECT FROM mdm_internal.entities entity JOIN mdm_internal.definitions definition ON definition.entity_id = entity.entity_id AND definition.definition_version = entity.desired_version WHERE entity.entity_id = target.entity_id)",
            None,
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    client
        .update(
            "INSERT INTO mdm_graph.source_identity_map AS target (entity_id, entity_name, source_identity_id, source_name) SELECT e.entity_id, e.entity_name::text, s.source_identity_id, s.source_name::text FROM mdm_internal.entities e JOIN mdm_internal.source_identities s ON s.entity_id = e.entity_id WHERE e.entity_id = $1::pg_catalog.uuid ON CONFLICT (entity_id, source_name) DO UPDATE SET entity_name = EXCLUDED.entity_name, source_identity_id = EXCLUDED.source_identity_id, source_name = EXCLUDED.source_name WHERE target.entity_name IS DISTINCT FROM EXCLUDED.entity_name OR target.source_identity_id IS DISTINCT FROM EXCLUDED.source_identity_id OR target.source_name IS DISTINCT FROM EXCLUDED.source_name",
            None,
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    client
        .update(
            "INSERT INTO mdm_graph.source_records AS target (entity_id, source_identity_id, source_record_key, source_record_id, active) SELECT entity_id, source_identity_id, source_record_key, source_record_id, active FROM mdm_internal.source_records WHERE entity_id = $1::pg_catalog.uuid ON CONFLICT (source_record_id) DO UPDATE SET entity_id = EXCLUDED.entity_id, source_identity_id = EXCLUDED.source_identity_id, source_record_key = EXCLUDED.source_record_key, source_record_id = EXCLUDED.source_record_id, active = EXCLUDED.active WHERE target.entity_id IS DISTINCT FROM EXCLUDED.entity_id OR target.source_identity_id IS DISTINCT FROM EXCLUDED.source_identity_id OR target.source_record_key IS DISTINCT FROM EXCLUDED.source_record_key OR target.source_record_id IS DISTINCT FROM EXCLUDED.source_record_id OR target.active IS DISTINCT FROM EXCLUDED.active",
            None,
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    client
        .update(
            "INSERT INTO mdm_graph.definition_limits AS target (entity_id, entity_name, expanded_definition) SELECT e.entity_id, e.entity_name::text, d.expanded_definition FROM mdm_internal.entities e JOIN mdm_internal.definitions d ON d.entity_id = e.entity_id AND d.definition_version = e.desired_version WHERE e.entity_id = $1::pg_catalog.uuid ON CONFLICT (entity_id) DO UPDATE SET entity_name = EXCLUDED.entity_name, expanded_definition = EXCLUDED.expanded_definition WHERE target.entity_name IS DISTINCT FROM EXCLUDED.entity_name OR target.expanded_definition IS DISTINCT FROM EXCLUDED.expanded_definition",
            None,
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    let rows = client
        .select(
            "SELECT contract_version, graph_digest, contract FROM pgtrickle.graph_contract(ARRAY[$1::regclass])",
            Some(1),
            &[context.graph_root.clone().into()],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    if rows.is_empty() {
        return Err(MdmError::GraphContract(
            "graph root contract is missing".into(),
        ));
    }
    let contract = rows.first();
    let version = contract
        .get::<i16>(1)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("graph contract version is NULL".into()))?;
    let current_digest = contract
        .get::<Vec<u8>>(2)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("graph contract digest is NULL".into()))?;
    let current_contract = contract
        .get::<JsonB>(3)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("graph contract is NULL".into()))?
        .0;
    if version != 1
        || current_digest.len() != 32
        || stable_graph_contract(&current_contract)
            != stable_graph_contract(&context.graph_contract)
    {
        return Err(MdmError::GraphContract(
            "graph contract changed since installation".into(),
        ));
    }
    let rows = client
        .select(
            "SELECT contract_version, graph_refresh_id, graph_digest, source_boundary, source_boundary_digest, node_results FROM pgtrickle.refresh_graph_strict(ARRAY[$1::regclass], $2::bytea, $3::text)",
            Some(1),
            &[
                context.graph_root.clone().into(),
                current_digest.clone().into(),
                full_policy.into(),
            ],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    if rows.is_empty() {
        return Err(MdmError::RefreshFailed(
            "strict graph refresh returned no result".into(),
        ));
    }
    let row = rows.first();
    let version = row
        .get::<i16>(1)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("refresh contract version is NULL".into()))?;
    let id = row
        .get::<i64>(2)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("graph refresh ID is NULL".into()))?;
    let refresh_digest = row
        .get::<Vec<u8>>(3)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("refresh graph digest is NULL".into()))?;
    let boundary = row
        .get::<JsonB>(4)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("source boundary is NULL".into()))?;
    let boundary_digest = row
        .get::<Vec<u8>>(5)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("source boundary digest is NULL".into()))?;
    let node_results = row
        .get::<JsonB>(6)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("node results are NULL".into()))?;
    if version != 1 || refresh_digest != current_digest || boundary_digest.len() != 32 {
        return Err(MdmError::RefreshBoundary(
            "strict graph refresh returned invalid metadata".into(),
        ));
    }
    if boundary.0.get("completeness").and_then(Value::as_str) != Some("PROVEN") {
        return Err(MdmError::RefreshBoundary(
            "strict graph refresh did not prove a complete source boundary".into(),
        ));
    }
    Ok(GraphRefresh {
        id,
        boundary: boundary.0,
        boundary_digest,
        node_results: node_results.0,
    })
}

fn delta_relation_sql(relation: &str) -> Result<String, MdmError> {
    let mut parts = relation.split('.');
    let schema = parts.next().unwrap_or_default();
    let table = parts.next().unwrap_or_default();
    let expected_schema = ["pgtrickle", "changes"].join("_");
    if schema != expected_schema || table.is_empty() || parts.next().is_some() {
        return Err(MdmError::DeltaProtocol(format!(
            "invalid delta relation {relation}"
        )));
    }
    Ok(format!(
        "{}.{}",
        quote_identifier(schema),
        quote_identifier(table)
    ))
}

fn delta_registration(
    client: &mut SpiClient<'_>,
    relation_oid: pg_sys::Oid,
    binding_id: &str,
    logical_id: &str,
    contract_digest: &[u8],
    start_position: &str,
) -> Result<(Uuid, String, i16), MdmError> {
    let consumer_name = format!("pg_mdm:{binding_id}:{logical_id}");
    let rows = client
        .select(
            "SELECT consumer_id, delta_relation, output_contract_digest, row_identity_version FROM pgtrickle.register_output_delta_consumer($1::pg_catalog.oid, $2::text, $3::bytea, $4::text)",
            Some(1),
            &[
                relation_oid.into(),
                consumer_name.into(),
                contract_digest.to_vec().into(),
                start_position.into(),
            ],
        )
        .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
    if rows.is_empty() {
        return Err(MdmError::DeltaProtocol(format!(
            "registration returned no consumer for {logical_id}"
        )));
    }
    let row = rows.first();
    let consumer_id = row
        .get::<Uuid>(1)
        .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
        .ok_or_else(|| MdmError::DeltaProtocol("consumer ID is NULL".into()))?;
    let relation = row
        .get::<String>(2)
        .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
        .ok_or_else(|| MdmError::DeltaProtocol("delta relation is NULL".into()))?;
    let digest = row
        .get::<Vec<u8>>(3)
        .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
        .ok_or_else(|| MdmError::DeltaProtocol("delta digest is NULL".into()))?;
    let row_identity_version = row
        .get::<i16>(4)
        .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
        .ok_or_else(|| MdmError::DeltaProtocol("row identity version is NULL".into()))?;
    if digest != contract_digest || row_identity_version <= 0 {
        return Err(MdmError::DeltaProtocol(format!(
            "consumer contract for {logical_id} does not match the graph member"
        )));
    }
    Ok((consumer_id, relation, row_identity_version))
}

#[derive(Debug)]
struct DeltaStatus {
    delta_relation: String,
    state: String,
    acknowledged_token: i64,
    log_head: i64,
    lag: i64,
    contract_digest: Vec<u8>,
    row_identity_version: i16,
}

fn read_delta_status(
    client: &mut SpiClient<'_>,
    consumer_id: Uuid,
) -> Result<DeltaStatus, MdmError> {
    let rows = client
        .select(
            "SELECT delta_relation, state, acknowledged_batch_token, log_head, batch_lag, output_contract_digest, row_identity_version FROM pgtrickle.output_delta_consumer_status() WHERE consumer_id = $1::uuid",
            Some(1),
            &[consumer_id.into()],
        )
        .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
    if rows.is_empty() {
        return Err(MdmError::DeltaProtocol(format!(
            "consumer {consumer_id} is missing from pgtrickle status"
        )));
    }
    let row = rows.first();
    Ok(DeltaStatus {
        delta_relation: row
            .get::<String>(1)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("status relation is NULL".into()))?,
        state: row
            .get::<String>(2)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("status state is NULL".into()))?,
        acknowledged_token: row
            .get::<i64>(3)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("acknowledged token is NULL".into()))?,
        log_head: row
            .get::<i64>(4)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("log head is NULL".into()))?,
        lag: row
            .get::<i64>(5)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("batch lag is NULL".into()))?,
        contract_digest: row
            .get::<Vec<u8>>(6)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("status digest is NULL".into()))?,
        row_identity_version: row
            .get::<i16>(7)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("status row version is NULL".into()))?,
    })
}

fn ensure_delta_consumers(
    client: &mut SpiClient<'_>,
    context: &Context,
) -> Result<Vec<DeltaConsumer>, MdmError> {
    let members = client
        .select(
            "SELECT m.logical_id, m.relation_oid, m.contract_digest, c.consumer_id, c.delta_relation_name, c.row_identity_version FROM mdm_internal.graph_members m LEFT JOIN mdm_internal.graph_delta_consumers c ON c.graph_binding_id = m.graph_binding_id AND c.logical_id = m.logical_id WHERE m.graph_binding_id = $1::pg_catalog.uuid AND m.logical_id IN ($2, $3) ORDER BY m.logical_id",
            None,
            &[
                context.graph_binding_id.clone().into(),
                format!("evidence/{}", context.entity.name).into(),
                format!("golden/{}", context.entity.name).into(),
            ],
        )
        .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
    if members.len() != 2 {
        return Err(MdmError::DeltaProtocol(
            "evidence and golden graph members are required".into(),
        ));
    }
    let mut consumers = Vec::with_capacity(2);
    for row in members {
        let logical_id = row
            .get::<String>(1)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("logical ID is NULL".into()))?;
        let relation_oid = row
            .get::<pg_sys::Oid>(2)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("relation OID is NULL".into()))?;
        let member_digest = row
            .get::<Vec<u8>>(3)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
            .ok_or_else(|| MdmError::DeltaProtocol("member contract digest is NULL".into()))?;
        let (consumer_id, delta_relation, row_identity_version) = if let Some(consumer_id) = row
            .get::<Uuid>(4)
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
        {
            let delta_relation = row
                .get::<String>(5)
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                .ok_or_else(|| MdmError::DeltaProtocol("delta relation is NULL".into()))?;
            let row_identity_version = row
                .get::<i16>(6)
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                .ok_or_else(|| MdmError::DeltaProtocol("row identity version is NULL".into()))?;
            if row_identity_version <= 0 {
                return Err(MdmError::DeltaProtocol(format!(
                    "stored consumer row identity version for {logical_id} is invalid"
                )));
            }
            (consumer_id, delta_relation, row_identity_version)
        } else {
            let registered = delta_registration(
                client,
                relation_oid,
                &context.graph_binding_id,
                &logical_id,
                &member_digest,
                "RESNAPSHOT_REQUIRED",
            )?;
            client
                    .update(
                        "INSERT INTO mdm_internal.graph_delta_consumers (graph_binding_id, logical_id, consumer_id, delta_relation_name, row_identity_version) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5)",
                        None,
                        &[
                            context.graph_binding_id.clone().into(),
                            logical_id.clone().into(),
                            registered.0.into(),
                            registered.1.clone().into(),
                            registered.2.into(),
                        ],
                    )
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
            (registered.0, registered.1, registered.2)
        };
        let status = read_delta_status(client, consumer_id)?;
        if status.delta_relation != delta_relation
            || status.contract_digest != member_digest
            || status.row_identity_version != row_identity_version
        {
            return Err(MdmError::DeltaProtocol(format!(
                "status for {logical_id} does not match its catalog binding"
            )));
        }
        if !matches!(
            status.state.as_str(),
            "ACTIVE" | "RESNAPSHOT_REQUIRED" | "INVALIDATED"
        ) {
            return Err(MdmError::DeltaProtocol(format!(
                "unsupported state {} for {logical_id}",
                status.state
            )));
        }
        let mut consumer = DeltaConsumer {
            logical_id: logical_id.clone(),
            consumer_id,
            delta_relation,
            contract_digest: member_digest,
            row_identity_version,
            state: status.state,
            acknowledged_token: status.acknowledged_token,
            log_head: status.log_head,
            resnapshot_token: None,
            through_token: None,
            batch_count: 0,
            row_count: 0,
            saw_full_invalidation: false,
            affected_records: BTreeSet::new(),
            lag: status.lag,
        };
        if matches!(
            consumer.state.as_str(),
            "RESNAPSHOT_REQUIRED" | "INVALIDATED"
        ) {
            let rows = client
                .select(
                    "SELECT log_head, output_contract_digest, row_identity_version, resnapshot_token FROM pgtrickle.begin_output_delta_resnapshot($1::uuid)",
                    Some(1),
                    &[consumer.consumer_id.into()],
                )
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
            if rows.is_empty() {
                return Err(MdmError::DeltaProtocol(format!(
                    "resnapshot did not start for {logical_id}"
                )));
            }
            let row = rows.first();
            let log_head = row
                .get::<i64>(1)
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                .ok_or_else(|| MdmError::DeltaProtocol("resnapshot log head is NULL".into()))?;
            let digest = row
                .get::<Vec<u8>>(2)
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                .ok_or_else(|| MdmError::DeltaProtocol("resnapshot digest is NULL".into()))?;
            let version = row
                .get::<i16>(3)
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                .ok_or_else(|| MdmError::DeltaProtocol("resnapshot row version is NULL".into()))?;
            if digest != consumer.contract_digest || version != consumer.row_identity_version {
                return Err(MdmError::DeltaProtocol(format!(
                    "resnapshot contract for {logical_id} does not match the graph member"
                )));
            }
            consumer.log_head = log_head;
            consumer.through_token = Some(log_head);
            consumer.resnapshot_token = row
                .get::<Uuid>(4)
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
            if consumer.resnapshot_token.is_none() {
                return Err(MdmError::DeltaProtocol("resnapshot token is NULL".into()));
            }
        }
        consumers.push(consumer);
    }
    Ok(consumers)
}

fn read_delta_batches(
    client: &mut SpiClient<'_>,
    delta: &mut [DeltaConsumer],
) -> Result<(), MdmError> {
    for consumer in delta.iter_mut() {
        if consumer.resnapshot_token.is_some() || consumer.state != "ACTIVE" {
            continue;
        }
        let rows = client
            .select(
                "SELECT batch_token, row_count, rows_inserted, rows_deleted, mode, output_contract_digest, row_identity_version FROM pgtrickle.output_delta_batches($1::uuid, NULL::bigint) ORDER BY batch_token",
                None,
                &[consumer.consumer_id.into()],
            )
            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
        let relation = delta_relation_sql(&consumer.delta_relation)?;
        let payload_columns = match consumer.logical_id.as_str() {
            logical_id if logical_id.starts_with("evidence/") => {
                "left_source_record_id, right_source_record_id"
            }
            logical_id if logical_id.starts_with("golden/") => "source_record_id",
            logical_id => {
                return Err(MdmError::DeltaProtocol(format!(
                    "unsupported terminal logical ID {logical_id}"
                )));
            }
        };
        let mut expected = consumer
            .acknowledged_token
            .checked_add(1)
            .ok_or_else(|| MdmError::DeltaProtocol("acknowledged token is exhausted".into()))?;
        for row in rows {
            let batch = DeltaBatch {
                token: row
                    .get::<i64>(1)
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                    .ok_or_else(|| MdmError::DeltaProtocol("batch token is NULL".into()))?,
                row_count: row
                    .get::<i64>(2)
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                    .ok_or_else(|| MdmError::DeltaProtocol("batch row count is NULL".into()))?,
                rows_inserted: row
                    .get::<i64>(3)
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                    .ok_or_else(|| MdmError::DeltaProtocol("insert count is NULL".into()))?,
                rows_deleted: row
                    .get::<i64>(4)
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                    .ok_or_else(|| MdmError::DeltaProtocol("delete count is NULL".into()))?,
                mode: row
                    .get::<String>(5)
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                    .ok_or_else(|| MdmError::DeltaProtocol("batch mode is NULL".into()))?,
                contract_digest: row
                    .get::<Vec<u8>>(6)
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                    .ok_or_else(|| MdmError::DeltaProtocol("batch digest is NULL".into()))?,
                row_identity_version: row
                    .get::<i16>(7)
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                    .ok_or_else(|| MdmError::DeltaProtocol("batch row version is NULL".into()))?,
            };
            validate_delta_batch(
                &batch,
                expected,
                &consumer.contract_digest,
                consumer.row_identity_version,
            )?;
            let payload = client
                .select(
                    &format!(
                        "SELECT action, row_identity, {payload_columns} FROM {relation} WHERE batch_token = $1 ORDER BY ordinal"
                    ),
                    None,
                    &[batch.token.into()],
                )
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
            let mut decoded = Vec::with_capacity(payload.len());
            for payload_row in payload {
                let action = payload_row
                    .get::<String>(1)
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                    .ok_or_else(|| MdmError::DeltaProtocol("payload action is NULL".into()))?;
                let identity = payload_row
                    .get::<Vec<u8>>(2)
                    .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                    .ok_or_else(|| {
                        MdmError::DeltaProtocol("payload row identity is NULL".into())
                    })?;
                let source_record_ids = if payload_columns.contains("left_source_record_id") {
                    vec![
                        payload_row
                            .get::<Uuid>(3)
                            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                            .ok_or_else(|| {
                                MdmError::DeltaProtocol(
                                    "evidence left source record ID is NULL".into(),
                                )
                            })?,
                        payload_row
                            .get::<Uuid>(4)
                            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                            .ok_or_else(|| {
                                MdmError::DeltaProtocol(
                                    "evidence right source record ID is NULL".into(),
                                )
                            })?,
                    ]
                } else {
                    vec![
                        payload_row
                            .get::<Uuid>(3)
                            .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                            .ok_or_else(|| {
                                MdmError::DeltaProtocol("golden source record ID is NULL".into())
                            })?,
                    ]
                };
                decoded.push(DeltaRow {
                    action,
                    row_identity: identity,
                    source_record_ids,
                });
            }
            validate_delta_payload(&decoded, &batch)?;
            for row in &decoded {
                consumer
                    .affected_records
                    .extend(row.source_record_ids.iter().copied());
            }
            consumer.batch_count += 1;
            consumer.row_count += usize::try_from(batch.row_count)
                .map_err(|_| MdmError::DeltaProtocol("batch row count is too large".into()))?;
            consumer.saw_full_invalidation |= batch.mode == "FULL_INVALIDATION";
            consumer.through_token = Some(batch.token);
            consumer.lag = consumer.log_head.saturating_sub(batch.token);
            expected = batch
                .token
                .checked_add(1)
                .ok_or_else(|| MdmError::DeltaProtocol("batch token is exhausted".into()))?;
        }
    }
    Ok(())
}

fn finish_delta(client: &mut SpiClient<'_>, delta: &mut [DeltaConsumer]) -> Result<(), MdmError> {
    for consumer in delta.iter_mut() {
        if let Some(token) = consumer.resnapshot_token {
            let rows = client
                .select(
                    "SELECT pgtrickle.ack_output_delta_resnapshot($1::uuid, $2::uuid)",
                    Some(1),
                    &[consumer.consumer_id.into(), token.into()],
                )
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
            if rows.is_empty() {
                return Err(MdmError::DeltaProtocol(
                    "resnapshot acknowledgement returned no row".into(),
                ));
            }
            let disposition = rows
                .first()
                .get::<String>(1)
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                .ok_or_else(|| {
                    MdmError::DeltaProtocol("resnapshot acknowledgement is NULL".into())
                })?;
            if disposition != "ACTIVE" {
                return Err(MdmError::DeltaProtocol(format!(
                    "unexpected resnapshot acknowledgement {disposition}"
                )));
            }
            consumer.acknowledged_token = consumer
                .through_token
                .ok_or_else(|| MdmError::DeltaProtocol("resnapshot has no through token".into()))?;
            consumer.state = "ACTIVE".into();
        } else if let Some(through_token) = consumer.through_token {
            let disposition = if consumer.saw_full_invalidation {
                "RESYNCHRONIZED"
            } else {
                "APPLIED"
            };
            let rows = client
                .select(
                    "SELECT pgtrickle.ack_output_delta($1::uuid, $2::bigint, $3::text)",
                    Some(1),
                    &[
                        consumer.consumer_id.into(),
                        through_token.into(),
                        disposition.into(),
                    ],
                )
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?;
            if rows.is_empty() {
                return Err(MdmError::DeltaProtocol(
                    "delta acknowledgement returned no row".into(),
                ));
            }
            let returned_disposition = rows
                .first()
                .get::<String>(1)
                .map_err(|error| MdmError::DeltaProtocol(error.to_string()))?
                .ok_or_else(|| MdmError::DeltaProtocol("delta acknowledgement is NULL".into()))?;
            if returned_disposition != disposition {
                return Err(MdmError::DeltaProtocol(format!(
                    "expected acknowledgement {disposition}, got {returned_disposition}"
                )));
            }
            consumer.acknowledged_token = through_token;
        }
        consumer.lag = 0;
    }
    Ok(())
}

fn load_sources(
    client: &mut SpiClient<'_>,
    context: &Context,
    max_active_records: usize,
) -> Result<Vec<SourceRow>, MdmError> {
    let rows = load_sources_ordered(client, context, max_active_records.saturating_add(1))?;
    if rows.len() > max_active_records {
        return Err(MdmError::ResolverLimit {
            resource: "max_active_records",
            observed: rows.len(),
            limit: max_active_records,
        });
    }
    Ok(rows)
}

fn load_sources_ordered(
    client: &mut SpiClient<'_>,
    context: &Context,
    limit: usize,
) -> Result<Vec<SourceRow>, MdmError> {
    let limit = i64::try_from(limit).map_err(|_| {
        MdmError::ResolverInvalid("max_active_records cannot be represented as bigint".into())
    })?;
    let rows = client
        .select(
            "SELECT r.source_record_id, s.source_name::text, r.source_record_key FROM mdm_internal.source_records r JOIN mdm_internal.source_identities s ON s.source_identity_id = r.source_identity_id WHERE r.entity_id = $1::pg_catalog.uuid AND r.active ORDER BY r.source_record_key, r.source_record_id LIMIT $2::bigint",
            None,
            &[context.entity_id.clone().into(), limit.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    rows.into_iter()
        .map(|row| {
            Ok(SourceRow {
                id: row
                    .get::<Uuid>(1)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("source record ID is NULL".into()))?,
                source_name: row
                    .get::<String>(2)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("source name is NULL".into()))?,
                sort_key: row
                    .get::<Vec<u8>>(3)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("source record key is NULL".into()))?,
            })
        })
        .collect()
}

fn load_decisions(
    client: &mut SpiClient<'_>,
    context: &Context,
    active: &BTreeSet<Uuid>,
) -> Result<(Vec<DecisionEdge>, Vec<DecisionEdge>), MdmError> {
    let rows = client
        .select(
            "SELECT decision_id, left_source_record_id, right_source_record_id, decision FROM mdm_internal.steward_decisions WHERE entity_id = $1::pg_catalog.uuid AND is_current ORDER BY decision_id",
            None,
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let mut matches = Vec::new();
    let mut cannot = Vec::new();
    for row in rows {
        let left = row
            .get::<Uuid>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("decision left ID is NULL".into()))?;
        let right = row
            .get::<Uuid>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("decision right ID is NULL".into()))?;
        if !active.contains(&left) || !active.contains(&right) {
            continue;
        }
        let edge = DecisionEdge {
            decision_id: row
                .get::<Uuid>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("decision ID is NULL".into()))?,
            left_source_record_id: left,
            right_source_record_id: right,
            decision: DecisionKind::parse(
                &row.get::<String>(4)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("decision is NULL".into()))?,
            )?,
        }
        .canonical();
        match edge.decision {
            DecisionKind::Match => matches.push(edge),
            DecisionKind::NotMatch => cannot.push(edge),
        }
    }
    Ok((matches, cannot))
}

fn load_pair_decisions(
    client: &mut SpiClient<'_>,
    context: &Context,
    active: &BTreeSet<Uuid>,
    source_names: &BTreeMap<Uuid, String>,
    scope: Option<&BTreeSet<Uuid>>,
) -> Result<Vec<crate::pair::PairDecision>, MdmError> {
    let max_candidate_pairs = context
        .entity
        .limits
        .get("max_candidate_pairs")
        .map(|value| crate::semantics::validate_limit_value("max_candidate_pairs", value))
        .transpose()?
        .unwrap_or_else(|| crate::semantics::candidate_limits().max_candidate_pairs);
    let (max_rows, fetch_limit) =
        pair_evidence_fetch_limit(max_candidate_pairs, context.entity.matches.len())?;
    let query = format!(
        "SELECT left_source_record_id, right_source_record_id, left_sort_key, right_sort_key, rule, evidence_group, class, score, comparator, comparator_version, left_value_digest, right_value_digest FROM {}{} ORDER BY left_sort_key, right_sort_key, rule LIMIT $1::bigint",
        context.evidence_relation,
        scope.map_or_else(String::new, |_| {
            " WHERE left_source_record_id IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($2::pg_catalog.jsonb)) OR right_source_record_id IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($2::pg_catalog.jsonb))".into()
        })
    );
    let mut query_args = vec![fetch_limit.into()];
    if let Some(scope) = scope {
        query_args.push(uuid_json(scope).into());
    }
    let rows = client
        .select(&query, None, &query_args)
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if rows.len() > max_rows {
        return Err(MdmError::ResolverLimit {
            resource: "max_pair_evidence_rows",
            observed: rows.len(),
            limit: max_rows,
        });
    }
    let mut grouped: BTreeMap<(Uuid, Uuid), PairEvidence> = BTreeMap::new();
    for row in rows {
        let left = row
            .get::<Uuid>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("evidence left ID is NULL".into()))?;
        let right = row
            .get::<Uuid>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("evidence right ID is NULL".into()))?;
        if !active.is_empty() && (!active.contains(&left) || !active.contains(&right)) {
            continue;
        }
        let left_key = row
            .get::<Vec<u8>>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("evidence left sort key is NULL".into()))?;
        let right_key = row
            .get::<Vec<u8>>(4)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("evidence right sort key is NULL".into()))?;
        let pair = CandidatePair {
            left_source_record_id: left,
            right_source_record_id: right,
            left_sort_key: left_key,
            right_sort_key: right_key,
            discovery_channels: Vec::new(),
        };
        let class = match row
            .get::<String>(7)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .as_deref()
        {
            Some("agree") => EvidenceClass::Agree,
            Some("disagree") => EvidenceClass::Disagree,
            Some("no_evidence") => EvidenceClass::NoEvidence,
            Some(other) => {
                return Err(MdmError::EvidenceInvalid(format!(
                    "unknown evidence class {other}"
                )));
            }
            None => return Err(MdmError::Spi("evidence class is NULL".into())),
        };
        let item = EvidenceItem {
            rule: row
                .get::<String>(5)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("evidence rule is NULL".into()))?,
            evidence_group: row
                .get::<String>(6)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("evidence group is NULL".into()))?,
            class,
            score: row
                .get::<i16>(8)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .and_then(|value| u16::try_from(value).ok()),
            comparator: row
                .get::<String>(9)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("comparator is NULL".into()))?,
            comparator_version: row
                .get::<i16>(10)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("comparator version is NULL".into()))
                .and_then(|value| {
                    u16::try_from(value).map_err(|_| {
                        MdmError::EvidenceInvalid("comparator version is negative".into())
                    })
                })?,
            left_value_digest: row
                .get::<Vec<u8>>(11)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .map(|value| value.try_into())
                .transpose()
                .map_err(|_| {
                    MdmError::EvidenceInvalid("left value digest is not 32 bytes".into())
                })?,
            right_value_digest: row
                .get::<Vec<u8>>(12)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .map(|value| value.try_into())
                .transpose()
                .map_err(|_| {
                    MdmError::EvidenceInvalid("right value digest is not 32 bytes".into())
                })?,
        };
        let key = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        let entry = grouped.entry(key).or_insert((pair, Vec::new()));
        entry.1.push(item);
    }
    Ok(grouped
        .into_values()
        .map(|(pair, evidence)| {
            let authority_conflict = source_names
                .get(&pair.left_source_record_id)
                .zip(source_names.get(&pair.right_source_record_id))
                .is_some_and(|(left, right)| {
                    let left = evaluation::source_authority(&context.entity, left);
                    let right = evaluation::source_authority(&context.entity, right);
                    left.iter().any(|(field, left_value)| {
                        right
                            .get(field)
                            .is_some_and(|right_value| left_value != right_value)
                    })
                });
            decide_pair_for_rules(
                pair,
                evidence,
                &context.entity.matches,
                authority_conflict,
                None,
            )
        })
        .collect())
}

fn pair_evidence_fetch_limit(
    max_candidate_pairs: usize,
    rule_count: usize,
) -> Result<(usize, i64), MdmError> {
    let max_rows = max_candidate_pairs
        .checked_mul(rule_count.max(1))
        .ok_or_else(|| MdmError::ResolverInvalid("pair evidence row limit overflow".into()))?;
    let fetch_limit = max_rows
        .checked_add(1)
        .and_then(|limit| i64::try_from(limit).ok())
        .ok_or_else(|| MdmError::ResolverInvalid("pair evidence row limit overflow".into()))?;
    Ok((max_rows, fetch_limit))
}

fn uuid_json(ids: &BTreeSet<Uuid>) -> JsonB {
    JsonB(json!(
        ids.iter().map(ToString::to_string).collect::<Vec<_>>()
    ))
}

fn load_sources_by_ids(
    client: &SpiClient<'_>,
    context: &Context,
    ids: &BTreeSet<Uuid>,
) -> Result<Vec<SourceRow>, MdmError> {
    let rows = client
        .select(
            "SELECT r.source_record_id, s.source_name::text, r.source_record_key FROM mdm_internal.source_records r JOIN mdm_internal.source_identities s ON s.source_identity_id = r.source_identity_id WHERE r.entity_id = $1::pg_catalog.uuid AND r.active AND r.source_record_id IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($2::pg_catalog.jsonb)) ORDER BY r.source_record_key, r.source_record_id",
            None,
            &[context.entity_id.clone().into(), uuid_json(ids).into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    rows.into_iter()
        .map(|row| {
            Ok(SourceRow {
                id: row
                    .get::<Uuid>(1)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("source record ID is NULL".into()))?,
                source_name: row
                    .get::<String>(2)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("source name is NULL".into()))?,
                sort_key: row
                    .get::<Vec<u8>>(3)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("source record key is NULL".into()))?,
            })
        })
        .collect()
}

fn load_decisions_for_scope(
    client: &SpiClient<'_>,
    context: &Context,
    scope: &BTreeSet<Uuid>,
) -> Result<(Vec<DecisionEdge>, Vec<DecisionEdge>), MdmError> {
    let rows = client
        .select(
            "SELECT d.decision_id, d.left_source_record_id, d.right_source_record_id, d.decision FROM mdm_internal.steward_decisions d JOIN mdm_internal.source_records l ON l.entity_id = d.entity_id AND l.source_record_id = d.left_source_record_id AND l.active JOIN mdm_internal.source_records r ON r.entity_id = d.entity_id AND r.source_record_id = d.right_source_record_id AND r.active WHERE d.entity_id = $1::pg_catalog.uuid AND d.is_current AND (d.left_source_record_id IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($2::pg_catalog.jsonb)) OR d.right_source_record_id IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($2::pg_catalog.jsonb))) ORDER BY d.decision_id",
            None,
            &[context.entity_id.clone().into(), uuid_json(scope).into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let mut matches = Vec::new();
    let mut cannot = Vec::new();
    for row in rows {
        let edge = DecisionEdge {
            decision_id: row
                .get::<Uuid>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("decision ID is NULL".into()))?,
            left_source_record_id: row
                .get::<Uuid>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("decision left ID is NULL".into()))?,
            right_source_record_id: row
                .get::<Uuid>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("decision right ID is NULL".into()))?,
            decision: DecisionKind::parse(
                &row.get::<String>(4)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("decision is NULL".into()))?,
            )?,
        }
        .canonical();
        match edge.decision {
            DecisionKind::Match => matches.push(edge),
            DecisionKind::NotMatch => cannot.push(edge),
        }
    }
    Ok((matches, cannot))
}

#[allow(clippy::useless_conversion)]
fn load_old_identity(
    client: &mut SpiClient<'_>,
    context: &Context,
) -> Result<IdentityState, MdmError> {
    let registry = client
        .select("SELECT mdm_id, created_revision, retired_revision, status FROM mdm_internal.identity_registry WHERE entity_id = $1::pg_catalog.uuid", None, &[context.entity_id.clone().into()])
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .into_iter()
        .map(|row| {
            let status = match row.get::<String>(4).map_err(|error| MdmError::Spi(error.to_string()))?.as_deref() {
                Some("active") => identity::IdentityStatus::Active,
                Some("merged") => identity::IdentityStatus::Merged,
                Some("split") => identity::IdentityStatus::Split,
                Some("retired") => identity::IdentityStatus::Retired,
                _ => return Err(MdmError::IdentityInvalid("unknown identity status".into())),
            };
            Ok(identity::IdentityRecord { mdm_id: row.get::<Uuid>(1).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("mdm ID is NULL".into()))?, created_revision: row.get::<i64>(2).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("identity creation revision is NULL".into()))?, retired_revision: row.get::<i64>(3).map_err(|error| MdmError::Spi(error.to_string()))?, status })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let memberships = client
        .select("SELECT m.source_record_id, r.source_record_key, m.mdm_id, m.active, m.first_membership_revision, m.last_membership_revision, m.membership_reason, m.last_change_revision FROM mdm_internal.memberships m JOIN mdm_internal.source_records r ON r.entity_id = m.entity_id AND r.source_record_id = m.source_record_id WHERE m.entity_id = $1::pg_catalog.uuid", None, &[context.entity_id.clone().into()])
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .into_iter()
        .map(|row| Ok(identity::IdentityMembership { source_record_id: row.get::<Uuid>(1).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("membership source ID is NULL".into()))?, source_sort_key: row.get::<Vec<u8>>(2).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("membership sort key is NULL".into()))?, mdm_id: row.get::<Uuid>(3).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("membership MDM ID is NULL".into()))?, active: row.get::<bool>(4).map_err(|error| MdmError::Spi(error.to_string()))?.unwrap_or(false), first_membership_revision: row.get::<i64>(5).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("membership first revision is NULL".into()))?, last_membership_revision: row.get::<i64>(6).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("membership last revision is NULL".into()))?, membership_reason: row.get::<String>(7).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("membership reason is NULL".into()))?, last_change_revision: row.get::<i64>(8).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("membership change revision is NULL".into()))? }))
        .collect::<Result<Vec<_>, _>>()?;
    let aliases = client
        .select("SELECT alias_mdm_id, canonical_mdm_id, publication_revision FROM mdm_internal.identity_aliases WHERE entity_id = $1::pg_catalog.uuid", None, &[context.entity_id.clone().into()])
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .into_iter()
        .map(|row| Ok(identity::IdentityAlias { alias_mdm_id: row.get::<Uuid>(1).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("alias ID is NULL".into()))?, canonical_mdm_id: row.get::<Uuid>(2).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("canonical ID is NULL".into()))?, publication_revision: row.get::<i64>(3).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("alias revision is NULL".into()))? }))
        .collect::<Result<Vec<_>, _>>()?;
    let splits = client
        .select("SELECT parent_mdm_id, child_mdm_id, publication_revision FROM mdm_internal.identity_splits WHERE entity_id = $1::pg_catalog.uuid", None, &[context.entity_id.clone().into()])
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .into_iter()
        .map(|row| Ok(identity::IdentitySplit { parent_mdm_id: row.get::<Uuid>(1).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("split parent ID is NULL".into()))?, child_mdm_id: row.get::<Uuid>(2).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("split child ID is NULL".into()))?, publication_revision: row.get::<i64>(3).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("split revision is NULL".into()))? }))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(IdentityState {
        registry,
        memberships,
        aliases,
        splits,
    })
}

#[allow(clippy::useless_conversion)]
fn load_reviews(client: &mut SpiClient<'_>, context: &Context) -> Result<Vec<Review>, MdmError> {
    client
        .select("SELECT review_id, issue_key, occurrence, status, severity, reason_code, subjects, masked_summary, opened_revision, resolved_revision, last_change_revision, concurrency_version FROM mdm_internal.reviews WHERE entity_id = $1::pg_catalog.uuid", None, &[context.entity_id.clone().into()])
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .into_iter()
        .map(|row| Ok(Review { review_id: row.get::<Uuid>(1).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review ID is NULL".into()))?, issue_key: row.get::<Vec<u8>>(2).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review issue key is NULL".into()))?.try_into().map_err(|_| MdmError::Spi("review issue key is not 32 bytes".into()))?, occurrence: row.get::<i32>(3).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review occurrence is NULL".into()))?, status: match row.get::<String>(4).map_err(|error| MdmError::Spi(error.to_string()))?.as_deref() { Some("open") => ReviewStatus::Open, Some("resolved") => ReviewStatus::Resolved, _ => return Err(MdmError::Spi("review status is invalid".into())) }, severity: row.get::<String>(5).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review severity is NULL".into()))?, reason_code: row.get::<String>(6).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review reason is NULL".into()))?, subjects: row.get::<JsonB>(7).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review subjects are NULL".into()))?.0, masked_summary: row.get::<JsonB>(8).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review summary is NULL".into()))?.0, opened_revision: row.get::<i64>(9).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review opened revision is NULL".into()))?, resolved_revision: row.get::<i64>(10).map_err(|error| MdmError::Spi(error.to_string()))?, last_change_revision: row.get::<i64>(11).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review change revision is NULL".into()))?, concurrency_version: row.get::<i64>(12).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::Spi("review version is NULL".into()))? }))
        .collect()
}

#[allow(clippy::useless_conversion)]
fn load_golden_rows(
    client: &mut SpiClient<'_>,
    context: &Context,
) -> Result<Vec<GoldenRow>, MdmError> {
    load_golden_rows_scope(client, context, None)
}

fn load_golden_rows_scope(
    client: &SpiClient<'_>,
    context: &Context,
    scope: Option<&BTreeSet<Uuid>>,
) -> Result<Vec<GoldenRow>, MdmError> {
    let scope_filter = scope.map_or_else(String::new, |_| {
        " WHERE source_record_id IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($1::pg_catalog.jsonb))".into()
    });
    let query = format!(
        "SELECT source_record_id, source_name, field_name, raw_value::text, extract(epoch FROM row_changed_at)::bigint, state, normalized, canonical_bytes FROM {}{}",
        context.golden_relation, scope_filter
    );
    let args = scope.map_or_else(Vec::new, |scope| vec![uuid_json(scope).into()]);
    client
        .select(&query, None, &args)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .map(|row| {
            Ok(GoldenRow {
                source_record_id: row
                    .get::<Uuid>(1)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("golden source ID is NULL".into()))?,
                source_name: row
                    .get::<String>(2)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("golden source name is NULL".into()))?,
                field: row
                    .get::<String>(3)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("golden field is NULL".into()))?,
                raw_value: row
                    .get::<String>(4)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .map(|value| serde_json::from_str(&value).unwrap_or(Value::String(value))),
                row_changed_at: row
                    .get::<i64>(5)
                    .map_err(|error| MdmError::Spi(error.to_string()))?,
                state: normalized_state(
                    &row.get::<String>(6)
                        .map_err(|error| MdmError::Spi(error.to_string()))?
                        .ok_or_else(|| MdmError::Spi("golden state is NULL".into()))?,
                )?,
                normalized: row
                    .get::<String>(7)
                    .map_err(|error| MdmError::Spi(error.to_string()))?,
                canonical_bytes: row
                    .get::<Vec<u8>>(8)
                    .map_err(|error| MdmError::Spi(error.to_string()))?,
            })
        })
        .collect()
}

fn load_overrides(
    client: &mut SpiClient<'_>,
    context: &Context,
) -> Result<BTreeMap<String, Vec<GoldenOverride>>, MdmError> {
    let rows = client.select("SELECT field_name::text, override_id, anchor_source_record_id, (extract(epoch FROM created_at) * 1000000)::bigint, value FROM mdm_internal.golden_override_directives WHERE entity_id = $1::pg_catalog.uuid AND is_current AND action = 'SET' ORDER BY field_name, created_at, override_id", None, &[context.entity_id.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
    let mut result: BTreeMap<String, Vec<GoldenOverride>> = BTreeMap::new();
    for row in rows {
        let field = row
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("override field is NULL".into()))?;
        let value = row
            .get::<JsonB>(5)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("override value is NULL".into()))?
            .0;
        let normalized = value.as_str().map(str::to_owned);
        result.entry(field).or_default().push(GoldenOverride {
            anchor_source_record_id: row
                .get::<Uuid>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("override anchor is NULL".into()))?,
            directive_id: row
                .get::<Uuid>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("override ID is NULL".into()))?,
            created_at: row
                .get::<i64>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or_default(),
            raw_value: value,
            normalized: normalized.clone(),
            canonical_bytes: normalized.map(|value| value.into_bytes()),
        });
    }
    Ok(result)
}

fn load_limits(context: &Context) -> ResolverLimits {
    let limits = &context.entity.limits;
    let get = |name: &str, default: usize| {
        limits
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(default)
    };
    ResolverLimits {
        max_active_records: get(
            "max_active_records",
            ResolverLimits::default().max_active_records,
        ),
        max_automatic_edges: get(
            "max_automatic_edges",
            ResolverLimits::default().max_automatic_edges,
        ),
        max_records_per_component: get(
            "max_records_per_component",
            ResolverLimits::default().max_records_per_component,
        ),
        max_component_checks: get(
            "max_component_checks",
            ResolverLimits::default().max_component_checks,
        ),
    }
}

fn load_control_changes(
    client: &SpiClient<'_>,
    context: &Context,
) -> Result<ControlChanges, MdmError> {
    let publication = client
        .select(
            "SELECT decision_epoch FROM mdm_internal.publications WHERE entity_id = $1::pg_catalog.uuid AND publication_revision = $2",
            Some(1),
            &[
                context.entity_id.clone().into(),
                context.publication_revision.into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let previous_epoch = if publication.is_empty() {
        None
    } else {
        publication
            .first()
            .get::<i64>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
    };
    let Some(previous_epoch) = previous_epoch else {
        return Ok(ControlChanges {
            reconstructable: context.publication_revision == 0 && context.decision_epoch == 0,
            changed: context.decision_epoch != 0,
            ..ControlChanges::default()
        });
    };
    if previous_epoch > context.decision_epoch {
        return Ok(ControlChanges {
            reconstructable: false,
            changed: true,
            ..ControlChanges::default()
        });
    }

    let mut changes = ControlChanges {
        reconstructable: true,
        changed: previous_epoch != context.decision_epoch,
        ..ControlChanges::default()
    };
    let mut epochs = BTreeSet::new();
    let decision_rows = client
        .select(
            "SELECT decision_epoch, left_source_record_id, right_source_record_id FROM mdm_internal.steward_decisions WHERE entity_id = $1::pg_catalog.uuid AND decision_epoch > $2 AND decision_epoch <= $3 ORDER BY decision_epoch, decision_id",
            None,
            &[
                context.entity_id.clone().into(),
                previous_epoch.into(),
                context.decision_epoch.into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    for row in decision_rows {
        let epoch = row
            .get::<i64>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("decision epoch is NULL".into()))?;
        changes.reconstructable &= epochs.insert(epoch);
        for index in [2, 3] {
            changes.seeds.insert(
                row.get::<Uuid>(index)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("decision endpoint is NULL".into()))?,
            );
        }
    }
    let override_rows = client
        .select(
            "SELECT decision_epoch, anchor_source_record_id FROM mdm_internal.golden_override_directives WHERE entity_id = $1::pg_catalog.uuid AND decision_epoch > $2 AND decision_epoch <= $3 ORDER BY decision_epoch, override_id",
            None,
            &[
                context.entity_id.clone().into(),
                previous_epoch.into(),
                context.decision_epoch.into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    for row in override_rows {
        let epoch = row
            .get::<i64>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("override epoch is NULL".into()))?;
        changes.reconstructable &= epochs.insert(epoch);
        changes.seeds.insert(
            row.get::<Uuid>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("override anchor is NULL".into()))?,
        );
    }
    let mut epoch = previous_epoch;
    while epoch < context.decision_epoch {
        epoch = epoch
            .checked_add(1)
            .ok_or_else(|| MdmError::OperationState("decision epoch exhausted".into()))?;
        if !epochs.contains(&epoch) {
            changes.reconstructable = false;
        }
    }
    Ok(changes)
}

fn graph_generation_transition(
    client: &SpiClient<'_>,
    context: &Context,
) -> Result<bool, MdmError> {
    if context.publication_revision == 0 {
        return Ok(false);
    }
    let rows = client
        .select(
            "SELECT artifact_id::text FROM mdm_internal.publication_observations WHERE entity_id = $1::pg_catalog.uuid ORDER BY observed_at DESC, observation_id DESC",
            Some(1),
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if rows.is_empty() {
        return Ok(true);
    }
    let Some(artifact_id) = rows
        .first()
        .get::<String>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
    else {
        return Ok(true);
    };
    Ok(artifact_id != context.artifact_id)
}

fn allocate_ids(client: &mut SpiClient<'_>, count: usize) -> Result<Vec<Uuid>, MdmError> {
    (0..count)
        .map(|_| {
            SpiClient::select(client, "SELECT pg_catalog.uuidv7()", Some(1), &[])
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .first()
                .get::<Uuid>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::OperationState("uuidv7 returned NULL".into()))
        })
        .collect()
}

fn load_current_golden(client: &SpiClient<'_>, context: &Context) -> Result<Value, MdmError> {
    let rows = client
        .select(
            "SELECT mdm_id, field_name::text, value, normalized_value, status, winning_source_record_id, policy, policy_version, tie_break, contributors FROM mdm_internal.golden_provenance WHERE entity_id = $1::pg_catalog.uuid AND publication_revision = $2 ORDER BY mdm_id, field_name",
            None,
            &[
                context.entity_id.clone().into(),
                context.publication_revision.into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let mut values = Vec::new();
    for row in rows {
        values.push(json!([
            row.get::<Uuid>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .map(|id| id.to_string()),
            row.get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?,
            row.get::<JsonB>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .map(|value| value.0),
            row.get::<String>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?,
            row.get::<String>(5)
                .map_err(|error| MdmError::Spi(error.to_string()))?,
            row.get::<Uuid>(6)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .map(|id| id.to_string()),
            row.get::<String>(7)
                .map_err(|error| MdmError::Spi(error.to_string()))?,
            row.get::<i16>(8)
                .map_err(|error| MdmError::Spi(error.to_string()))?,
            row.get::<String>(9)
                .map_err(|error| MdmError::Spi(error.to_string()))?,
            row.get::<JsonB>(10)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .map(|value| value.0),
        ]));
    }
    Ok(Value::Array(values))
}

fn load_current_resolution_facts(
    client: &SpiClient<'_>,
    context: &Context,
) -> Result<Value, MdmError> {
    let rows = client
        .select(
            "SELECT pg_catalog.convert_from(subject_key, 'UTF8'), fact_kind, fact->>'reason_code', fact->'evidence_groups' FROM mdm_internal.resolution_facts WHERE entity_id = $1::pg_catalog.uuid AND publication_revision = $2 AND subject_kind = 'pair'",
            None,
            &[
                context.entity_id.clone().into(),
                context.publication_revision.into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let mut facts = Vec::with_capacity(rows.len());
    for row in rows {
        let subject_key = row
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("resolution fact subject is NULL".into()))?;
        let kind = row
            .get::<String>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("resolution fact kind is NULL".into()))?;
        let reason = row
            .get::<String>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("resolution fact reason is NULL".into()))?;
        let groups = row
            .get::<JsonB>(4)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("resolution fact evidence groups are NULL".into()))?
            .0
            .as_array()
            .ok_or_else(|| MdmError::Spi("resolution fact evidence groups are invalid".into()))?
            .iter()
            .map(|value| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    MdmError::Spi("resolution fact evidence group is not text".into())
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        facts.push((subject_key, kind, reason, groups));
    }
    facts.sort();
    Ok(json!(facts))
}

fn scope_resolution_facts(facts: &Value, source_record_ids: &BTreeSet<Uuid>) -> Value {
    let selected = source_record_ids
        .iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let rows = facts
        .as_array()
        .expect("resolution fact projection is an array")
        .iter()
        .filter(|row| {
            row.get(0)
                .and_then(Value::as_str)
                .and_then(|key| key.split_once(':'))
                .is_some_and(|(left, right)| selected.contains(left) && selected.contains(right))
        })
        .cloned()
        .collect::<Vec<_>>();
    Value::Array(rows)
}

fn start_operation(
    client: &mut SpiClient<'_>,
    context: &Context,
    kind: &str,
    actor: &catalog::Role,
    session: &catalog::Role,
) -> Result<String, MdmError> {
    client.update("INSERT INTO mdm_internal.operations (operation_kind, entity_name, status, outcome, actor_name, actor_role_name) VALUES ($1, $2, 'running', $3, $4, $5) RETURNING operation_id::text", Some(1), &[kind.into(), context.entity.name.clone().into(), JsonB(json!({"definition_version": context.definition_version})).into(), session.name.clone().into(), actor.name.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?.first().get::<String>(1).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::OperationState("operation ID is NULL".into()))
}

fn complete_operation(
    client: &mut SpiClient<'_>,
    operation_id: &str,
    result: &RefreshResult,
) -> Result<(), MdmError> {
    client.update("UPDATE mdm_internal.operations SET status = 'succeeded', result_code = 'MDM_OK', outcome = $2, completed_at = pg_catalog.clock_timestamp() WHERE operation_id = $1::pg_catalog.uuid AND status = 'running'", Some(1), &[operation_id.into(), JsonB(serde_json::to_value(result).map_err(|error| MdmError::OperationState(error.to_string()))?).into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
    Ok(())
}

fn ensure_output(
    client: &mut SpiClient<'_>,
    context: &Context,
) -> Result<Vec<OutputField>, MdmError> {
    let fields = context
        .entity
        .golden_values
        .iter()
        .enumerate()
        .map(|(index, golden)| OutputField {
            name: golden.field.clone(),
            ordinal: (index + 1) as u16,
            type_name: context
                .entity
                .fields
                .iter()
                .find(|field| field.name == golden.field)
                .map(|field| field.logical_type.clone())
                .unwrap_or_else(|| "text".into()),
        })
        .collect::<Vec<_>>();
    let existing = client.select("SELECT field_name::text, field_ordinal, field_type_name FROM mdm_internal.output_fields WHERE entity_id = $1::pg_catalog.uuid ORDER BY field_ordinal", None, &[context.entity_id.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
    let previous = existing
        .into_iter()
        .map(|row| {
            Ok(OutputField {
                name: row
                    .get::<String>(1)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("output field name is NULL".into()))?,
                ordinal: row
                    .get::<i16>(2)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("output field ordinal is NULL".into()))?
                    as u16,
                type_name: row
                    .get::<String>(3)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("output field type is NULL".into()))?,
            })
        })
        .collect::<Result<Vec<_>, MdmError>>()?;
    output::validate_append_only(&previous, &fields)?;
    for field in fields.iter().skip(previous.len()) {
        client.update("INSERT INTO mdm_internal.output_fields (entity_id, field_name, field_ordinal, field_type_name, created_definition_version) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5) ON CONFLICT DO NOTHING", None, &[context.entity_id.clone().into(), field.name.clone().into(), (field.ordinal as i16).into(), field.type_name.clone().into(), context.definition_version.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
    }
    let output_name = client.select("SELECT output_name::text FROM mdm_internal.output_names WHERE entity_id = $1::pg_catalog.uuid AND output_kind = 'entity'", Some(1), &[context.entity_id.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?.first().get::<String>(1).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::OutputInvalid("entity output name is missing".into()))?;
    let ddl = output::output_table_ddl(&output_name, &fields)?;
    let exists = client
        .select(
            "SELECT pg_catalog.to_regclass($1)",
            Some(1),
            &[format!("mdm_out.{output_name}").into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .first()
        .get::<pg_sys::Oid>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .is_some();
    if !exists {
        for statement in ddl {
            client
                .update(&statement, None, &[])
                .map_err(|error| MdmError::OutputInvalid(error.to_string()))?;
        }
    } else {
        for field in fields.iter().skip(previous.len()) {
            client
                .update(
                    &format!(
                        "ALTER TABLE mdm_out.{} ADD COLUMN {} {}",
                        quote_identifier(&output_name),
                        quote_identifier(&field.name),
                        field.type_name
                    ),
                    None,
                    &[],
                )
                .map_err(|error| MdmError::OutputInvalid(error.to_string()))?;
        }
    }
    Ok(fields)
}

fn delete_stale_output_rows(
    client: &mut SpiClient<'_>,
    table: &str,
    key_column: &str,
    desired: &BTreeSet<Uuid>,
    scope: Option<&BTreeSet<Uuid>>,
) -> Result<(), MdmError> {
    if let Some(scope) = scope {
        client
            .update(
                &format!(
                    "DELETE FROM mdm_out.{} AS target WHERE target.{} IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($1::pg_catalog.jsonb)) AND NOT EXISTS (SELECT FROM pg_catalog.jsonb_array_elements_text($2::pg_catalog.jsonb) AS desired(value) WHERE desired.value::uuid = target.{})",
                    quote_identifier(table),
                    quote_identifier(key_column),
                    quote_identifier(key_column),
                ),
                None,
                &[uuid_json(scope).into(), uuid_json(desired).into()],
            )
            .map_err(|error| MdmError::OutputInvalid(error.to_string()))?;
        return Ok(());
    }
    let rows = client
        .select(
            &format!(
                "SELECT {} FROM mdm_out.{}",
                quote_identifier(key_column),
                quote_identifier(table)
            ),
            None,
            &[],
        )
        .map_err(|error| MdmError::OutputInvalid(error.to_string()))?;
    for row in rows {
        let id = row
            .get::<Uuid>(1)
            .map_err(|error| MdmError::OutputInvalid(error.to_string()))?
            .ok_or_else(|| MdmError::OutputInvalid("output key is NULL".into()))?;
        if !desired.contains(&id) {
            client
                .update(
                    &format!(
                        "DELETE FROM mdm_out.{} WHERE {} = $1",
                        quote_identifier(table),
                        quote_identifier(key_column),
                    ),
                    None,
                    &[id.into()],
                )
                .map_err(|error| MdmError::OutputInvalid(error.to_string()))?;
        }
    }
    Ok(())
}

fn persist_publication_snapshots(
    client: &mut SpiClient<'_>,
    context: &Context,
    revision: i64,
    golden: &BTreeMap<(Uuid, String), GoldenSelection>,
    new_facts: &Value,
    scope: Option<(&BTreeSet<Uuid>, &BTreeSet<Uuid>)>,
) -> Result<(), MdmError> {
    if let Some((_, mdm_scope)) = scope {
        client
            .update(
                "INSERT INTO mdm_internal.golden_provenance (entity_id, publication_revision, mdm_id, field_name, value, normalized_value, status, winning_source_record_id, policy, policy_version, tie_break, contributors, definition_version) SELECT entity_id, $2, mdm_id, field_name, value, normalized_value, status, winning_source_record_id, policy, policy_version, tie_break, contributors, definition_version FROM mdm_internal.golden_provenance WHERE entity_id = $1::pg_catalog.uuid AND publication_revision = $3 AND NOT (mdm_id IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($4::pg_catalog.jsonb)))",
                None,
                &[
                    context.entity_id.clone().into(),
                    revision.into(),
                    context.publication_revision.into(),
                    uuid_json(mdm_scope).into(),
                ],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
    }
    for ((mdm_id, field), selection) in golden {
        client
            .update("INSERT INTO mdm_internal.golden_provenance (entity_id, publication_revision, mdm_id, field_name, value, normalized_value, status, winning_source_record_id, policy, policy_version, tie_break, contributors, definition_version) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)", None, &[context.entity_id.clone().into(), revision.into(), (*mdm_id).into(), field.clone().into(), selection.value.clone().map(JsonB).into(), selection.normalized.clone().into(), selection.status.as_str().into(), selection.winning_source_record_id.into(), selection.policy.clone().into(), (selection.policy_version as i16).into(), selection.tie_break.clone().into(), JsonB(json!(selection.contributors.iter().map(ToString::to_string).collect::<Vec<_>>())).into(), context.definition_version.into()])
            .map_err(|error| MdmError::Spi(error.to_string()))?;
    }

    let (query, args) = if let Some((records, _)) = scope {
        (
            "WITH facts AS (SELECT pg_catalog.jsonb_build_array(pg_catalog.convert_from(f.subject_key, 'UTF8'), f.fact_kind, f.fact->>'reason_code', f.fact->'evidence_groups') AS fact FROM mdm_internal.resolution_facts f WHERE f.entity_id = $1::pg_catalog.uuid AND f.publication_revision = $2 AND NOT (pg_catalog.split_part(pg_catalog.convert_from(f.subject_key, 'UTF8'), ':', 1)::uuid IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($4::pg_catalog.jsonb)) AND pg_catalog.split_part(pg_catalog.convert_from(f.subject_key, 'UTF8'), ':', 2)::uuid IN (SELECT value::uuid FROM pg_catalog.jsonb_array_elements_text($4::pg_catalog.jsonb))) UNION ALL SELECT value AS fact FROM pg_catalog.jsonb_array_elements($3::pg_catalog.jsonb)), numbered AS (SELECT pg_catalog.row_number() OVER (ORDER BY fact->>0, fact->>1, fact->>2, fact->>3) AS fact_number, fact FROM facts) INSERT INTO mdm_internal.resolution_facts (entity_id, publication_revision, fact_number, subject_kind, subject_key, fact_kind, fact) SELECT $1::pg_catalog.uuid, $5, fact_number, 'pair', pg_catalog.convert_to(fact->>0, 'UTF8'), fact->>1, pg_catalog.jsonb_build_object('reason_code', fact->>2, 'evidence_groups', fact->3) FROM numbered",
            vec![
                context.entity_id.clone().into(),
                context.publication_revision.into(),
                JsonB(new_facts.clone()).into(),
                uuid_json(records).into(),
                revision.into(),
            ],
        )
    } else {
        (
            "WITH numbered AS (SELECT pg_catalog.row_number() OVER (ORDER BY fact->>0, fact->>1, fact->>2, fact->>3) AS fact_number, fact FROM pg_catalog.jsonb_array_elements($3::pg_catalog.jsonb) AS facts(fact)) INSERT INTO mdm_internal.resolution_facts (entity_id, publication_revision, fact_number, subject_kind, subject_key, fact_kind, fact) SELECT $1::pg_catalog.uuid, $2, fact_number, 'pair', pg_catalog.convert_to(fact->>0, 'UTF8'), fact->>1, pg_catalog.jsonb_build_object('reason_code', fact->>2, 'evidence_groups', fact->3) FROM numbered",
            vec![
                context.entity_id.clone().into(),
                revision.into(),
                JsonB(new_facts.clone()).into(),
            ],
        )
    };
    client
        .update(query, None, &args)
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn persist_outputs(
    client: &mut SpiClient<'_>,
    context: &Context,
    fields: &[OutputField],
    identity: &IdentityState,
    golden: &BTreeMap<(Uuid, String), GoldenSelection>,
    reviews: &[Review],
    revision: i64,
    source_scope: Option<&BTreeSet<Uuid>>,
    mdm_scope: Option<&BTreeSet<Uuid>>,
    review_scope: Option<&BTreeSet<Uuid>>,
) -> Result<(), MdmError> {
    let output_name = client.select("SELECT output_name::text FROM mdm_internal.output_names WHERE entity_id = $1::pg_catalog.uuid AND output_kind = 'entity'", Some(1), &[context.entity_id.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?.first().get::<String>(1).map_err(|error| MdmError::Spi(error.to_string()))?.ok_or_else(|| MdmError::OutputInvalid("entity output name is missing".into()))?;
    let members_name = format!("{output_name}_members");
    let review_name = format!("{output_name}_review");
    let open_reviews = reviews
        .iter()
        .filter(|review| review.status == ReviewStatus::Open)
        .count();
    for identity_row in identity
        .registry
        .iter()
        .filter(|row| row.status == identity::IdentityStatus::Active)
    {
        let members = identity
            .memberships
            .iter()
            .filter(|row| row.active && row.mdm_id == identity_row.mdm_id)
            .collect::<Vec<_>>();
        let mut columns = vec!["mdm_id".into()];
        let mut values = vec![format!("'{}'::uuid", identity_row.mdm_id)];
        for field in fields {
            columns.push(quote_identifier(&field.name));
            values.push(sql_literal(
                &golden
                    .get(&(identity_row.mdm_id, field.name.clone()))
                    .and_then(|selection| selection.value.clone())
                    .unwrap_or(Value::Null),
                &field.type_name,
            ));
        }
        columns.extend([
            "member_count".into(),
            "has_review".into(),
            "last_change_revision".into(),
        ]);
        values.extend([
            members.len().to_string(),
            (open_reviews > 0).to_string(),
            revision.to_string(),
        ]);
        let update_columns = columns
            .iter()
            .skip(1)
            .map(|column| format!("{column} = EXCLUDED.{column}"))
            .collect::<Vec<_>>();
        let changed_columns = columns
            .iter()
            .skip(1)
            .map(|column| format!("target.{column} IS DISTINCT FROM EXCLUDED.{column}"))
            .collect::<Vec<_>>();
        client
            .update(
                &format!(
                    "INSERT INTO mdm_out.{} AS target ({}) VALUES ({}) ON CONFLICT (mdm_id) DO UPDATE SET {} WHERE {}",
                    quote_identifier(&output_name),
                    columns.join(","),
                    values.join(","),
                    update_columns.join(","),
                    changed_columns.join(" OR ")
                ),
                None,
                &[],
            )
            .map_err(|error| MdmError::OutputInvalid(error.to_string()))?;
    }
    let entity_ids = identity
        .registry
        .iter()
        .filter(|row| row.status == identity::IdentityStatus::Active)
        .map(|row| row.mdm_id)
        .collect::<BTreeSet<_>>();
    delete_stale_output_rows(client, &output_name, "mdm_id", &entity_ids, mdm_scope)?;
    let members_sql = format!(
        "INSERT INTO mdm_out.{} AS target (source_record_id, source_name, source_id, mdm_id, active, first_membership_revision, last_membership_revision, membership_reason, last_change_revision) SELECT $1, s.source_name::name, pg_catalog.jsonb_build_object('source_record_key', pg_catalog.encode($2, 'hex')), $3, $4, $5, $6, $7, $8 FROM mdm_internal.source_records r JOIN mdm_internal.source_identities s ON s.source_identity_id = r.source_identity_id WHERE r.source_record_id = $1 ON CONFLICT (source_record_id) DO UPDATE SET source_name = EXCLUDED.source_name, source_id = EXCLUDED.source_id, mdm_id = EXCLUDED.mdm_id, active = EXCLUDED.active, first_membership_revision = EXCLUDED.first_membership_revision, last_membership_revision = EXCLUDED.last_membership_revision, membership_reason = EXCLUDED.membership_reason, last_change_revision = EXCLUDED.last_change_revision WHERE target.source_name IS DISTINCT FROM EXCLUDED.source_name OR target.source_id IS DISTINCT FROM EXCLUDED.source_id OR target.mdm_id IS DISTINCT FROM EXCLUDED.mdm_id OR target.active IS DISTINCT FROM EXCLUDED.active OR target.first_membership_revision IS DISTINCT FROM EXCLUDED.first_membership_revision OR target.last_membership_revision IS DISTINCT FROM EXCLUDED.last_membership_revision OR target.membership_reason IS DISTINCT FROM EXCLUDED.membership_reason OR target.last_change_revision IS DISTINCT FROM EXCLUDED.last_change_revision",
        quote_identifier(&members_name)
    );
    for membership in &identity.memberships {
        client
            .update(
                &members_sql,
                None,
                &[
                    membership.source_record_id.into(),
                    membership.source_sort_key.clone().into(),
                    membership.mdm_id.into(),
                    membership.active.into(),
                    membership.first_membership_revision.into(),
                    membership.last_membership_revision.into(),
                    membership.membership_reason.clone().into(),
                    membership.last_change_revision.into(),
                ],
            )
            .map_err(|error| MdmError::OutputInvalid(error.to_string()))?;
    }
    let member_ids = identity
        .memberships
        .iter()
        .map(|row| row.source_record_id)
        .collect::<BTreeSet<_>>();
    delete_stale_output_rows(
        client,
        &members_name,
        "source_record_id",
        &member_ids,
        source_scope,
    )?;
    let review_sql = format!(
        "INSERT INTO mdm_out.{} AS target (review_id, issue_key, occurrence, status, severity, reason_code, subjects, masked_summary, opened_revision, resolved_revision, last_change_revision, concurrency_version) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) ON CONFLICT (review_id) DO UPDATE SET issue_key = EXCLUDED.issue_key, occurrence = EXCLUDED.occurrence, status = EXCLUDED.status, severity = EXCLUDED.severity, reason_code = EXCLUDED.reason_code, subjects = EXCLUDED.subjects, opened_revision = EXCLUDED.opened_revision, resolved_revision = EXCLUDED.resolved_revision, last_change_revision = EXCLUDED.last_change_revision, concurrency_version = EXCLUDED.concurrency_version WHERE target.issue_key IS DISTINCT FROM EXCLUDED.issue_key OR target.occurrence IS DISTINCT FROM EXCLUDED.occurrence OR target.status IS DISTINCT FROM EXCLUDED.status OR target.severity IS DISTINCT FROM EXCLUDED.severity OR target.reason_code IS DISTINCT FROM EXCLUDED.reason_code OR target.subjects IS DISTINCT FROM EXCLUDED.subjects OR target.masked_summary IS DISTINCT FROM EXCLUDED.masked_summary OR target.opened_revision IS DISTINCT FROM EXCLUDED.opened_revision OR target.resolved_revision IS DISTINCT FROM EXCLUDED.resolved_revision OR target.last_change_revision IS DISTINCT FROM EXCLUDED.last_change_revision OR target.concurrency_version IS DISTINCT FROM EXCLUDED.concurrency_version",
        quote_identifier(&review_name)
    );
    for review in reviews {
        client
            .update(
                &review_sql,
                None,
                &[
                    review.review_id.into(),
                    review.issue_key.to_vec().into(),
                    review.occurrence.into(),
                    (match review.status {
                        ReviewStatus::Open => "open",
                        ReviewStatus::Resolved => "resolved",
                    })
                    .into(),
                    review.severity.clone().into(),
                    review.reason_code.clone().into(),
                    JsonB(review.subjects.clone()).into(),
                    JsonB(review.masked_summary.clone()).into(),
                    review.opened_revision.into(),
                    review.resolved_revision.into(),
                    review.last_change_revision.into(),
                    review.concurrency_version.into(),
                ],
            )
            .map_err(|error| MdmError::OutputInvalid(error.to_string()))?;
    }
    let review_ids = reviews
        .iter()
        .map(|row| row.review_id)
        .collect::<BTreeSet<_>>();
    delete_stale_output_rows(client, &review_name, "review_id", &review_ids, review_scope)?;
    Ok(())
}

fn persist_refresh_inner(
    request: &RefreshRequest,
    session: &catalog::Role,
    selected: &catalog::Role,
) -> Result<RefreshResult, MdmError> {
    let operation_started = Instant::now();
    let delta_admitted = crate::integration::output_delta_enabled()?;
    Spi::connect_mut(|client| {
        let context = load_context(client, &request.entity_name, selected, true)?;
        client
            .update(
                "SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended($1, 0))",
                None,
                &[context.entity_id.clone().into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let operation_id = start_operation(
            client,
            &context,
            if request.rebuild {
                "rebuild"
            } else {
                "refresh"
            },
            selected,
            session,
        )?;
        let mut delta = if delta_admitted {
            ensure_delta_consumers(client, &context)?
        } else {
            Vec::new()
        };
        sync_source_records(client, &context, &request.source_snapshots)?;
        let graph_refresh_started = Instant::now();
        let graph = refresh_graph(client, &context, &request.full_policy)?;
        let graph_refresh_ms = graph_refresh_started.elapsed().as_millis() as u64;
        if delta_admitted {
            read_delta_batches(client, &mut delta)?;
        }
        let control_changes = if delta_admitted {
            load_control_changes(client, &context)?
        } else {
            ControlChanges {
                reconstructable: false,
                changed: false,
                ..ControlChanges::default()
            }
        };
        let graph_transition = graph_generation_transition(client, &context)?;
        let resolution_started = Instant::now();
        let limits = load_limits(&context);
        limits.validate()?;
        let (active_record_count, candidate_pair_count) = load_global_counts(client, &context)?;
        if active_record_count > limits.max_active_records {
            return Err(MdmError::ResolverLimit {
                resource: "max_active_records",
                observed: active_record_count,
                limit: limits.max_active_records,
            });
        }
        let delta_seeds = delta
            .iter()
            .flat_map(|consumer| consumer.affected_records.iter().copied())
            .collect::<BTreeSet<_>>();
        let resnapshot_required = delta
            .iter()
            .any(|consumer| consumer.resnapshot_token.is_some() || consumer.state != "ACTIVE");
        let full_invalidation = delta.iter().any(|consumer| consumer.saw_full_invalidation);
        let exact_delta_range = delta_admitted
            && delta.len() == 2
            && context.publication_revision > 0
            && !request.rebuild
            && !graph_transition
            && !resnapshot_required
            && !full_invalidation
            && delta.iter().all(|consumer| {
                consumer.through_token.is_some() || consumer.log_head == consumer.acknowledged_token
            });
        let empty_delta_range =
            exact_delta_range && delta.iter().all(|consumer| consumer.row_count == 0);
        if empty_delta_range && !control_changes.changed {
            let (identities, open_reviews) = load_publication_counts(client, &context)?;
            let mdm_resolution_ms = resolution_started.elapsed().as_millis() as u64;
            let publication_started = Instant::now();
            client.update("INSERT INTO mdm_internal.publication_observations (entity_id, publication_revision, decision_epoch, artifact_id, operation_id, graph_refresh_id, source_boundary, source_boundary_digest, node_results) VALUES ($1::pg_catalog.uuid, $2, $3, $4::pg_catalog.uuid, $5::pg_catalog.uuid, $6, $7, $8, $9)", None, &[context.entity_id.clone().into(), context.publication_revision.into(), context.decision_epoch.into(), context.artifact_id.clone().into(), operation_id.clone().into(), graph.id.into(), JsonB(graph.boundary.clone()).into(), graph.boundary_digest.clone().into(), JsonB(graph.node_results.clone()).into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            let reviews = load_reviews(client, &context)?;
            project_policy_cases(
                client,
                &context.entity_id,
                &context.entity.name,
                &context.artifact_digest,
                context.definition_version,
                context.publication_revision,
                context.decision_epoch,
                &graph.boundary_digest,
                &reviews,
            )?;
            if delta_admitted {
                finish_delta(client, &mut delta)?;
            }
            let (effective_node_modes, unexpected_full_fallbacks) =
                node_mode_summary(&graph.node_results);
            let result = RefreshResult {
                operation_id: operation_id.clone(),
                entity_name: context.entity.name.clone(),
                changed: false,
                publication_revision: context.publication_revision,
                graph_refresh_id: graph.id,
                source_boundary: graph.boundary,
                source_boundary_digest: hex(&graph.boundary_digest),
                node_results: graph.node_results,
                active_records: active_record_count,
                identities,
                open_reviews,
                stage_timings_ms: json!({
                    "graph_refresh": graph_refresh_ms,
                    "mdm_resolution": mdm_resolution_ms,
                    "publication": publication_started.elapsed().as_millis() as u64,
                    "elapsed_before_operation_completion": operation_started.elapsed().as_millis() as u64
                }),
                component_checks: 0,
                resolver_strategy: "skipped".into(),
                resolver_fallback_reason: None,
                delta_batch_count: delta.iter().map(|consumer| consumer.batch_count).sum(),
                delta_row_count: delta.iter().map(|consumer| consumer.row_count).sum(),
                delta_acknowledged_token: delta
                    .iter()
                    .map(|consumer| consumer.acknowledged_token)
                    .max()
                    .map(|token| token.to_string()),
                delta_lag: delta.iter().map(|consumer| consumer.lag).max(),
                effective_node_modes,
                unexpected_full_fallbacks,
                affected_records: 0,
                affected_components: 0,
                shadow_comparison: None,
            };
            complete_operation(client, &operation_id, &result)?;
            return Ok(result);
        }
        let revision = context
            .publication_revision
            .checked_add(1)
            .ok_or_else(|| MdmError::OperationState("publication revision exhausted".into()))?;
        let mut resolver_strategy = "full".to_owned();
        let mut resolver_fallback_reason = Some(
            if !delta_admitted {
                "delta_capability_unavailable"
            } else if request.rebuild {
                "rebuild_requested"
            } else if context.publication_revision == 0 {
                "initial_population"
            } else if graph_transition {
                "graph_generation_transition"
            } else if resnapshot_required {
                "delta_resnapshot_required"
            } else if full_invalidation {
                "delta_full_invalidation"
            } else if !exact_delta_range {
                "delta_range_unavailable"
            } else if !control_changes.reconstructable {
                "control_history_unreconstructable"
            } else if delta_seeds.is_empty() && !control_changes.changed {
                "no_affected_records"
            } else if candidate_pair_count.is_none()
                || candidate_pair_count.is_some_and(|count| {
                    count > limits.max_automatic_edges || count > limits.max_component_checks
                })
            {
                "global_limit_uncertain"
            } else {
                "affected_evaluation_failed"
            }
            .to_owned(),
        );
        let shadow_comparison = None;
        let affected_eligible = exact_delta_range
            && control_changes.reconstructable
            && (!delta_seeds.is_empty() || control_changes.changed)
            && candidate_pair_count.is_some_and(|count| {
                count <= limits.max_automatic_edges && count <= limits.max_component_checks
            });
        let mut prepared = None;
        if affected_eligible {
            let mut seeds = delta_seeds;
            seeds.extend(control_changes.seeds.iter().copied());
            match evaluate_affected(client, &context, limits.clone(), revision, seeds) {
                Ok(value) => {
                    resolver_strategy = "affected".into();
                    resolver_fallback_reason = None;
                    prepared = Some(value);
                }
                Err(error) => {
                    resolver_fallback_reason = Some(
                        if matches!(error, MdmError::AffectedClosure(_)) {
                            "affected_closure_failed"
                        } else if matches!(error, MdmError::ResolverLimit { .. }) {
                            "global_limit_uncertain"
                        } else {
                            "affected_evaluation_failed"
                        }
                        .into(),
                    );
                }
            }
        }
        let prepared = match prepared {
            Some(value) => value,
            None => evaluate_full(client, &context, limits, revision)?,
        };
        let mdm_resolution_ms = resolution_started.elapsed().as_millis() as u64;
        let resolution = &prepared.evaluation.resolution;
        let next_identity = &prepared.evaluation.identity;
        let next_golden = &prepared.evaluation.golden;
        let next_reviews = &prepared.evaluation.reviews;
        let changed = prepared.evaluation.changed;
        let active_records = active_record_count;
        let affected_records = prepared
            .affected_records
            .as_ref()
            .map_or(active_records, BTreeSet::len);
        let affected_components = prepared
            .evaluation
            .resolution
            .memberships
            .iter()
            .map(|membership| membership.component_key.clone())
            .collect::<BTreeSet<_>>()
            .len();
        let publication_started = Instant::now();
        let fields = ensure_output(client, &context)?;
        let publication_revision = if changed {
            revision
        } else {
            context.publication_revision
        };
        if changed {
            let result_digest = digest(
                "pg_mdm/publication/v1",
                &[&json_bytes(&semantic_projection_with_values(
                    &prepared.merged_identity,
                    prepared.merged_golden.clone(),
                    &prepared.merged_reviews,
                    prepared.merged_resolution_facts.clone(),
                ))],
            );
            client.update("INSERT INTO mdm_internal.publications (entity_id, publication_revision, definition_version, decision_epoch, operation_id, result_digest) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5::pg_catalog.uuid, $6)", None, &[context.entity_id.clone().into(), revision.into(), context.definition_version.into(), context.decision_epoch.into(), operation_id.clone().into(), result_digest.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            for row in &next_identity.registry {
                client.update("INSERT INTO mdm_internal.identity_registry AS target (entity_id, mdm_id, created_revision, retired_revision, status) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5) ON CONFLICT (entity_id, mdm_id) DO UPDATE SET retired_revision = EXCLUDED.retired_revision, status = EXCLUDED.status WHERE target.retired_revision IS DISTINCT FROM EXCLUDED.retired_revision OR target.status IS DISTINCT FROM EXCLUDED.status", None, &[context.entity_id.clone().into(), row.mdm_id.into(), row.created_revision.into(), row.retired_revision.into(), format!("{:?}", row.status).to_lowercase().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_identity.memberships {
                client.update("INSERT INTO mdm_internal.memberships AS target (entity_id, source_record_id, source_name, source_id, mdm_id, active, first_membership_revision, last_membership_revision, membership_reason, last_change_revision) SELECT $1::pg_catalog.uuid, $2, s.source_name::name, pg_catalog.jsonb_build_object('source_record_key', pg_catalog.encode(r.source_record_key, 'hex')), $3, $4, $5, $6, $7, $8 FROM mdm_internal.source_records r JOIN mdm_internal.source_identities s ON s.source_identity_id = r.source_identity_id WHERE r.source_record_id = $2 ON CONFLICT (entity_id, source_record_id) DO UPDATE SET source_name = EXCLUDED.source_name, source_id = EXCLUDED.source_id, mdm_id = EXCLUDED.mdm_id, active = EXCLUDED.active, first_membership_revision = EXCLUDED.first_membership_revision, last_membership_revision = EXCLUDED.last_membership_revision, membership_reason = EXCLUDED.membership_reason, last_change_revision = EXCLUDED.last_change_revision WHERE target.source_name IS DISTINCT FROM EXCLUDED.source_name OR target.source_id IS DISTINCT FROM EXCLUDED.source_id OR target.mdm_id IS DISTINCT FROM EXCLUDED.mdm_id OR target.active IS DISTINCT FROM EXCLUDED.active OR target.first_membership_revision IS DISTINCT FROM EXCLUDED.first_membership_revision OR target.last_membership_revision IS DISTINCT FROM EXCLUDED.last_membership_revision OR target.membership_reason IS DISTINCT FROM EXCLUDED.membership_reason OR target.last_change_revision IS DISTINCT FROM EXCLUDED.last_change_revision", None, &[context.entity_id.clone().into(), row.source_record_id.into(), row.mdm_id.into(), row.active.into(), row.first_membership_revision.into(), row.last_membership_revision.into(), row.membership_reason.clone().into(), row.last_change_revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_identity.aliases {
                client.update("INSERT INTO mdm_internal.identity_aliases (entity_id, alias_mdm_id, canonical_mdm_id, publication_revision) VALUES ($1::pg_catalog.uuid, $2, $3, $4) ON CONFLICT DO NOTHING", None, &[context.entity_id.clone().into(), row.alias_mdm_id.into(), row.canonical_mdm_id.into(), row.publication_revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_identity.splits {
                client.update("INSERT INTO mdm_internal.identity_splits (entity_id, parent_mdm_id, child_mdm_id, publication_revision) VALUES ($1::pg_catalog.uuid, $2, $3, $4) ON CONFLICT DO NOTHING", None, &[context.entity_id.clone().into(), row.parent_mdm_id.into(), row.child_mdm_id.into(), row.publication_revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            let snapshot_scope = prepared
                .affected_records
                .as_ref()
                .zip(prepared.affected_mdm_ids.as_ref());
            persist_publication_snapshots(
                client,
                &context,
                revision,
                next_golden,
                &evaluation::semantic_resolution_facts(resolution),
                snapshot_scope,
            )?;
            for row in next_reviews {
                client.update("INSERT INTO mdm_internal.reviews AS target (review_id, entity_id, issue_key, occurrence, status, severity, reason_code, subjects, masked_summary, opened_revision, resolved_revision, last_change_revision, concurrency_version) VALUES ($1, $2::pg_catalog.uuid, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) ON CONFLICT (review_id) DO UPDATE SET status = EXCLUDED.status, severity = EXCLUDED.severity, reason_code = EXCLUDED.reason_code, subjects = EXCLUDED.subjects, masked_summary = EXCLUDED.masked_summary, resolved_revision = EXCLUDED.resolved_revision, last_change_revision = EXCLUDED.last_change_revision, concurrency_version = EXCLUDED.concurrency_version WHERE target.status IS DISTINCT FROM EXCLUDED.status OR target.severity IS DISTINCT FROM EXCLUDED.severity OR target.reason_code IS DISTINCT FROM EXCLUDED.reason_code OR target.subjects IS DISTINCT FROM EXCLUDED.subjects OR target.masked_summary IS DISTINCT FROM EXCLUDED.masked_summary OR target.resolved_revision IS DISTINCT FROM EXCLUDED.resolved_revision OR target.last_change_revision IS DISTINCT FROM EXCLUDED.last_change_revision OR target.concurrency_version IS DISTINCT FROM EXCLUDED.concurrency_version", None, &[row.review_id.into(), context.entity_id.clone().into(), row.issue_key.to_vec().into(), row.occurrence.into(), (match row.status { ReviewStatus::Open => "open", ReviewStatus::Resolved => "resolved" }).into(), row.severity.clone().into(), row.reason_code.clone().into(), JsonB(row.subjects.clone()).into(), JsonB(row.masked_summary.clone()).into(), row.opened_revision.into(), row.resolved_revision.into(), row.last_change_revision.into(), row.concurrency_version.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            persist_outputs(
                client,
                &context,
                &fields,
                next_identity,
                next_golden,
                next_reviews,
                revision,
                prepared.affected_records.as_ref(),
                prepared.affected_mdm_ids.as_ref(),
                prepared.review_scope.as_ref(),
            )?;
            client.update("UPDATE mdm_internal.entities SET active_version = desired_version, publication_revision = $2 WHERE entity_id = $1::pg_catalog.uuid", None, &[context.entity_id.clone().into(), revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
        }
        client.update("INSERT INTO mdm_internal.publication_observations (entity_id, publication_revision, decision_epoch, artifact_id, operation_id, graph_refresh_id, source_boundary, source_boundary_digest, node_results) VALUES ($1::pg_catalog.uuid, $2, $3, $4::pg_catalog.uuid, $5::pg_catalog.uuid, $6, $7, $8, $9)", None, &[context.entity_id.clone().into(), publication_revision.into(), context.decision_epoch.into(), context.artifact_id.clone().into(), operation_id.clone().into(), graph.id.into(), JsonB(graph.boundary.clone()).into(), graph.boundary_digest.clone().into(), JsonB(graph.node_results.clone()).into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
        let policy_reviews = if changed {
            next_reviews.clone()
        } else {
            load_reviews(client, &context)?
        };
        project_policy_cases(
            client,
            &context.entity_id,
            &context.entity.name,
            &context.artifact_digest,
            context.definition_version,
            publication_revision,
            context.decision_epoch,
            &graph.boundary_digest,
            &policy_reviews,
        )?;
        if delta_admitted {
            finish_delta(client, &mut delta)?;
        }
        let publication_ms = publication_started.elapsed().as_millis() as u64;
        let delta_batch_count = delta.iter().map(|consumer| consumer.batch_count).sum();
        let delta_row_count = delta.iter().map(|consumer| consumer.row_count).sum();
        let delta_acknowledged_token = delta
            .iter()
            .map(|consumer| consumer.acknowledged_token)
            .max()
            .map(|token| token.to_string());
        let delta_lag = delta.iter().map(|consumer| consumer.lag).max();
        let (effective_node_modes, unexpected_full_fallbacks) =
            node_mode_summary(&graph.node_results);
        let result = RefreshResult {
            operation_id: operation_id.clone(),
            entity_name: context.entity.name.clone(),
            changed,
            publication_revision,
            graph_refresh_id: graph.id,
            source_boundary: graph.boundary,
            source_boundary_digest: hex(&graph.boundary_digest),
            node_results: graph.node_results,
            active_records: active_record_count,
            identities: prepared
                .merged_identity
                .registry
                .iter()
                .filter(|row| row.status == identity::IdentityStatus::Active)
                .count(),
            open_reviews: prepared
                .merged_reviews
                .iter()
                .filter(|row| row.status == ReviewStatus::Open)
                .count(),
            stage_timings_ms: json!({
                "graph_refresh": graph_refresh_ms,
                "mdm_resolution": mdm_resolution_ms,
                "publication": publication_ms,
                "elapsed_before_operation_completion": operation_started.elapsed().as_millis() as u64
            }),
            component_checks: resolution.accepted.len() + resolution.rejected.len(),
            resolver_strategy,
            resolver_fallback_reason,
            delta_batch_count,
            delta_row_count,
            delta_acknowledged_token,
            delta_lag,
            effective_node_modes,
            unexpected_full_fallbacks,
            affected_records,
            affected_components,
            shadow_comparison,
        };
        complete_operation(client, &operation_id, &result)?;
        Ok(result)
    })
}

#[pg_extern(name = "refresh", requires = [persist_refresh], sql = "CREATE FUNCTION mdm.refresh(entity_name text, full_policy text DEFAULT 'ALLOW') RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'refresh_wrapper';")]
pub(crate) fn refresh(entity_name: String, full_policy: default!(String, "'ALLOW'")) -> JsonB {
    let source_snapshots =
        grant_refresh_access(&entity_name).unwrap_or_else(|error| crate::raise(error));
    catalog::call_helper(
        "persist_refresh",
        RefreshRequest {
            entity_name,
            full_policy,
            rebuild: false,
            source_snapshots,
        },
    )
    .unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(name = "rebuild", requires = [persist_refresh], sql = "CREATE FUNCTION mdm_admin.rebuild(entity_name text, full_policy text DEFAULT 'ALLOW') RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'rebuild_wrapper';")]
pub(crate) fn rebuild(entity_name: String, full_policy: default!(String, "'ALLOW'")) -> JsonB {
    let source_snapshots =
        grant_refresh_access(&entity_name).unwrap_or_else(|error| crate::raise(error));
    catalog::call_helper(
        "persist_refresh",
        RefreshRequest {
            entity_name,
            full_policy,
            rebuild: true,
            source_snapshots,
        },
    )
    .unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "persist_refresh",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_refresh(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_refresh_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn persist_refresh(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only the public refresh and rebuild wrappers construct this request.
        let request = unsafe { request.get::<RefreshRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("refresh request is required".into()))?;
        let helper_owner = catalog::validate_helper_owner()?;
        let (session, selected) = catalog::validate_caller(&helper_owner)?;
        persist_refresh_inner(request, &session, &selected)
    })();
    result
        .map(|value| {
            JsonB(
                serde_json::to_value(value)
                    .unwrap_or_else(|_| json!({"error": "serialization failure"})),
            )
        })
        .unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(name = "preview", requires = [preview_entity], sql = "CREATE FUNCTION mdm.preview(entity_name text, mode text DEFAULT 'validation', options jsonb DEFAULT '{}'::jsonb) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'preview_wrapper';")]
pub(crate) fn preview(
    entity_name: String,
    mode: default!(String, "'validation'"),
    options: default!(Option<JsonB>, "'{}'::jsonb"),
) -> JsonB {
    catalog::call_helper(
        "preview_entity",
        PreviewRequest {
            entity_name,
            mode,
            options: options.map(|value| value.0).unwrap_or_else(|| json!({})),
        },
    )
    .unwrap_or_else(|error| crate::raise(error))
}

enum PreviewOptions {
    Validation,
    Sampled(usize),
    Scoped {
        source_record_ids: Vec<Uuid>,
        mdm_ids: Vec<Uuid>,
    },
}

fn parse_preview_uuid(text: &str) -> Result<Uuid, MdmError> {
    let raw = text.as_bytes();
    if raw.len() != 36
        || [8, 13, 18, 23].iter().any(|index| raw[*index] != b'-')
        || raw.iter().enumerate().any(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                false
            } else {
                !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase()
            }
        })
    {
        return Err(MdmError::DefinitionInvalid(
            "preview subject must be a canonical UUID".into(),
        ));
    }
    let hex = text.replace('-', "");
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).map_err(|_| {
            MdmError::DefinitionInvalid("preview subject must be a canonical UUID".into())
        })?;
    }
    Ok(Uuid::from_bytes(bytes))
}

fn parse_preview_ids(options: &Value, key: &str) -> Result<Vec<Uuid>, MdmError> {
    let Some(value) = options.get(key) else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        MdmError::DefinitionInvalid(format!("preview option {key} must be an array of UUIDs"))
    })?;
    if values.len() > 1000 {
        return Err(MdmError::DefinitionInvalid(format!(
            "preview option {key} exceeds 1000 subjects"
        )));
    }
    let mut seen = BTreeSet::new();
    values
        .iter()
        .map(|value| {
            let text = value.as_str().ok_or_else(|| {
                MdmError::DefinitionInvalid(format!(
                    "preview option {key} must contain UUID strings"
                ))
            })?;
            let id = parse_preview_uuid(text).map_err(|_| {
                MdmError::DefinitionInvalid(format!("invalid UUID in preview option {key}"))
            })?;
            if !seen.insert(id) {
                return Err(MdmError::DefinitionInvalid(format!(
                    "preview option {key} contains a duplicate subject"
                )));
            }
            Ok(id)
        })
        .collect()
}

fn parse_preview_options(mode: &str, options: &Value) -> Result<PreviewOptions, MdmError> {
    let object = options.as_object().ok_or_else(|| {
        MdmError::DefinitionInvalid("preview options must be a JSON object".into())
    })?;
    match mode {
        "validation" => {
            if !object.is_empty() {
                return Err(MdmError::DefinitionInvalid(
                    "validation preview does not accept options".into(),
                ));
            }
            Ok(PreviewOptions::Validation)
        }
        "sampled" => {
            if object.keys().any(|key| key != "sample_size") {
                return Err(MdmError::DefinitionInvalid(
                    "sampled preview accepts only sample_size".into(),
                ));
            }
            let sample_size = object
                .get("sample_size")
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|value| usize::try_from(value).ok())
                        .filter(|value| (1..=1000).contains(value))
                        .ok_or_else(|| {
                            MdmError::DefinitionInvalid(
                                "sample_size must be between 1 and 1000".into(),
                            )
                        })
                })
                .transpose()?
                .unwrap_or(25);
            Ok(PreviewOptions::Sampled(sample_size))
        }
        "scoped" => {
            if object
                .keys()
                .any(|key| key != "source_record_ids" && key != "mdm_ids")
            {
                return Err(MdmError::DefinitionInvalid(
                    "scoped preview accepts only source_record_ids and mdm_ids".into(),
                ));
            }
            let source_record_ids = parse_preview_ids(options, "source_record_ids")?;
            let mdm_ids = parse_preview_ids(options, "mdm_ids")?;
            if source_record_ids.is_empty() && mdm_ids.is_empty() {
                return Err(MdmError::DefinitionInvalid(
                    "scoped preview requires source_record_ids or mdm_ids".into(),
                ));
            }
            Ok(PreviewOptions::Scoped {
                source_record_ids,
                mdm_ids,
            })
        }
        _ => Err(MdmError::DefinitionInvalid(
            "preview mode must be validation, sampled, or scoped".into(),
        )),
    }
}

fn validate_preview_contract(
    client: &SpiClient<'_>,
    context: &Context,
    selected: &catalog::Role,
) -> Result<(), MdmError> {
    let contract = client
        .select(
            "SELECT contract_version, graph_digest, contract FROM pgtrickle.graph_contract(ARRAY[$1::regclass])",
            Some(1),
            &[context.graph_root.clone().into()],
        )
        .map_err(|error| MdmError::GraphContract(error.to_string()))?;
    if contract.is_empty() {
        return Err(MdmError::GraphContract("graph contract is missing".into()));
    }
    let contract = contract.first();
    let version = contract
        .get::<i16>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let digest = contract
        .get::<Vec<u8>>(2)
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let current_contract = contract
        .get::<JsonB>(3)
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if version != Some(1)
        || digest.as_ref().is_none_or(|value| value.len() != 32)
        || current_contract.as_ref().is_none_or(|value| {
            stable_graph_contract(&value.0) != stable_graph_contract(&context.graph_contract)
        })
    {
        return Err(MdmError::GraphContract(
            "graph contract changed since installation".into(),
        ));
    }

    let sources = client
        .select(
            "SELECT s.source_name::text, s.relation_name, b.relation_oid, b.binding_fingerprint, c.oid, pg_catalog.jsonb_build_object('relation_oid', c.oid::bigint, 'relation_name', pg_catalog.format('%I.%I', n.nspname, c.relname), 'relkind', c.relkind::text, 'relpersistence', c.relpersistence::text, 'row_security', c.relrowsecurity, 'force_row_security', c.relforcerowsecurity, 'policies', COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('name', p.polname::text, 'permissive', p.polpermissive, 'roles', p.polroles::text, 'using', pg_catalog.pg_get_expr(p.polqual, p.polrelid), 'check', pg_catalog.pg_get_expr(p.polwithcheck, p.polrelid)) ORDER BY p.polname) FROM pg_catalog.pg_policy p WHERE p.polrelid = c.oid), '[]'::pg_catalog.jsonb)), pg_catalog.has_schema_privilege($2, c.relnamespace, 'USAGE') AND pg_catalog.has_table_privilege($2, c.oid, 'SELECT') AND pg_catalog.has_table_privilege($2, c.oid, 'MAINTAIN') FROM mdm_internal.source_identities s JOIN mdm_internal.source_bindings b USING (source_identity_id) LEFT JOIN pg_catalog.pg_class c ON c.oid = pg_catalog.to_regclass(s.relation_name)::pg_catalog.oid LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE s.entity_id = $1::pg_catalog.uuid ORDER BY s.source_name",
            None,
            &[context.entity_id.clone().into(), selected.oid.into()],
        )
        .map_err(|error| MdmError::SourceInvalid(error.to_string()))?;
    if sources.len() != context.entity.sources.len() {
        return Err(MdmError::SourceInvalid(
            "source identity or binding is missing".into(),
        ));
    }
    for row in sources {
        let source_name = row
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::SourceInvalid("source name is missing".into()))?;
        let source = context
            .entity
            .sources
            .iter()
            .find(|source| source.name == source_name)
            .ok_or_else(|| MdmError::SourceInvalid("source definition is missing".into()))?;
        let relation_name = row
            .get::<String>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::SourceInvalid("source relation is missing".into()))?;
        let current_oid = row
            .get::<pg_sys::Oid>(5)
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let bound_oid = row
            .get::<pg_sys::Oid>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let stored_fingerprint = row
            .get::<JsonB>(4)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .map(|value| value.0);
        let current_fingerprint = row
            .get::<JsonB>(6)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .map(|value| value.0);
        let permitted = row
            .get::<bool>(7)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or(false);
        if relation_name != source.relation
            || current_oid != bound_oid
            || stored_fingerprint != current_fingerprint
            || !permitted
        {
            return Err(MdmError::SourceInvalid(format!(
                "source {} binding or execution-role privileges changed",
                source.name
            )));
        }
    }
    Ok(())
}

fn scope_identity(old: &IdentityState, selected: &BTreeSet<Uuid>) -> IdentityState {
    let identity_ids = old
        .memberships
        .iter()
        .filter(|membership| selected.contains(&membership.source_record_id))
        .map(|membership| membership.mdm_id)
        .collect::<BTreeSet<_>>();
    IdentityState {
        registry: old
            .registry
            .iter()
            .filter(|row| identity_ids.contains(&row.mdm_id))
            .cloned()
            .collect(),
        memberships: old
            .memberships
            .iter()
            .filter(|row| selected.contains(&row.source_record_id))
            .cloned()
            .collect(),
        aliases: Vec::new(),
        splits: Vec::new(),
    }
}

fn json_mentions_any(value: &Value, ids: &BTreeSet<String>) -> bool {
    match value {
        Value::String(value) => ids.contains(value),
        Value::Array(values) => values.iter().any(|value| json_mentions_any(value, ids)),
        Value::Object(values) => values.values().any(|value| json_mentions_any(value, ids)),
        _ => false,
    }
}

fn scope_reviews(
    reviews: &[Review],
    record_ids: &BTreeSet<Uuid>,
    mdm_ids: &BTreeSet<Uuid>,
) -> Vec<Review> {
    let ids = record_ids
        .iter()
        .chain(mdm_ids)
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    reviews
        .iter()
        .filter(|review| json_mentions_any(&review.subjects, &ids))
        .cloned()
        .collect()
}

fn scope_current_golden(current: Value, mdm_ids: &BTreeSet<Uuid>) -> Value {
    Value::Array(
        current
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| {
                row.get(0)
                    .and_then(Value::as_str)
                    .and_then(|id| parse_preview_uuid(id).ok())
                    .is_some_and(|id| mdm_ids.contains(&id))
            })
            .cloned()
            .collect(),
    )
}

fn selected_mdm_ids(identity: &IdentityState, selected: &BTreeSet<Uuid>) -> BTreeSet<Uuid> {
    identity
        .memberships
        .iter()
        .filter(|membership| selected.contains(&membership.source_record_id))
        .map(|membership| membership.mdm_id)
        .collect()
}

fn splice_identity(
    old: &IdentityState,
    selected: &BTreeSet<Uuid>,
    scoped: &IdentityState,
) -> IdentityState {
    let selected_mdm_ids = selected_mdm_ids(old, selected);
    let mut registry = old
        .registry
        .iter()
        .filter(|row| !selected_mdm_ids.contains(&row.mdm_id))
        .map(|row| (row.mdm_id, row.clone()))
        .collect::<BTreeMap<_, _>>();
    for row in &scoped.registry {
        registry.insert(row.mdm_id, row.clone());
    }
    let mut aliases = old.aliases.clone();
    for row in &scoped.aliases {
        if !aliases.iter().any(|existing| existing == row) {
            aliases.push(row.clone());
        }
    }
    let mut splits = old.splits.clone();
    for row in &scoped.splits {
        if !splits.iter().any(|existing| existing == row) {
            splits.push(row.clone());
        }
    }
    IdentityState {
        registry: registry.into_values().collect(),
        memberships: old
            .memberships
            .iter()
            .filter(|row| !selected.contains(&row.source_record_id))
            .cloned()
            .chain(scoped.memberships.iter().cloned())
            .collect(),
        aliases,
        splits,
    }
}

fn splice_golden(
    old: &Value,
    selected_mdm_ids: &BTreeSet<Uuid>,
    scoped: &BTreeMap<(Uuid, String), GoldenSelection>,
) -> Value {
    let mut rows = old
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            row.get(0)
                .and_then(Value::as_str)
                .and_then(|id| parse_preview_uuid(id).ok())
                .is_none_or(|id| !selected_mdm_ids.contains(&id))
        })
        .cloned()
        .chain(
            evaluation::semantic_golden(scoped)
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        )
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.get(0)
            .and_then(Value::as_str)
            .cmp(&right.get(0).and_then(Value::as_str))
            .then_with(|| {
                left.get(1)
                    .and_then(Value::as_str)
                    .cmp(&right.get(1).and_then(Value::as_str))
            })
    });
    Value::Array(rows)
}

fn splice_reviews(
    old: &[Review],
    selected: &BTreeSet<Uuid>,
    selected_mdm_ids: &BTreeSet<Uuid>,
    scoped: &[Review],
) -> Vec<Review> {
    let ids = selected
        .iter()
        .chain(selected_mdm_ids)
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    old.iter()
        .filter(|review| !json_mentions_any(&review.subjects, &ids))
        .cloned()
        .chain(scoped.iter().cloned())
        .collect()
}

fn fact_is_scoped(row: &Value, selected: &BTreeSet<Uuid>) -> bool {
    row.get(0)
        .and_then(Value::as_str)
        .and_then(|key| key.split_once(':'))
        .and_then(|(left, right)| {
            Some((
                parse_preview_uuid(left).ok()?,
                parse_preview_uuid(right).ok()?,
            ))
        })
        .is_some_and(|(left, right)| selected.contains(&left) && selected.contains(&right))
}

fn splice_resolution_facts(
    old: &Value,
    selected: &BTreeSet<Uuid>,
    scoped: &crate::resolver::Resolution,
) -> Value {
    let mut rows = old
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| !fact_is_scoped(row, selected))
        .cloned()
        .chain(
            evaluation::semantic_resolution_facts(scoped)
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        )
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| serde_json::to_string(row).unwrap_or_default());
    Value::Array(rows)
}

fn semantic_projection_with_values(
    identity: &IdentityState,
    golden: Value,
    reviews: &[Review],
    resolution_facts: Value,
) -> Value {
    json!({
        "identity": evaluation::semantic_identity(identity),
        "golden": golden,
        "reviews": evaluation::semantic_reviews(reviews),
        "resolution_facts": resolution_facts
    })
}

struct PreparedEvaluation {
    evaluation: evaluation::EvaluationResult,
    merged_identity: IdentityState,
    merged_golden: Value,
    merged_reviews: Vec<Review>,
    merged_resolution_facts: Value,
    affected_records: Option<BTreeSet<Uuid>>,
    affected_mdm_ids: Option<BTreeSet<Uuid>>,
    review_scope: Option<BTreeSet<Uuid>>,
}

struct AffectedScope {
    records: Vec<SourceRow>,
    manual_matches: Vec<DecisionEdge>,
    cannot_links: Vec<DecisionEdge>,
    pair_decisions: Vec<crate::pair::PairDecision>,
    selected: BTreeSet<Uuid>,
}

fn load_global_counts(
    client: &SpiClient<'_>,
    context: &Context,
) -> Result<(usize, Option<usize>), MdmError> {
    let active = client
        .select(
            "SELECT count(*)::bigint FROM mdm_internal.source_records WHERE entity_id = $1::pg_catalog.uuid AND active",
            Some(1),
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .first()
        .get::<i64>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("active record count is NULL".into()))?;
    let active = usize::try_from(active)
        .map_err(|_| MdmError::ResolverInvalid("active record count is negative".into()))?;
    let Some(relation) = &context.pair_stats_relation else {
        return Ok((active, None));
    };
    let candidates = client
        .select(
            &format!("SELECT candidate_pairs FROM {relation}"),
            Some(1),
            &[],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .first()
        .get::<i64>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("candidate pair count is NULL".into()))?;
    let candidates = usize::try_from(candidates)
        .map_err(|_| MdmError::ResolverInvalid("candidate pair count is negative".into()))?;
    Ok((active, Some(candidates)))
}

fn load_publication_counts(
    client: &SpiClient<'_>,
    context: &Context,
) -> Result<(usize, usize), MdmError> {
    let row = client
        .select(
            "SELECT (SELECT count(*)::bigint FROM mdm_internal.identity_registry WHERE entity_id = $1::pg_catalog.uuid AND status = 'active'), (SELECT count(*)::bigint FROM mdm_internal.reviews WHERE entity_id = $1::pg_catalog.uuid AND status = 'open')",
            Some(1),
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .first();
    let identities = row
        .get::<i64>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("active identity count is NULL".into()))?;
    let open_reviews = row
        .get::<i64>(2)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("open review count is NULL".into()))?;
    Ok((
        usize::try_from(identities)
            .map_err(|_| MdmError::Spi("active identity count is negative".into()))?,
        usize::try_from(open_reviews)
            .map_err(|_| MdmError::Spi("open review count is negative".into()))?,
    ))
}

fn load_affected_scope(
    client: &mut SpiClient<'_>,
    context: &Context,
    limits: &ResolverLimits,
    seeds: BTreeSet<Uuid>,
    old_identity: &IdentityState,
) -> Result<AffectedScope, MdmError> {
    if seeds.is_empty() {
        return Err(MdmError::AffectedClosure("affected seeds are empty".into()));
    }
    let mut selected = seeds.clone();
    let mut matches = BTreeMap::<Uuid, DecisionEdge>::new();
    let mut cannot = BTreeMap::<Uuid, DecisionEdge>::new();
    let mut pairs = BTreeMap::<(Uuid, Uuid), crate::pair::PairDecision>::new();
    let mut source_names = load_sources_by_ids(client, context, &selected)?
        .into_iter()
        .map(|source| (source.id, source.source_name))
        .collect::<BTreeMap<_, _>>();
    loop {
        if selected.len() > limits.max_active_records {
            return Err(MdmError::AffectedClosure(
                "affected closure exceeds the active-record limit".into(),
            ));
        }
        let (new_matches, new_cannot) = load_decisions_for_scope(client, context, &selected)?;
        for decision in new_matches {
            matches.insert(decision.decision_id, decision);
        }
        for decision in new_cannot {
            cannot.insert(decision.decision_id, decision);
        }
        let no_active_filter = BTreeSet::new();
        let new_pairs = load_pair_decisions(
            client,
            context,
            &no_active_filter,
            &source_names,
            Some(&selected),
        )?;
        let mut endpoint_ids = selected.clone();
        for decision in new_pairs {
            endpoint_ids.insert(decision.pair.left_source_record_id);
            endpoint_ids.insert(decision.pair.right_source_record_id);
            let key = if decision.pair.left_source_record_id <= decision.pair.right_source_record_id
            {
                (
                    decision.pair.left_source_record_id,
                    decision.pair.right_source_record_id,
                )
            } else {
                (
                    decision.pair.right_source_record_id,
                    decision.pair.left_source_record_id,
                )
            };
            pairs.insert(key, decision);
        }
        let mut names_changed = false;
        for source in load_sources_by_ids(client, context, &endpoint_ids)? {
            names_changed |= source_names.insert(source.id, source.source_name).is_none();
        }
        if pairs.values().any(|decision| {
            !source_names.contains_key(&decision.pair.left_source_record_id)
                || !source_names.contains_key(&decision.pair.right_source_record_id)
        }) {
            return Err(MdmError::AffectedClosure(
                "affected evidence references an inactive source record".into(),
            ));
        }
        let affected = crate::affected::build(
            seeds.iter().copied(),
            &old_identity.memberships,
            &matches.values().cloned().collect::<Vec<_>>(),
            &pairs.values().cloned().collect::<Vec<_>>(),
        )?
        .into_records();
        if affected == selected && !names_changed {
            break;
        }
        selected = affected;
    }
    let records = load_sources_by_ids(client, context, &selected)?;
    let known = records
        .iter()
        .map(|record| record.id)
        .chain(
            old_identity
                .memberships
                .iter()
                .map(|membership| membership.source_record_id),
        )
        .collect::<BTreeSet<_>>();
    if selected.iter().any(|record| !known.contains(record)) {
        return Err(MdmError::AffectedClosure(
            "affected delta references an unknown source record".into(),
        ));
    }
    let manual_matches = matches
        .into_values()
        .filter(|decision| {
            selected.contains(&decision.left_source_record_id)
                && selected.contains(&decision.right_source_record_id)
        })
        .collect();
    let cannot_links = cannot
        .into_values()
        .filter(|decision| {
            selected.contains(&decision.left_source_record_id)
                && selected.contains(&decision.right_source_record_id)
        })
        .collect();
    let pair_decisions = pairs
        .into_values()
        .filter(|decision| {
            selected.contains(&decision.pair.left_source_record_id)
                && selected.contains(&decision.pair.right_source_record_id)
        })
        .collect();
    Ok(AffectedScope {
        records,
        manual_matches,
        cannot_links,
        pair_decisions,
        selected,
    })
}

fn evaluate_full(
    client: &mut SpiClient<'_>,
    context: &Context,
    limits: ResolverLimits,
    revision: i64,
) -> Result<PreparedEvaluation, MdmError> {
    let sources = load_sources(client, context, limits.max_active_records)?;
    let active = sources.iter().map(|row| row.id).collect::<BTreeSet<_>>();
    let records = sources
        .iter()
        .map(|row| EvaluationRecord {
            source_record_id: row.id,
            source_name: row.source_name.clone(),
            source_sort_key: row.sort_key.clone(),
        })
        .collect::<Vec<_>>();
    let (manual_matches, cannot_links) = load_decisions(client, context, &active)?;
    let source_names = sources
        .iter()
        .map(|source| (source.id, source.source_name.clone()))
        .collect::<BTreeMap<_, _>>();
    let pair_decisions = load_pair_decisions(client, context, &active, &source_names, None)?;
    let old_identity = load_old_identity(client, context)?;
    let old_reviews = load_reviews(client, context)?;
    let old_golden = load_current_golden(client, context)?;
    let old_resolution_facts = load_current_resolution_facts(client, context)?;
    let golden_rows = load_golden_rows(client, context)?;
    let overrides = load_overrides(client, context)?;
    let mut ids = allocate_ids(
        client,
        sources
            .len()
            .saturating_add(old_reviews.len())
            .saturating_add(pair_decisions.len())
            .saturating_add(1),
    )?;
    let mut allocator = || ids.pop().unwrap_or_else(|| Uuid::from_bytes([0; 16]));
    let evaluation = evaluation::resolve_and_compare(evaluation::EvaluationInput {
        entity: &context.entity,
        definition_version: context.definition_version,
        publication_revision: revision,
        records: &records,
        manual_matches,
        cannot_links,
        pair_decisions,
        limits,
        old_identity: &old_identity,
        old_reviews: &old_reviews,
        old_golden: &old_golden,
        old_resolution_facts: &old_resolution_facts,
        golden_rows: &golden_rows,
        overrides: &overrides,
        allocator: &mut allocator,
    })?;
    Ok(PreparedEvaluation {
        merged_identity: evaluation.identity.clone(),
        merged_golden: evaluation::semantic_golden(&evaluation.golden),
        merged_reviews: evaluation.reviews.clone(),
        merged_resolution_facts: evaluation::semantic_resolution_facts(&evaluation.resolution),
        evaluation,
        affected_records: None,
        affected_mdm_ids: None,
        review_scope: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn evaluate_affected(
    client: &mut SpiClient<'_>,
    context: &Context,
    limits: ResolverLimits,
    revision: i64,
    seeds: BTreeSet<Uuid>,
) -> Result<PreparedEvaluation, MdmError> {
    let old_identity = load_old_identity(client, context)?;
    let old_reviews = load_reviews(client, context)?;
    let old_golden = load_current_golden(client, context)?;
    let old_resolution_facts = load_current_resolution_facts(client, context)?;
    let scope = load_affected_scope(client, context, &limits, seeds, &old_identity)?;
    let selected_mdm_ids = selected_mdm_ids(&old_identity, &scope.selected);
    let scoped_old_identity = scope_identity(&old_identity, &scope.selected);
    let scoped_old_reviews = scope_reviews(&old_reviews, &scope.selected, &selected_mdm_ids);
    let scoped_old_golden = scope_current_golden(old_golden.clone(), &selected_mdm_ids);
    let scoped_old_resolution_facts =
        scope_resolution_facts(&old_resolution_facts, &scope.selected);
    let records = scope
        .records
        .iter()
        .map(|row| EvaluationRecord {
            source_record_id: row.id,
            source_name: row.source_name.clone(),
            source_sort_key: row.sort_key.clone(),
        })
        .collect::<Vec<_>>();
    let golden_rows = load_golden_rows_scope(client, context, Some(&scope.selected))?;
    let overrides = load_overrides(client, context)?
        .into_iter()
        .map(|(field, values)| {
            (
                field,
                values
                    .into_iter()
                    .filter(|value| scope.selected.contains(&value.anchor_source_record_id))
                    .collect(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut ids = allocate_ids(
        client,
        records
            .len()
            .saturating_add(scoped_old_reviews.len())
            .saturating_add(scope.pair_decisions.len())
            .saturating_add(1),
    )?;
    let mut allocator = || ids.pop().unwrap_or_else(|| Uuid::from_bytes([0; 16]));
    let evaluation = evaluation::resolve_and_compare(evaluation::EvaluationInput {
        entity: &context.entity,
        definition_version: context.definition_version,
        publication_revision: revision,
        records: &records,
        manual_matches: scope.manual_matches,
        cannot_links: scope.cannot_links,
        pair_decisions: scope.pair_decisions,
        limits,
        old_identity: &scoped_old_identity,
        old_reviews: &scoped_old_reviews,
        old_golden: &scoped_old_golden,
        old_resolution_facts: &scoped_old_resolution_facts,
        golden_rows: &golden_rows,
        overrides: &overrides,
        allocator: &mut allocator,
    })?;
    let merged_identity = splice_identity(&old_identity, &scope.selected, &evaluation.identity);
    let merged_golden = splice_golden(&old_golden, &selected_mdm_ids, &evaluation.golden);
    let merged_reviews = splice_reviews(
        &old_reviews,
        &scope.selected,
        &selected_mdm_ids,
        &evaluation.reviews,
    );
    let merged_resolution_facts = splice_resolution_facts(
        &old_resolution_facts,
        &scope.selected,
        &evaluation.resolution,
    );
    let mut review_scope = scoped_old_reviews
        .iter()
        .map(|review| review.review_id)
        .collect::<BTreeSet<_>>();
    review_scope.extend(evaluation.reviews.iter().map(|review| review.review_id));
    let mut affected_mdm_ids = selected_mdm_ids;
    affected_mdm_ids.extend(evaluation.identity.registry.iter().map(|row| row.mdm_id));
    Ok(PreparedEvaluation {
        evaluation,
        merged_identity,
        merged_golden,
        merged_reviews,
        merged_resolution_facts,
        affected_records: Some(scope.selected),
        affected_mdm_ids: Some(affected_mdm_ids),
        review_scope: Some(review_scope),
    })
}

fn validate_scoped_subjects(
    client: &SpiClient<'_>,
    context: &Context,
    source_ids: &[Uuid],
    mdm_ids: &[Uuid],
    active_sources: &[SourceRow],
    old_identity: &IdentityState,
) -> Result<BTreeSet<Uuid>, MdmError> {
    let active = active_sources
        .iter()
        .map(|row| row.id)
        .collect::<BTreeSet<_>>();
    let mut selected = BTreeSet::new();
    for id in source_ids {
        let record = client
            .select(
                "SELECT entity_id::text, active FROM mdm_internal.source_records WHERE source_record_id = $1::pg_catalog.uuid",
                Some(1),
                &[(*id).into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if record.is_empty() {
            return Err(MdmError::SourceInvalid(format!(
                "source record {id} does not exist"
            )));
        }
        let row = record.first();
        let entity_id = row
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("source record entity is NULL".into()))?;
        let is_active = row
            .get::<bool>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or(false);
        if entity_id != context.entity_id {
            return Err(MdmError::SourceInvalid(
                "scoped preview cannot include a record from another entity".into(),
            ));
        }
        if !is_active || !active.contains(id) {
            return Err(MdmError::SourceInvalid(format!(
                "source record {id} is not active in the current publication"
            )));
        }
        selected.insert(*id);
    }
    for id in mdm_ids {
        let identity = client
            .select(
                "SELECT status FROM mdm_internal.identity_registry WHERE entity_id = $1::pg_catalog.uuid AND mdm_id = $2::pg_catalog.uuid",
                Some(1),
                &[context.entity_id.clone().into(), (*id).into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if identity.is_empty() {
            let exists = client
                .select(
                    "SELECT EXISTS (SELECT FROM mdm_internal.identity_registry WHERE mdm_id = $1::pg_catalog.uuid)",
                    Some(1),
                    &[(*id).into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .first()
                .get::<bool>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false);
            return Err(MdmError::SourceInvalid(if exists {
                "scoped preview cannot include an identity from another entity".into()
            } else {
                format!("identity {id} does not exist")
            }));
        }
        let status = identity
            .first()
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("identity status is NULL".into()))?;
        if status != "active" {
            return Err(MdmError::SourceInvalid(format!(
                "identity {id} is not active"
            )));
        }
        for membership in old_identity.memberships.iter().filter(|membership| {
            membership.active
                && membership.mdm_id == *id
                && active.contains(&membership.source_record_id)
        }) {
            selected.insert(membership.source_record_id);
        }
    }
    if selected.is_empty() {
        return Err(MdmError::DefinitionInvalid(
            "scoped preview has no active subjects".into(),
        ));
    }
    Ok(selected)
}

fn expand_decision_scope(
    context: &Context,
    mut selected: BTreeSet<Uuid>,
    active_sources: &[SourceRow],
    old_identity: &IdentityState,
    pair_decisions: &[crate::pair::PairDecision],
    matches: &[DecisionEdge],
    cannot_links: &[DecisionEdge],
) -> Result<BTreeSet<Uuid>, MdmError> {
    let active = active_sources
        .iter()
        .map(|row| row.id)
        .collect::<BTreeSet<_>>();
    let closure_limit = context
        .entity
        .limits
        .get("max_decision_closure")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(crate::semantics::DEFAULT_MAX_DECISION_CLOSURE);
    let mut neighbors = BTreeMap::<Uuid, BTreeSet<Uuid>>::new();
    let mut connect = |left: Uuid, right: Uuid| {
        neighbors.entry(left).or_default().insert(right);
        neighbors.entry(right).or_default().insert(left);
    };
    for decision in pair_decisions {
        connect(
            decision.pair.left_source_record_id,
            decision.pair.right_source_record_id,
        );
    }
    for edge in matches.iter().chain(cannot_links) {
        connect(edge.left_source_record_id, edge.right_source_record_id);
    }
    let mut records_by_identity = BTreeMap::<Uuid, Vec<Uuid>>::new();
    for membership in old_identity
        .memberships
        .iter()
        .filter(|membership| membership.active && active.contains(&membership.source_record_id))
    {
        records_by_identity
            .entry(membership.mdm_id)
            .or_default()
            .push(membership.source_record_id);
    }
    for records in records_by_identity.values() {
        if let Some(first) = records.first() {
            for record in records.iter().skip(1) {
                connect(*first, *record);
            }
        }
    }
    if selected.len() > closure_limit {
        return Err(MdmError::ResolverLimit {
            resource: "max_decision_closure",
            observed: selected.len(),
            limit: closure_limit,
        });
    }
    let mut pending = selected.iter().copied().collect::<Vec<_>>();
    let mut index = 0;
    while index < pending.len() {
        let current = pending[index];
        index += 1;
        if let Some(adjacent) = neighbors.get(&current) {
            for neighbor in adjacent {
                if selected.insert(*neighbor) {
                    if selected.len() > closure_limit {
                        return Err(MdmError::ResolverLimit {
                            resource: "max_decision_closure",
                            observed: selected.len(),
                            limit: closure_limit,
                        });
                    }
                    pending.push(*neighbor);
                }
            }
        }
    }
    Ok(selected)
}

fn preview_run(
    client: &mut SpiClient<'_>,
    context: &Context,
    selected_role: &catalog::Role,
    options: PreviewOptions,
) -> Result<Value, MdmError> {
    validate_preview_contract(client, context, selected_role)?;
    let (scoped, sample_size, scope) = match options {
        PreviewOptions::Validation => {
            return Ok(json!({
                "entity_name": context.entity.name,
                "mode": "validation",
                "evidence_level": "validation",
                "exact": false,
                "definition_version": context.definition_version,
                "artifact_id": context.artifact_id,
                "graph_digest": hex(&context.graph_digest),
                "data_read": false,
                "publication_revision": context.publication_revision
            }));
        }
        PreviewOptions::Sampled(sample_size) => (false, Some(sample_size), None),
        PreviewOptions::Scoped {
            source_record_ids,
            mdm_ids,
        } => (true, None, Some((source_record_ids, mdm_ids))),
    };
    let limits = load_limits(context);
    limits.validate()?;
    let sources = if let Some(sample_size) = sample_size {
        load_sources_ordered(client, context, sample_size)?
    } else {
        load_sources(client, context, limits.max_active_records)?
    };
    let all_active = sources.iter().map(|row| row.id).collect::<BTreeSet<_>>();
    let source_names = sources
        .iter()
        .map(|source| (source.id, source.source_name.clone()))
        .collect::<BTreeMap<_, _>>();
    let (manual_matches, cannot_links) = load_decisions(client, context, &all_active)?;
    let pair_decisions = load_pair_decisions(client, context, &all_active, &source_names, None)?;
    let old_identity_all = load_old_identity(client, context)?;
    let (selected_ids, source_ids, requested_mdm_ids) = if let Some((source_ids, mdm_ids)) = scope {
        let seeds = validate_scoped_subjects(
            client,
            context,
            &source_ids,
            &mdm_ids,
            &sources,
            &old_identity_all,
        )?;
        let closure = expand_decision_scope(
            context,
            seeds,
            &sources,
            &old_identity_all,
            &pair_decisions,
            &manual_matches,
            &cannot_links,
        )?;
        (closure, source_ids, mdm_ids)
    } else {
        (all_active.clone(), Vec::new(), Vec::new())
    };
    let selected_mdm_ids = old_identity_all
        .memberships
        .iter()
        .filter(|membership| {
            membership.active && selected_ids.contains(&membership.source_record_id)
        })
        .map(|membership| membership.mdm_id)
        .collect::<BTreeSet<_>>();
    let old_identity = scope_identity(&old_identity_all, &selected_ids);
    let old_reviews = scope_reviews(
        &load_reviews(client, context)?,
        &selected_ids,
        &selected_mdm_ids,
    );
    let old_golden = scope_current_golden(load_current_golden(client, context)?, &selected_mdm_ids);
    let old_resolution_facts = scope_resolution_facts(
        &load_current_resolution_facts(client, context)?,
        &selected_ids,
    );
    let golden_rows = load_golden_rows(client, context)?
        .into_iter()
        .filter(|row| selected_ids.contains(&row.source_record_id))
        .collect::<Vec<_>>();
    let overrides = load_overrides(client, context)?
        .into_iter()
        .map(|(field, values)| {
            (
                field,
                values
                    .into_iter()
                    .filter(|value| selected_ids.contains(&value.anchor_source_record_id))
                    .collect(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let sources = sources
        .into_iter()
        .filter(|source| selected_ids.contains(&source.id))
        .map(|source| EvaluationRecord {
            source_record_id: source.id,
            source_name: source.source_name,
            source_sort_key: source.sort_key,
        })
        .collect::<Vec<_>>();
    let pair_decisions = pair_decisions
        .into_iter()
        .filter(|decision| {
            selected_ids.contains(&decision.pair.left_source_record_id)
                && selected_ids.contains(&decision.pair.right_source_record_id)
        })
        .collect::<Vec<_>>();
    let pair_examples = pair_decisions
        .iter()
        .take(10)
        .map(|decision| {
            json!({
                "left_source_record_id": decision.pair.left_source_record_id.to_string(),
                "right_source_record_id": decision.pair.right_source_record_id.to_string(),
                "decision": decision.result.as_str(),
                "reason_codes": decision.reason_codes
            })
        })
        .collect::<Vec<_>>();
    let pair_count = pair_decisions.len();
    let old_revision = context
        .publication_revision
        .checked_add(1)
        .ok_or_else(|| MdmError::OperationState("publication revision exhausted".into()))?;
    let seed = json_bytes(&json!({
        "entity_id": context.entity_id,
        "source_record_ids": selected_ids.iter().map(ToString::to_string).collect::<Vec<_>>()
    }));
    let mut counter = 0u64;
    let mut reserved = old_identity_all
        .registry
        .iter()
        .map(|row| row.mdm_id)
        .collect::<BTreeSet<_>>();
    let mut allocator = || loop {
        let current = counter;
        counter = counter.saturating_add(1);
        let hash = digest("pg_mdm/preview_id/v1", &[&seed, &current.to_be_bytes()]);
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&hash[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let id = Uuid::from_bytes(bytes);
        if reserved.insert(id) {
            break id;
        }
    };
    let evaluation = evaluation::resolve_and_compare(evaluation::EvaluationInput {
        entity: &context.entity,
        definition_version: context.definition_version,
        publication_revision: old_revision,
        records: &sources,
        manual_matches,
        cannot_links,
        pair_decisions,
        limits,
        old_identity: &old_identity,
        old_reviews: &old_reviews,
        old_golden: &old_golden,
        old_resolution_facts: &old_resolution_facts,
        golden_rows: &golden_rows,
        overrides: &overrides,
        allocator: &mut allocator,
    })?;
    let evidence_level = if scoped { "exact_subjects" } else { "sampled" };
    let mut result = json!({
        "entity_name": context.entity.name,
        "mode": if scoped { "scoped" } else { "sampled" },
        "evidence_level": evidence_level,
        "exact": scoped,
        "definition_version": context.definition_version,
        "publication_revision": context.publication_revision,
        "data_as_of": "last successful publication; later source changes may be pending",
        "record_count": sources.len(),
        "candidate_pair_count": pair_count,
        "observed_work": {
            "candidate_pairs": pair_count,
            "union_facts": evaluation.resolution.accepted.len() + evaluation.resolution.rejected.len()
        },
        "changed_within_scope": evaluation.changed,
        "examples": {
            "pair_decisions": pair_examples,
            "identity": evaluation::semantic_identity(&evaluation.identity),
            "golden": evaluation::semantic_golden(&evaluation.golden),
            "reviews": evaluation::semantic_reviews(&evaluation.reviews)
        }
    });
    if scoped {
        result["requested_subjects"] = json!({
            "source_record_ids": source_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "mdm_ids": requested_mdm_ids.iter().map(ToString::to_string).collect::<Vec<_>>()
        });
        result["materialized_source_record_ids"] = json!(
            selected_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
        result["limitation"] = json!("Omitted records can change the full-entity result.");
    } else if let Some(sample_size) = sample_size {
        result["sample_size"] = json!(sample_size);
    }
    Ok(result)
}

#[pg_extern(
    name = "preview_entity",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.preview_entity(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'preview_entity_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn preview_entity(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only mdm.preview constructs this request.
        let request = unsafe { request.get::<PreviewRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("preview request is required".into()))?;
        let helper_owner = catalog::validate_helper_owner()?;
        let (_, selected) = catalog::validate_caller(&helper_owner)?;
        let options = parse_preview_options(&request.mode, &request.options)?;
        Spi::connect_mut(|client| {
            let context = load_context(client, &request.entity_name, &selected, false)?;
            Ok(JsonB(preview_run(client, &context, &selected, options)?))
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_relation_sql_accepts_only_pgtrickle_payloads() {
        let schema = ["pgtrickle", "changes"].join("_");
        assert_eq!(
            delta_relation_sql(&format!("{schema}.output_delta_42")).unwrap(),
            format!("\"{schema}\".\"output_delta_42\"")
        );
        assert!(delta_relation_sql("pg_catalog.pg_class").is_err());
        assert!(delta_relation_sql(&format!("{schema}.bad.table")).is_err());
    }

    #[test]
    fn delta_protocol_validates_metadata_and_typed_payload() {
        let batch = DeltaBatch {
            token: 7,
            row_count: 2,
            rows_inserted: 1,
            rows_deleted: 1,
            mode: "EXACT".into(),
            contract_digest: vec![1, 2, 3],
            row_identity_version: 2,
        };
        assert_eq!(validate_delta_batch(&batch, 7, &[1, 2, 3], 2), Ok(()));
        assert_eq!(
            validate_delta_payload(
                &[
                    DeltaRow {
                        action: "DELETE".into(),
                        row_identity: vec![9],
                        source_record_ids: vec![Uuid::from_bytes([1; 16])],
                    },
                    DeltaRow {
                        action: "INSERT".into(),
                        row_identity: vec![8],
                        source_record_ids: vec![Uuid::from_bytes([2; 16])],
                    },
                ],
                &batch,
            ),
            Ok(())
        );
    }

    #[test]
    fn delta_protocol_rejects_gaps_and_inconsistent_payloads() {
        let mut batch = DeltaBatch {
            token: 7,
            row_count: 1,
            rows_inserted: 1,
            rows_deleted: 0,
            mode: "EXACT".into(),
            contract_digest: vec![1],
            row_identity_version: 2,
        };
        assert_eq!(
            validate_delta_batch(&batch, 6, &[1], 2),
            Err(MdmError::DeltaProtocol(
                "invalid batch metadata for token 7".into()
            ))
        );
        assert_eq!(
            validate_delta_payload(
                &[DeltaRow {
                    action: "UPDATE".into(),
                    row_identity: vec![9],
                    source_record_ids: vec![Uuid::from_bytes([1; 16])],
                }],
                &batch,
            ),
            Err(MdmError::DeltaProtocol("payload action is invalid".into()))
        );

        batch.mode = "FULL_INVALIDATION".into();
        batch.row_count = 0;
        batch.rows_inserted = 0;
        assert_eq!(validate_delta_batch(&batch, 7, &[1], 2), Ok(()));
        assert_eq!(validate_delta_payload(&[], &batch), Ok(()));

        batch.rows_inserted = 1;
        assert_eq!(
            validate_delta_batch(&batch, 7, &[1], 2),
            Err(MdmError::DeltaProtocol(
                "invalid batch metadata for token 7".into()
            ))
        );
        batch.rows_inserted = 0;
        batch.contract_digest = vec![2];
        assert_eq!(
            validate_delta_batch(&batch, 7, &[1], 2),
            Err(MdmError::DeltaProtocol(
                "invalid batch metadata for token 7".into()
            ))
        );
        batch.contract_digest = vec![1];
        batch.row_identity_version = 3;
        assert_eq!(
            validate_delta_batch(&batch, 7, &[1], 2),
            Err(MdmError::DeltaProtocol(
                "invalid batch metadata for token 7".into()
            ))
        );
    }

    #[test]
    fn node_mode_summary_counts_modes_and_fallback_reasons() {
        assert_eq!(
            node_mode_summary(&json!({
                "z": {"identity": "z", "action": "FULL"},
                "a": {"identity": "a", "action": "DIFFERENTIAL"},
                "b": {
                    "identity": "b",
                    "action": "FULL",
                    "fallback_reason": "unproven boundary"
                }
            })),
            (
                json!({"DIFFERENTIAL": 1, "FULL": 2}),
                json!([{"logical_id": "b", "reason": "unproven boundary"}]),
            )
        );
    }

    #[test]
    fn stable_graph_contract_ignores_only_provider_generation_counters() {
        let installed = json!({
            "contract_version": 1,
            "graph_digest": "installed",
            "members": [{
                "oid": 42,
                "identity": "mdm_graph.member",
                "contract_digest": "stable",
                "contract_generation": 12,
                "orchestration_mode": "EXTERNAL"
            }]
        });
        let refreshed = json!({
            "contract_version": 1,
            "graph_digest": "refreshed",
            "members": [{
                "oid": 42,
                "identity": "mdm_graph.member",
                "contract_digest": "stable",
                "contract_generation": 22,
                "orchestration_mode": "EXTERNAL"
            }]
        });
        assert_eq!(
            stable_graph_contract(&refreshed),
            json!({
                "contract_version": 1,
                "members": [{
                    "oid": 42,
                    "identity": "mdm_graph.member",
                    "contract_digest": "stable",
                    "orchestration_mode": "EXTERNAL"
                }]
            })
        );
        let changed = json!({
            "contract_version": 1,
            "members": [{
                "oid": 42,
                "identity": "mdm_graph.member",
                "contract_digest": "changed",
                "contract_generation": 22,
                "orchestration_mode": "EXTERNAL"
            }]
        });
        assert_ne!(
            stable_graph_contract(&installed),
            stable_graph_contract(&changed)
        );
    }

    #[test]
    fn evidence_reader_fetches_at_most_one_row_past_its_candidate_ceiling() {
        assert_eq!(pair_evidence_fetch_limit(10, 3).unwrap(), (30, 31));
        assert_eq!(pair_evidence_fetch_limit(10, 0).unwrap(), (10, 11));
        assert!(pair_evidence_fetch_limit(usize::MAX, 2).is_err());
    }

    #[test]
    fn refresh_metadata_rejects_unproven_boundaries() {
        assert_eq!(
            normalized_state("value").expect("value is a valid state"),
            NormalizedState::Value
        );
        assert!(normalized_state("partial").is_err());
    }

    #[test]
    fn scoped_resolution_facts_keep_only_pairs_within_scope() {
        let id = |value| Uuid::from_bytes([value; 16]);
        let facts = json!([
            [
                "01010101-0101-0101-0101-010101010101:02020202-0202-0202-0202-020202020202",
                "accepted",
                "ALREADY_CONNECTED",
                ["name"]
            ],
            [
                "02020202-0202-0202-0202-020202020202:03030303-0303-0303-0303-030303030303",
                "rejected",
                "CANNOT_LINK",
                ["name"]
            ]
        ]);
        assert_eq!(
            scope_resolution_facts(&facts, &BTreeSet::from([id(1), id(2)])),
            json!([[
                "01010101-0101-0101-0101-010101010101:02020202-0202-0202-0202-020202020202",
                "accepted",
                "ALREADY_CONNECTED",
                ["name"]
            ]])
        );
    }

    #[test]
    fn semantic_state_comparison_ignores_row_order() {
        let id = |value| Uuid::from_bytes([value; 16]);
        let mut state = IdentityState {
            registry: vec![
                identity::IdentityRecord {
                    mdm_id: id(2),
                    created_revision: 1,
                    retired_revision: None,
                    status: identity::IdentityStatus::Active,
                },
                identity::IdentityRecord {
                    mdm_id: id(1),
                    created_revision: 1,
                    retired_revision: None,
                    status: identity::IdentityStatus::Active,
                },
            ],
            memberships: vec![
                identity::IdentityMembership {
                    source_record_id: id(4),
                    source_sort_key: vec![2],
                    mdm_id: id(2),
                    active: true,
                    first_membership_revision: 1,
                    last_membership_revision: 1,
                    membership_reason: "new".into(),
                    last_change_revision: 1,
                },
                identity::IdentityMembership {
                    source_record_id: id(3),
                    source_sort_key: vec![1],
                    mdm_id: id(1),
                    active: true,
                    first_membership_revision: 1,
                    last_membership_revision: 1,
                    membership_reason: "new".into(),
                    last_change_revision: 1,
                },
            ],
            ..IdentityState::default()
        };
        let expected_identity = evaluation::semantic_identity(&state);
        state.registry.reverse();
        state.memberships.reverse();
        assert_eq!(evaluation::semantic_identity(&state), expected_identity);

        let review = |key, reason: &str| Review {
            review_id: id(key),
            issue_key: [key; 32],
            occurrence: 1,
            status: ReviewStatus::Open,
            severity: "warning".into(),
            reason_code: reason.into(),
            subjects: json!([]),
            masked_summary: json!({}),
            opened_revision: 1,
            resolved_revision: None,
            last_change_revision: 1,
            concurrency_version: 1,
        };
        assert_eq!(
            evaluation::semantic_reviews(&[review(1, "FIRST"), review(2, "SECOND")]),
            evaluation::semantic_reviews(&[review(2, "SECOND"), review(1, "FIRST")]),
        );
    }

    #[test]
    fn scoped_identity_keeps_registry_rows_for_historical_memberships() {
        let id = |value| Uuid::from_bytes([value; 16]);
        let old = IdentityState {
            registry: vec![identity::IdentityRecord {
                mdm_id: id(2),
                created_revision: 1,
                retired_revision: Some(2),
                status: identity::IdentityStatus::Merged,
            }],
            memberships: vec![identity::IdentityMembership {
                source_record_id: id(3),
                source_sort_key: vec![3],
                mdm_id: id(2),
                active: false,
                first_membership_revision: 1,
                last_membership_revision: 2,
                membership_reason: "merged".into(),
                last_change_revision: 2,
            }],
            ..IdentityState::default()
        };

        let scoped = scope_identity(&old, &BTreeSet::from([id(3)]));
        assert_eq!(scoped.registry.len(), 1);
        assert_eq!(scoped.memberships.len(), 1);
        assert!(
            identity::reconcile(
                &scoped,
                &crate::resolver::Resolution {
                    memberships: Vec::new(),
                    accepted: Vec::new(),
                    rejected: Vec::new(),
                },
                3,
                &mut || id(4)
            )
            .is_ok()
        );
    }

    #[test]
    fn affected_splice_replaces_scope_and_preserves_untouched_projection() {
        let id = |value| Uuid::from_bytes([value; 16]);
        let old_identity = IdentityState {
            registry: vec![
                identity::IdentityRecord {
                    mdm_id: id(10),
                    created_revision: 1,
                    retired_revision: None,
                    status: identity::IdentityStatus::Active,
                },
                identity::IdentityRecord {
                    mdm_id: id(20),
                    created_revision: 1,
                    retired_revision: None,
                    status: identity::IdentityStatus::Active,
                },
            ],
            memberships: vec![
                identity::IdentityMembership {
                    source_record_id: id(1),
                    source_sort_key: vec![1],
                    mdm_id: id(10),
                    active: true,
                    first_membership_revision: 1,
                    last_membership_revision: 1,
                    membership_reason: "new".into(),
                    last_change_revision: 1,
                },
                identity::IdentityMembership {
                    source_record_id: id(2),
                    source_sort_key: vec![2],
                    mdm_id: id(20),
                    active: true,
                    first_membership_revision: 1,
                    last_membership_revision: 1,
                    membership_reason: "new".into(),
                    last_change_revision: 1,
                },
            ],
            ..IdentityState::default()
        };
        let merged_identity = splice_identity(
            &old_identity,
            &BTreeSet::from([id(1)]),
            &scope_identity(&old_identity, &BTreeSet::from([id(1)])),
        );
        assert_eq!(
            evaluation::semantic_identity(&merged_identity),
            evaluation::semantic_identity(&old_identity)
        );

        let old_golden = json!([
            [
                id(10).to_string(),
                "name",
                "old",
                "old",
                "selected",
                id(1).to_string(),
                "source_priority",
                1,
                "source",
                [id(1).to_string()]
            ],
            [
                id(20).to_string(),
                "name",
                "untouched",
                "untouched",
                "selected",
                id(2).to_string(),
                "source_priority",
                1,
                "source",
                [id(2).to_string()]
            ]
        ]);
        let mut scoped_golden = BTreeMap::new();
        scoped_golden.insert(
            (id(10), "name".into()),
            GoldenSelection {
                value: Some(json!("new")),
                normalized: Some("new".into()),
                canonical_bytes: Some(b"new".to_vec()),
                status: crate::golden::GoldenStatus::Selected,
                winning_source_record_id: Some(id(1)),
                policy: "source_priority".into(),
                policy_version: 1,
                tie_break: "source".into(),
                contributors: vec![id(1)],
                issues: Vec::new(),
            },
        );
        assert_eq!(
            splice_golden(&old_golden, &BTreeSet::from([id(10)]), &scoped_golden),
            json!([
                [
                    id(10).to_string(),
                    "name",
                    "new",
                    "new",
                    "selected",
                    id(1).to_string(),
                    "source_priority",
                    1,
                    "source",
                    [id(1).to_string()]
                ],
                [
                    id(20).to_string(),
                    "name",
                    "untouched",
                    "untouched",
                    "selected",
                    id(2).to_string(),
                    "source_priority",
                    1,
                    "source",
                    [id(2).to_string()]
                ]
            ])
        );

        let fact = crate::resolver::UnionFact {
            left_component_key: vec![1],
            right_component_key: vec![2],
            edge: crate::resolver::PairKey {
                left_source_record_id: id(1),
                right_source_record_id: id(2),
                left_sort_key: vec![1],
                right_sort_key: vec![2],
            },
            outcome: crate::resolver::UnionOutcome::Accepted,
            reason_code: "BRIDGE".into(),
            evidence_groups: vec!["name".into()],
        };
        let old_facts = json!([
            [format!("{}:{}", id(1), id(2)), "accepted", "OLD", ["name"]],
            [
                format!("{}:{}", id(3), id(4)),
                "rejected",
                "UNTOUCHED",
                ["email"]
            ]
        ]);
        assert_eq!(
            splice_resolution_facts(
                &old_facts,
                &BTreeSet::from([id(1), id(2)]),
                &crate::resolver::Resolution {
                    memberships: Vec::new(),
                    accepted: vec![fact],
                    rejected: Vec::new(),
                }
            ),
            json!([
                [
                    format!("{}:{}", id(1), id(2)),
                    "accepted",
                    "BRIDGE",
                    ["name"]
                ],
                [
                    format!("{}:{}", id(3), id(4)),
                    "rejected",
                    "UNTOUCHED",
                    ["email"]
                ]
            ])
        );
    }

    #[test]
    fn preview_options_reject_unknown_duplicate_and_empty_subjects() {
        assert!(parse_preview_options("validation", &json!({})).is_ok());
        assert!(parse_preview_options("validation", &json!({"limit": 2})).is_err());
        assert!(parse_preview_options("sampled", &json!({"sample_size": 2})).is_ok());
        assert!(parse_preview_options("sampled", &json!({"limit": 2})).is_err());
        assert!(parse_preview_options("scoped", &json!({})).is_err());
        assert!(
            parse_preview_options(
                "scoped",
                &json!({"source_record_ids": [
                    "00000000-0000-0000-0000-000000000001",
                    "00000000-0000-0000-0000-000000000001"
                ]})
            )
            .is_err()
        );
        assert!(
            parse_preview_options(
                "scoped",
                &json!({"source_record_ids": ["00000000-0000-0000-0000-000000000001"]})
            )
            .is_ok()
        );
        assert!(parse_preview_uuid("00000000-0000-0000-0000-000000000001").is_ok());
        assert!(parse_preview_uuid("00000000-0000-0000-0000-00000000000A").is_err());
        assert!(parse_preview_uuid("００００００００-0000-0000-0000-000000000001").is_err());
    }

    #[test]
    fn scoped_preview_rejects_decision_closure_over_limit() {
        let id = |value| Uuid::from_bytes([value; 16]);
        let context = Context {
            entity_id: String::new(),
            graph_binding_id: String::new(),
            entity: parse_entity(json!({
                "name": "customer",
                "sources": [], "fields": [], "matches": [], "golden_values": [],
                "preset": null, "limits": {"max_decision_closure": 2},
                "execution_role": null
            }))
            .expect("test entity parses"),
            definition_version: 1,
            decision_epoch: 0,
            publication_revision: 0,
            artifact_id: String::new(),
            artifact_digest: Vec::new(),
            graph_digest: Vec::new(),
            graph_contract: Value::Null,
            graph_root: String::new(),
            pair_stats_relation: None,
            evidence_relation: String::new(),
            golden_relation: String::new(),
        };
        let records = [id(1), id(2), id(3)]
            .into_iter()
            .map(|id| SourceRow {
                id,
                source_name: "test".into(),
                sort_key: id.as_bytes().to_vec(),
            })
            .collect::<Vec<_>>();
        let edges = [(id(1), id(2)), (id(2), id(3))]
            .into_iter()
            .map(
                |(left_source_record_id, right_source_record_id)| DecisionEdge {
                    decision_id: id(0),
                    left_source_record_id,
                    right_source_record_id,
                    decision: DecisionKind::Match,
                },
            )
            .collect::<Vec<_>>();

        assert!(matches!(
            expand_decision_scope(
                &context,
                BTreeSet::from([id(1)]),
                &records,
                &IdentityState::default(),
                &[],
                &edges,
                &[],
            ),
            Err(MdmError::ResolverLimit {
                resource: "max_decision_closure",
                observed: 3,
                limit: 2
            })
        ));
    }
}
