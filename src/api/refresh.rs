use std::collections::{BTreeMap, BTreeSet};

use pgrx::prelude::*;
use pgrx::spi::SpiClient;
use pgrx::{Internal, JsonB, Uuid, default};
use serde::Serialize;
use serde_json::{Value, json};

use crate::candidate::CandidatePair;
use crate::catalog;
use crate::constraint::{DecisionEdge, DecisionKind};
use crate::definition::canonical::{digest, hex, json_bytes};
use crate::definition::{Entity, parse_entity};
use crate::error::MdmError;
use crate::evidence::{EvidenceClass, EvidenceItem};
use crate::golden::{GoldenCandidate, GoldenOverride, GoldenSelection};
use crate::identity::{self, IdentityState};
use crate::normalization::NormalizedState;
use crate::output::{self, OutputField};
use crate::pair::decide_pair_for_rules;
use crate::resolver::{ResolverInput, ResolverLimits, ResolverRecord};
use crate::review::{Review, ReviewCandidate, ReviewStatus, Subject};
use crate::source_record::quote_identifier as quote_sql_identifier;

struct RefreshRequest {
    entity_name: String,
    full_policy: String,
    rebuild: bool,
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

#[derive(Clone, Debug)]
struct GoldenRow {
    source_record_id: Uuid,
    source_name: String,
    field: String,
    raw_value: Option<Value>,
    row_changed_at: Option<i64>,
    state: NormalizedState,
    normalized: Option<String>,
    canonical_bytes: Option<Vec<u8>>,
}

type PairEvidence = (CandidatePair, Vec<(EvidenceItem, String)>);

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

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
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

fn source_priority(entity: &Entity, source_name: &str) -> u32 {
    entity
        .sources
        .iter()
        .position(|source| source.name == source_name)
        .unwrap_or(entity.sources.len()) as u32
}

fn source_authority(entity: &Entity, source_name: &str) -> BTreeMap<String, String> {
    entity
        .sources
        .iter()
        .find(|source| source.name == source_name)
        .map(|source| {
            source
                .authority
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.into())))
                .collect()
        })
        .unwrap_or_default()
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

fn grant_refresh_access(entity_name: &str) -> Result<(), MdmError> {
    let helper_owner = catalog::validate_helper_owner()?;
    let relations = catalog::call_helper(
        "refresh_access",
        RefreshAccessRequest {
            entity_name: entity_name.to_owned(),
        },
    )?
    .0
    .get("relations")
    .and_then(Value::as_array)
    .ok_or_else(|| MdmError::Spi("refresh access helper returned invalid relations".into()))?
    .iter()
    .map(|relation| {
        relation
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MdmError::Spi("refresh access relation is not text".into()))
    })
    .collect::<Result<Vec<_>, _>>()?;
    Spi::connect_mut(|client| {
        for relation_name in relations {
            client
                .update(
                    &format!(
                        "GRANT SELECT ON {relation_name} TO {}",
                        quote_sql_identifier(&helper_owner.name)
                    ),
                    None,
                    &[],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
        }
        Ok(())
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
            Ok(JsonB(json!({"relations": relations})))
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
    let limit = i64::try_from(max_active_records.saturating_add(1)).map_err(|_| {
        MdmError::ResolverInvalid("max_active_records cannot be represented as bigint".into())
    })?;
    let rows = client
        .select(
            "SELECT r.source_record_id, s.source_name::text, r.source_record_key FROM mdm_internal.source_records r JOIN mdm_internal.source_identities s ON s.source_identity_id = r.source_identity_id WHERE r.entity_id = $1::pg_catalog.uuid AND r.active ORDER BY r.source_record_key, r.source_record_id LIMIT $2::bigint",
            None,
            &[context.entity_id.clone().into(), limit.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if rows.len() > max_active_records {
        return Err(MdmError::ResolverLimit {
            resource: "max_active_records",
            observed: rows.len(),
            limit: max_active_records,
        });
    }
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
        let strength = context
            .entity
            .matches
            .iter()
            .find(|rule| rule.name == item.rule)
            .map(|rule| rule.strength.clone())
            .unwrap_or_else(|| "supporting".into());
        let key = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        let entry = grouped.entry(key).or_insert((pair, Vec::new()));
        entry.1.push((item, strength));
    }
    Ok(grouped
        .into_values()
        .map(|(pair, evidence)| {
            let authority_conflict = source_names
                .get(&pair.left_source_record_id)
                .zip(source_names.get(&pair.right_source_record_id))
                .is_some_and(|(left, right)| {
                    let left = source_authority(&context.entity, left);
                    let right = source_authority(&context.entity, right);
                    left.iter().any(|(field, left_value)| {
                        right
                            .get(field)
                            .is_some_and(|right_value| left_value != right_value)
                    })
                });
            decide_pair_for_rules(
                pair,
                evidence.into_iter().map(|(item, _)| item).collect(),
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

fn issue_candidates(
    context: &Context,
    rejected: &[crate::resolver::UnionFact],
    golden: &BTreeMap<(Uuid, String), GoldenSelection>,
) -> Vec<ReviewCandidate> {
    let mut result = rejected
        .iter()
        .map(|fact| {
            let subjects = vec![Subject::uuid("source_record", fact.edge.left_source_record_id), Subject::uuid("source_record", fact.edge.right_source_record_id)];
            ReviewCandidate::new(context.definition_version, "warning", fact.reason_code.clone(), &subjects, json!({"left_source_record_id": fact.edge.left_source_record_id.to_string(), "right_source_record_id": fact.edge.right_source_record_id.to_string()}), json!({"reason_code": fact.reason_code, "evidence_groups": fact.evidence_groups}))
        })
        .collect::<Vec<_>>();
    for ((mdm_id, field), selection) in golden {
        for issue in &selection.issues {
            let subject = Subject::new(
                "golden",
                [mdm_id.as_bytes().as_slice(), field.as_bytes()].concat(),
            );
            result.push(ReviewCandidate::new(
                context.definition_version,
                "warning",
                issue.as_str(),
                &[subject],
                json!({"mdm_id": mdm_id.to_string(), "field": field}),
                json!({"issue": issue.as_str()}),
            ));
        }
    }
    result
}

fn load_golden(
    client: &mut SpiClient<'_>,
    context: &Context,
    memberships: &identity::IdentityState,
) -> Result<BTreeMap<(Uuid, String), GoldenSelection>, MdmError> {
    let rows = load_golden_rows(client, context)?;
    let overrides = load_overrides(client, context)?;
    let by_record = memberships
        .memberships
        .iter()
        .filter(|membership| membership.active)
        .map(|membership| (membership.source_record_id, membership.mdm_id))
        .collect::<BTreeMap<_, _>>();
    let mut candidates: BTreeMap<(Uuid, String), Vec<GoldenCandidate>> = BTreeMap::new();
    for row in rows {
        let Some(&mdm_id) = by_record.get(&row.source_record_id) else {
            continue;
        };
        let Some(definition) = context
            .entity
            .golden_values
            .iter()
            .find(|definition| definition.field == row.field)
        else {
            continue;
        };
        if definition
            .sources
            .as_ref()
            .is_some_and(|sources| !sources.iter().any(|source| source == &row.source_name))
        {
            continue;
        }
        candidates
            .entry((mdm_id, row.field.clone()))
            .or_default()
            .push(GoldenCandidate {
                source_record_id: row.source_record_id,
                source_name: row.source_name.clone(),
                source_priority: source_priority(&context.entity, &row.source_name),
                row_changed_at: row.row_changed_at,
                authoritative: context
                    .entity
                    .sources
                    .iter()
                    .find(|source| source.name == row.source_name)
                    .and_then(|source| source.authority.get(&row.field))
                    .is_some(),
                source_sort_key: memberships
                    .memberships
                    .iter()
                    .find(|membership| membership.source_record_id == row.source_record_id)
                    .map(|membership| membership.source_sort_key.clone())
                    .unwrap_or_default(),
                raw_value: row.raw_value,
                state: row.state,
                normalized: row.normalized,
                canonical_bytes: row.canonical_bytes,
            });
    }
    let mut result = BTreeMap::new();
    for definition in &context.entity.golden_values {
        for identity in memberships
            .registry
            .iter()
            .filter(|identity| identity.status == identity::IdentityStatus::Active)
        {
            let key = (identity.mdm_id, definition.field.clone());
            let selection = crate::golden::select_golden(
                &definition.policy,
                candidates.get(&key).map(Vec::as_slice).unwrap_or(&[]),
                overrides
                    .get(&definition.field)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            )?;
            result.insert(key, selection);
        }
    }
    Ok(result)
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

fn semantic_identity(state: &IdentityState) -> Value {
    json!({"registry": state.registry.iter().map(|row| json!([row.mdm_id.to_string(), row.status])).collect::<Vec<_>>(), "memberships": state.memberships.iter().map(|row| json!([row.source_record_id.to_string(), row.source_sort_key, row.mdm_id.to_string(), row.active, row.first_membership_revision, row.last_membership_revision, row.membership_reason])).collect::<Vec<_>>(), "aliases": state.aliases.iter().map(|row| json!([row.alias_mdm_id.to_string(), row.canonical_mdm_id.to_string()])).collect::<Vec<_>>(), "splits": state.splits.iter().map(|row| json!([row.parent_mdm_id.to_string(), row.child_mdm_id.to_string()])).collect::<Vec<_>>()})
}

fn semantic_reviews(reviews: &[Review]) -> Value {
    json!(
        reviews
            .iter()
            .map(|row| json!([
                row.issue_key,
                row.occurrence,
                match row.status {
                    ReviewStatus::Open => "open",
                    ReviewStatus::Resolved => "resolved",
                },
                row.severity,
                row.reason_code,
                row.subjects,
                row.masked_summary
            ]))
            .collect::<Vec<_>>()
    )
}

fn semantic_golden(golden: &BTreeMap<(Uuid, String), GoldenSelection>) -> Value {
    json!(
        golden
            .iter()
            .map(|((mdm_id, field), row)| json!([
                mdm_id.to_string(),
                field,
                row.value,
                row.normalized,
                row.status.as_str(),
                row.winning_source_record_id.map(|id| id.to_string()),
                row.policy,
                row.policy_version,
                row.tie_break,
                row.contributors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            ]))
            .collect::<Vec<_>>()
    )
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
        let graph = refresh_graph(client, &context, &request.full_policy)?;
        let limits = load_limits(&context);
        limits.validate()?;
        let sources = load_sources(client, &context, limits.max_active_records)?;
        let active = sources.iter().map(|row| row.id).collect::<BTreeSet<_>>();
        let records = sources
            .iter()
            .map(|row| ResolverRecord {
                source_record_id: row.id,
                source_sort_key: row.sort_key.clone(),
                authority: source_authority(&context.entity, &row.source_name),
            })
            .collect::<Vec<_>>();
        let (manual_matches, cannot_links) = load_decisions(client, &context, &active)?;
        let source_names = sources
            .iter()
            .map(|source| (source.id, source.source_name.clone()))
            .collect::<BTreeMap<_, _>>();
        let pair_decisions = load_pair_decisions(client, &context, &active, &source_names)?;
        let resolution = crate::resolver::resolve(ResolverInput {
            records,
            manual_matches,
            cannot_links,
            pair_decisions,
            limits,
        })?;
        let old_identity = load_old_identity(client, &context)?;
        let old_reviews = load_reviews(client, &context)?;
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
        let next_identity =
            identity::reconcile(&old_identity, &resolution, revision, &mut allocator)?;
        let next_golden = load_golden(client, &context, &next_identity)?;
        let candidates = issue_candidates(&context, &resolution.rejected, &next_golden);
        let next_reviews =
            crate::review::reconcile(&old_reviews, &candidates, revision, &mut allocator);
        let changed = context.publication_revision == 0
            || semantic_identity(&old_identity) != semantic_identity(&next_identity)
            || semantic_reviews(&old_reviews) != semantic_reviews(&next_reviews)
            || semantic_golden(&next_golden) != load_current_golden(client, &context)?;
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
                    &json!({"identity": semantic_identity(&next_identity), "golden": semantic_golden(&next_golden), "reviews": semantic_reviews(&next_reviews)}),
                )],
            );
            client.update("INSERT INTO mdm_internal.publications (entity_id, publication_revision, definition_version, decision_epoch, operation_id, result_digest) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5::pg_catalog.uuid, $6)", None, &[context.entity_id.clone().into(), revision.into(), context.definition_version.into(), context.decision_epoch.into(), operation_id.clone().into(), result_digest.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            for row in &next_identity.registry {
                client.update("INSERT INTO mdm_internal.identity_registry (entity_id, mdm_id, created_revision, retired_revision, status) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5) ON CONFLICT (entity_id, mdm_id) DO UPDATE SET retired_revision = EXCLUDED.retired_revision, status = EXCLUDED.status", None, &[context.entity_id.clone().into(), row.mdm_id.into(), row.created_revision.into(), row.retired_revision.into(), format!("{:?}", row.status).to_lowercase().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_identity.memberships {
                client.update("INSERT INTO mdm_internal.memberships (entity_id, source_record_id, source_name, source_id, mdm_id, active, first_membership_revision, last_membership_revision, membership_reason, last_change_revision) SELECT $1::pg_catalog.uuid, $2, s.source_name::name, pg_catalog.jsonb_build_object('source_record_key', pg_catalog.encode(r.source_record_key, 'hex')), $3, $4, $5, $6, $7, $8 FROM mdm_internal.source_records r JOIN mdm_internal.source_identities s ON s.source_identity_id = r.source_identity_id WHERE r.source_record_id = $2", None, &[context.entity_id.clone().into(), row.source_record_id.into(), row.mdm_id.into(), row.active.into(), row.first_membership_revision.into(), row.last_membership_revision.into(), row.membership_reason.clone().into(), row.last_change_revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_identity.aliases {
                client.update("INSERT INTO mdm_internal.identity_aliases (entity_id, alias_mdm_id, canonical_mdm_id, publication_revision) VALUES ($1::pg_catalog.uuid, $2, $3, $4) ON CONFLICT DO NOTHING", None, &[context.entity_id.clone().into(), row.alias_mdm_id.into(), row.canonical_mdm_id.into(), row.publication_revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_identity.splits {
                client.update("INSERT INTO mdm_internal.identity_splits (entity_id, parent_mdm_id, child_mdm_id, publication_revision) VALUES ($1::pg_catalog.uuid, $2, $3, $4) ON CONFLICT DO NOTHING", None, &[context.entity_id.clone().into(), row.parent_mdm_id.into(), row.child_mdm_id.into(), row.publication_revision.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for ((mdm_id, field), selection) in &next_golden {
                client.update("INSERT INTO mdm_internal.golden_provenance (entity_id, publication_revision, mdm_id, field_name, value, normalized_value, status, winning_source_record_id, policy, policy_version, tie_break, contributors, definition_version) VALUES ($1::pg_catalog.uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)", None, &[context.entity_id.clone().into(), revision.into(), (*mdm_id).into(), field.clone().into(), selection.value.clone().map(JsonB).into(), selection.normalized.clone().into(), selection.status.as_str().into(), selection.winning_source_record_id.into(), selection.policy.clone().into(), (selection.policy_version as i16).into(), selection.tie_break.clone().into(), JsonB(json!(selection.contributors.iter().map(ToString::to_string).collect::<Vec<_>>())).into(), context.definition_version.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for (number, fact) in resolution
                .accepted
                .iter()
                .chain(resolution.rejected.iter())
                .enumerate()
            {
                client.update("INSERT INTO mdm_internal.resolution_facts (entity_id, publication_revision, fact_number, subject_kind, subject_key, fact_kind, fact) VALUES ($1::pg_catalog.uuid, $2, $3, 'pair', $4, $5, $6)", None, &[context.entity_id.clone().into(), revision.into(), (number as i64 + 1).into(), format!("{}:{}", fact.edge.left_source_record_id, fact.edge.right_source_record_id).into(), (match fact.outcome { crate::resolver::UnionOutcome::Accepted => "accepted", crate::resolver::UnionOutcome::Rejected => "rejected" }).into(), JsonB(json!({"reason_code": fact.reason_code, "evidence_groups": fact.evidence_groups})).into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            for row in &next_reviews {
                client.update("INSERT INTO mdm_internal.reviews (review_id, entity_id, issue_key, occurrence, status, severity, reason_code, subjects, masked_summary, opened_revision, resolved_revision, last_change_revision, concurrency_version) VALUES ($1, $2::pg_catalog.uuid, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) ON CONFLICT (review_id) DO UPDATE SET status = EXCLUDED.status, severity = EXCLUDED.severity, reason_code = EXCLUDED.reason_code, subjects = EXCLUDED.subjects, masked_summary = EXCLUDED.masked_summary, resolved_revision = EXCLUDED.resolved_revision, last_change_revision = EXCLUDED.last_change_revision, concurrency_version = EXCLUDED.concurrency_version", None, &[row.review_id.into(), context.entity_id.clone().into(), row.issue_key.to_vec().into(), row.occurrence.into(), (match row.status { ReviewStatus::Open => "open", ReviewStatus::Resolved => "resolved" }).into(), row.severity.clone().into(), row.reason_code.clone().into(), JsonB(row.subjects.clone()).into(), JsonB(row.masked_summary.clone()).into(), row.opened_revision.into(), row.resolved_revision.into(), row.last_change_revision.into(), row.concurrency_version.into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            persist_outputs(
                client,
                &context,
                &fields,
                &next_identity,
                &next_golden,
                &next_reviews,
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
    if let Err(error) = grant_refresh_access(&entity_name) {
        crate::raise(error);
    }
    catalog::call_helper(
        "persist_refresh",
        RefreshRequest {
            entity_name,
            full_policy,
            rebuild: false,
        },
    )
    .unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(name = "rebuild", requires = [persist_refresh], sql = "CREATE FUNCTION mdm_admin.rebuild(entity_name text, full_policy text DEFAULT 'ALLOW') RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'rebuild_wrapper';")]
pub(crate) fn rebuild(entity_name: String, full_policy: default!(String, "'ALLOW'")) -> JsonB {
    if let Err(error) = grant_refresh_access(&entity_name) {
        crate::raise(error);
    }
    catalog::call_helper(
        "persist_refresh",
        RefreshRequest {
            entity_name,
            full_policy,
            rebuild: true,
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
        if !matches!(request.mode.as_str(), "validation" | "sampled" | "scoped") {
            return Err(MdmError::DefinitionInvalid(
                "preview mode must be validation, sampled, or scoped".into(),
            ));
        }
        Spi::connect(|client| {
            let context = load_context(client, &request.entity_name, &selected, false)?;
            let result = json!({"entity_name": context.entity.name, "mode": request.mode, "exact": request.mode == "scoped", "definition_version": context.definition_version, "graph_digest": hex(&context.graph_digest), "source_records": client.select("SELECT count(*) FROM mdm_internal.source_records WHERE entity_id = $1::pg_catalog.uuid AND active", Some(1), &[context.entity_id.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?.first().get::<i64>(1).map_err(|error| MdmError::Spi(error.to_string()))?.unwrap_or(0), "options": request.options});
            Ok(JsonB(result))
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
}
