use std::collections::{BTreeMap, BTreeSet};

use pgrx::prelude::*;
use pgrx::spi::SpiClient;
use pgrx::{Internal, JsonB, Uuid, default};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
    entity: Entity,
    definition_version: i64,
    decision_epoch: i64,
    publication_revision: i64,
    artifact_id: String,
    graph_digest: Vec<u8>,
    graph_root: String,
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

fn load_context(
    client: &SpiClient<'_>,
    entity_name: &str,
    selected: &catalog::Role,
    lock: bool,
) -> Result<Context, MdmError> {
    let suffix = if lock { " FOR UPDATE OF e" } else { "" };
    let query = format!(
        "SELECT e.entity_id::text, e.desired_version, e.decision_epoch, e.publication_revision, e.execution_role_name, b.role_oid, d.expanded_definition, a.artifact_id::text FROM mdm_internal.entities e JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id JOIN mdm_internal.definitions d ON d.entity_id = e.entity_id AND d.definition_version = e.desired_version JOIN LATERAL (SELECT artifact_id FROM mdm_internal.definition_artifacts x WHERE x.entity_id = d.entity_id AND x.definition_version = d.definition_version ORDER BY x.artifact_id DESC LIMIT 1) a ON true WHERE e.entity_name = $1::pg_catalog.name{suffix}"
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
    let binding = client
        .select(
            "SELECT b.graph_binding_id::text, b.graph_digest, gm.relation_name FROM mdm_internal.graph_bindings b JOIN mdm_internal.graph_members gm ON gm.graph_binding_id = b.graph_binding_id AND gm.logical_id = $3 WHERE b.entity_id = $1::pg_catalog.uuid AND b.definition_version = $2 ORDER BY b.graph_generation DESC LIMIT 1",
            Some(1),
            &[entity_id.clone().into(), definition_version.into(), format!("golden/{}", entity_name).into()],
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
    let members = client
        .select(
            "SELECT m.logical_id, m.relation_name, m.relation_oid, pg_catalog.to_regclass(m.relation_name)::pg_catalog.oid, c.relowner FROM mdm_internal.graph_members m LEFT JOIN pg_catalog.pg_class c ON c.oid = m.relation_oid WHERE m.graph_binding_id = $1::pg_catalog.uuid ORDER BY m.topological_ordinal",
            None,
            &[binding_id.into()],
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
        entity,
        definition_version,
        decision_epoch,
        publication_revision,
        artifact_id,
        graph_digest,
        graph_root,
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
            "DELETE FROM mdm_graph.source_identity_map WHERE entity_id = $1::pg_catalog.uuid; DELETE FROM mdm_graph.source_records WHERE entity_id = $1::pg_catalog.uuid; DELETE FROM mdm_graph.definition_limits WHERE entity_id = $1::pg_catalog.uuid",
            None,
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    client
        .update(
            "INSERT INTO mdm_graph.source_identity_map (entity_id, entity_name, source_identity_id, source_name) SELECT e.entity_id, e.entity_name::text, s.source_identity_id, s.source_name::text FROM mdm_internal.entities e JOIN mdm_internal.source_identities s ON s.entity_id = e.entity_id WHERE e.entity_id = $1::pg_catalog.uuid",
            None,
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    client
        .update(
            "INSERT INTO mdm_graph.source_records (entity_id, source_identity_id, source_record_key, source_record_id, active) SELECT entity_id, source_identity_id, source_record_key, source_record_id, active FROM mdm_internal.source_records WHERE entity_id = $1::pg_catalog.uuid",
            None,
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    client
        .update(
            "INSERT INTO mdm_graph.definition_limits (entity_id, entity_name, expanded_definition) SELECT e.entity_id, e.entity_name::text, d.expanded_definition FROM mdm_internal.entities e JOIN mdm_internal.definitions d ON d.entity_id = e.entity_id AND d.definition_version = e.desired_version WHERE e.entity_id = $1::pg_catalog.uuid",
            None,
            &[context.entity_id.clone().into()],
        )
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?;
    let rows = client
        .select(
            "SELECT contract_version, graph_digest FROM pgtrickle.graph_contract(ARRAY[$1::regclass])",
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
    let digest = contract
        .get::<Vec<u8>>(2)
        .map_err(|error| MdmError::RefreshFailed(error.to_string()))?
        .ok_or_else(|| MdmError::RefreshFailed("graph contract digest is NULL".into()))?;
    if version != 1 || digest != context.graph_digest {
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
                context.graph_digest.clone().into(),
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
    let digest = row
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
    if version != 1 || digest != context.graph_digest || boundary_digest.len() != 32 {
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
) -> Result<Vec<crate::pair::PairDecision>, MdmError> {
    let query = format!(
        "SELECT left_source_record_id, right_source_record_id, left_sort_key, right_sort_key, rule, evidence_group, class, score, comparator, comparator_version, left_value_digest, right_value_digest FROM {} ORDER BY left_sort_key, right_sort_key, rule",
        context.evidence_relation
    );
    let rows = client
        .select(&query, None, &[])
        .map_err(|error| MdmError::Spi(error.to_string()))?;
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
        if !active.contains(&left) || !active.contains(&right) {
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
    let query = format!(
        "SELECT source_record_id, source_name, field_name, raw_value::text, extract(epoch FROM row_changed_at)::bigint, state, normalized, canonical_bytes FROM {}",
        context.golden_relation
    );
    client
        .select(&query, None, &[])
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .into_iter()
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
    let rows = client.select("SELECT field_name::text, override_id, anchor_source_record_id, created_at, value FROM mdm_internal.golden_override_directives WHERE entity_id = $1::pg_catalog.uuid AND is_current AND action = 'SET' ORDER BY field_name, created_at, override_id", None, &[context.entity_id.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
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
    client.update("UPDATE mdm_internal.operations SET status = 'succeeded', result_code = 'MDM_OK', outcome = $2, completed_at = pg_catalog.statement_timestamp() WHERE operation_id = $1::pg_catalog.uuid AND status = 'running'", Some(1), &[operation_id.into(), JsonB(serde_json::to_value(result).map_err(|error| MdmError::OperationState(error.to_string()))?).into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
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
) -> Result<(), MdmError> {
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

fn persist_outputs(
    client: &mut SpiClient<'_>,
    context: &Context,
    fields: &[OutputField],
    identity: &IdentityState,
    golden: &BTreeMap<(Uuid, String), GoldenSelection>,
    reviews: &[Review],
    revision: i64,
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
    delete_stale_output_rows(client, &output_name, "mdm_id", &entity_ids)?;
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
    delete_stale_output_rows(client, &members_name, "source_record_id", &member_ids)?;
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
    delete_stale_output_rows(client, &review_name, "review_id", &review_ids)?;
    Ok(())
}

fn persist_refresh_inner(
    request: &RefreshRequest,
    session: &catalog::Role,
    selected: &catalog::Role,
) -> Result<RefreshResult, MdmError> {
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
        sync_source_records(client, &context, &request.source_snapshots)?;
        let graph = refresh_graph(client, &context, &request.full_policy)?;
        let limits = load_limits(&context);
        limits.validate()?;
        let sources = load_sources(client, &context, limits.max_active_records)?;
        let active = sources.iter().map(|row| row.id).collect::<BTreeSet<_>>();
        let records = sources
            .iter()
            .map(|row| EvaluationRecord {
                source_record_id: row.id,
                source_name: row.source_name.clone(),
                source_sort_key: row.sort_key.clone(),
            })
            .collect::<Vec<_>>();
        let (manual_matches, cannot_links) = load_decisions(client, &context, &active)?;
        let source_names = sources
            .iter()
            .map(|source| (source.id, source.source_name.clone()))
            .collect::<BTreeMap<_, _>>();
        let pair_decisions = load_pair_decisions(client, &context, &active, &source_names)?;
        let old_identity = load_old_identity(client, &context)?;
        let old_reviews = load_reviews(client, &context)?;
        let old_golden = load_current_golden(client, &context)?;
        let golden_rows = load_golden_rows(client, &context)?;
        let overrides = load_overrides(client, &context)?;
        let revision = context
            .publication_revision
            .checked_add(1)
            .ok_or_else(|| MdmError::OperationState("publication revision exhausted".into()))?;
        let mut ids = allocate_ids(
            client,
            sources
                .len()
                .saturating_add(old_reviews.len())
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
            golden_rows: &golden_rows,
            overrides: &overrides,
            allocator: &mut allocator,
        })?;
        let resolution = &evaluation.resolution;
        let next_identity = &evaluation.identity;
        let next_golden = &evaluation.golden;
        let next_reviews = &evaluation.reviews;
        let changed = evaluation.changed;
        let fields = ensure_output(client, &context)?;
        let publication_revision = if changed {
            revision
        } else {
            context.publication_revision
        };
        if changed {
            let result_digest = digest(
                "pg_mdm/publication/v1",
                &[&json_bytes(
                    &json!({"identity": evaluation::semantic_identity(next_identity), "golden": evaluation::semantic_golden(next_golden), "reviews": evaluation::semantic_reviews(next_reviews)}),
                )],
            );
            client.update("INSERT INTO mdm_internal.publications (entity_id, publication_revision, definition_version, decision_epoch, operation_id, result_digest) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5::pg_catalog.uuid, $6)", None, &[context.entity_id.clone().into(), revision.into(), context.definition_version.into(), context.decision_epoch.into(), operation_id.clone().into(), result_digest.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            for row in &next_identity.registry {
                client.update("INSERT INTO mdm_internal.identity_registry (entity_id, mdm_id, created_revision, retired_revision, status) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5) ON CONFLICT (entity_id, mdm_id) DO UPDATE SET retired_revision = EXCLUDED.retired_revision, status = EXCLUDED.status", None, &[context.entity_id.clone().into(), row.mdm_id.into(), row.created_revision.into(), row.retired_revision.into(), format!("{:?}", row.status).to_lowercase().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_identity.memberships {
                client.update("INSERT INTO mdm_internal.memberships AS target (entity_id, source_record_id, source_name, source_id, mdm_id, active, first_membership_revision, last_membership_revision, membership_reason, last_change_revision) SELECT $1::pg_catalog.uuid, $2, s.source_name::name, pg_catalog.jsonb_build_object('source_record_key', pg_catalog.encode(r.source_record_key, 'hex')), $3, $4, $5, $6, $7, $8 FROM mdm_internal.source_records r JOIN mdm_internal.source_identities s ON s.source_identity_id = r.source_identity_id WHERE r.source_record_id = $2 ON CONFLICT (entity_id, source_record_id) DO UPDATE SET source_name = EXCLUDED.source_name, source_id = EXCLUDED.source_id, mdm_id = EXCLUDED.mdm_id, active = EXCLUDED.active, first_membership_revision = EXCLUDED.first_membership_revision, last_membership_revision = EXCLUDED.last_membership_revision, membership_reason = EXCLUDED.membership_reason, last_change_revision = EXCLUDED.last_change_revision", None, &[context.entity_id.clone().into(), row.source_record_id.into(), row.mdm_id.into(), row.active.into(), row.first_membership_revision.into(), row.last_membership_revision.into(), row.membership_reason.clone().into(), row.last_change_revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_identity.aliases {
                client.update("INSERT INTO mdm_internal.identity_aliases (entity_id, alias_mdm_id, canonical_mdm_id, publication_revision) VALUES ($1::pg_catalog.uuid, $2, $3, $4) ON CONFLICT DO NOTHING", None, &[context.entity_id.clone().into(), row.alias_mdm_id.into(), row.canonical_mdm_id.into(), row.publication_revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_identity.splits {
                client.update("INSERT INTO mdm_internal.identity_splits (entity_id, parent_mdm_id, child_mdm_id, publication_revision) VALUES ($1::pg_catalog.uuid, $2, $3, $4) ON CONFLICT DO NOTHING", None, &[context.entity_id.clone().into(), row.parent_mdm_id.into(), row.child_mdm_id.into(), row.publication_revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for ((mdm_id, field), selection) in next_golden {
                client.update("INSERT INTO mdm_internal.golden_provenance (entity_id, publication_revision, mdm_id, field_name, value, normalized_value, status, winning_source_record_id, policy, policy_version, tie_break, contributors, definition_version) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)", None, &[context.entity_id.clone().into(), revision.into(), (*mdm_id).into(), field.clone().into(), selection.value.clone().map(JsonB).into(), selection.normalized.clone().into(), selection.status.as_str().into(), selection.winning_source_record_id.into(), selection.policy.clone().into(), (selection.policy_version as i16).into(), selection.tie_break.clone().into(), JsonB(json!(selection.contributors.iter().map(ToString::to_string).collect::<Vec<_>>())).into(), context.definition_version.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for (number, fact) in resolution
                .accepted
                .iter()
                .chain(resolution.rejected.iter())
                .enumerate()
            {
                client.update("INSERT INTO mdm_internal.resolution_facts (entity_id, publication_revision, fact_number, subject_kind, subject_key, fact_kind, fact) VALUES ($1::pg_catalog.uuid, $2, $3, 'pair', pg_catalog.convert_to($4::text, 'UTF8'), $5, $6)", None, &[context.entity_id.clone().into(), revision.into(), (number as i64 + 1).into(), format!("{}:{}", fact.edge.left_source_record_id, fact.edge.right_source_record_id).into(), (match fact.outcome { crate::resolver::UnionOutcome::Accepted => "accepted", crate::resolver::UnionOutcome::Rejected => "rejected" }).into(), JsonB(json!({"reason_code": fact.reason_code, "evidence_groups": fact.evidence_groups})).into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in next_reviews {
                client.update("INSERT INTO mdm_internal.reviews (review_id, entity_id, issue_key, occurrence, status, severity, reason_code, subjects, masked_summary, opened_revision, resolved_revision, last_change_revision, concurrency_version) VALUES ($1, $2::pg_catalog.uuid, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) ON CONFLICT (review_id) DO UPDATE SET status = EXCLUDED.status, severity = EXCLUDED.severity, reason_code = EXCLUDED.reason_code, subjects = EXCLUDED.subjects, masked_summary = EXCLUDED.masked_summary, resolved_revision = EXCLUDED.resolved_revision, last_change_revision = EXCLUDED.last_change_revision, concurrency_version = EXCLUDED.concurrency_version", None, &[row.review_id.into(), context.entity_id.clone().into(), row.issue_key.to_vec().into(), row.occurrence.into(), (match row.status { ReviewStatus::Open => "open", ReviewStatus::Resolved => "resolved" }).into(), row.severity.clone().into(), row.reason_code.clone().into(), JsonB(row.subjects.clone()).into(), JsonB(row.masked_summary.clone()).into(), row.opened_revision.into(), row.resolved_revision.into(), row.last_change_revision.into(), row.concurrency_version.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            persist_outputs(
                client,
                &context,
                &fields,
                next_identity,
                next_golden,
                next_reviews,
                revision,
            )?;
            client.update("UPDATE mdm_internal.entities SET active_version = desired_version, publication_revision = $2 WHERE entity_id = $1::pg_catalog.uuid", None, &[context.entity_id.clone().into(), revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
        }
        client.update("INSERT INTO mdm_internal.publication_observations (entity_id, publication_revision, decision_epoch, artifact_id, operation_id, graph_refresh_id, source_boundary, source_boundary_digest, node_results) VALUES ($1::pg_catalog.uuid, $2, $3, $4::pg_catalog.uuid, $5::pg_catalog.uuid, $6, $7, $8, $9)", None, &[context.entity_id.clone().into(), publication_revision.into(), context.decision_epoch.into(), context.artifact_id.clone().into(), operation_id.clone().into(), graph.id.into(), JsonB(graph.boundary.clone()).into(), graph.boundary_digest.clone().into(), JsonB(graph.node_results.clone()).into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
        let result = RefreshResult {
            operation_id: operation_id.clone(),
            entity_name: context.entity.name.clone(),
            changed,
            publication_revision,
            graph_refresh_id: graph.id,
            source_boundary: graph.boundary,
            source_boundary_digest: hex(&graph.boundary_digest),
            node_results: graph.node_results,
            active_records: sources.len(),
            identities: next_identity
                .registry
                .iter()
                .filter(|row| row.status == identity::IdentityStatus::Active)
                .count(),
            open_reviews: next_reviews
                .iter()
                .filter(|row| row.status == ReviewStatus::Open)
                .count(),
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
            "SELECT contract_version, graph_digest FROM pgtrickle.graph_contract(ARRAY[$1::regclass])",
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
    if version != Some(1) || digest.as_deref() != Some(context.graph_digest.as_slice()) {
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
    let pair_decisions = load_pair_decisions(client, context, &all_active, &source_names)?;
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
    fn refresh_metadata_rejects_unproven_boundaries() {
        assert_eq!(
            normalized_state("value").expect("value is a valid state"),
            NormalizedState::Value
        );
        assert!(normalized_state("partial").is_err());
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
}
