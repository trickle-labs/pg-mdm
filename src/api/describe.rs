use pgrx::prelude::*;
use pgrx::{Internal, JsonB};
use serde_json::{Value, json};

use crate::catalog;
use crate::definition::canonical::hex;
use crate::error::MdmError;

struct DescribeRequest {
    entity_name: String,
    format: String,
}

fn graph_summary(
    entity_id: &str,
    definition_version: i64,
    selected: &catalog::Role,
    capability_enabled: bool,
) -> Result<Value, MdmError> {
    let mut errors = Vec::new();
    if !capability_enabled {
        errors.push(json!({
            "code": "MDM_PGT_CAPABILITY_DISABLED",
            "message": "Graph V1 is disabled"
        }));
    }
    let binding = Spi::connect(|client| {
        let table = client
            .select(
                "SELECT b.graph_generation, encode(b.graph_binding_digest, 'hex'), b.graph_contract_version, encode(b.graph_digest, 'hex'), cardinality(b.root_relation_oids), b.root_relation_oids[1], (SELECT count(*) FROM mdm_internal.graph_members m WHERE m.graph_binding_id = b.graph_binding_id) FROM mdm_internal.graph_bindings b WHERE b.entity_id = $1::pg_catalog.uuid AND b.definition_version = $2 ORDER BY b.graph_generation DESC LIMIT 1",
                Some(1),
                &[entity_id.into(), definition_version.into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if table.is_empty() {
            return Ok::<_, MdmError>(None);
        }
        let row = table.first();
        Ok(Some((
            row.get::<i64>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("graph generation is NULL".into()))?,
            row.get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("graph binding digest is NULL".into()))?,
            row.get::<i16>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("graph contract version is NULL".into()))?,
            row.get::<String>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("graph digest is NULL".into()))?,
            row.get::<i32>(5)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(0),
            row.get::<pg_sys::Oid>(6)
                .map_err(|error| MdmError::Spi(error.to_string()))?,
            row.get::<i64>(7)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("graph member count is NULL".into()))?,
        )))
    })?;
    let Some((
        generation,
        binding_digest,
        contract_version,
        graph_digest,
        root_count,
        root_oid,
        member_count,
    )) = binding
    else {
        errors.push(json!({
            "code": "MDM_GRAPH_BINDING_MISSING",
            "message": "No graph binding is installed for the desired definition"
        }));
        return Ok(json!({
            "graph_state": "absent",
            "graph_generation": Value::Null,
            "graph_binding_digest": Value::Null,
            "graph_contract_version": Value::Null,
            "graph_digest": Value::Null,
            "member_count": 0,
            "root_count": 0,
            "blocking_errors": errors
        }));
    };
    if binding_digest.len() != 64 || graph_digest.len() != 64 || contract_version != 1 {
        errors.push(json!({
            "code": "MDM_GRAPH_BINDING_INVALID",
            "message": "Stored graph binding metadata is invalid"
        }));
    }
    let members_valid = Spi::get_one_with_args::<bool>(
        "SELECT NOT EXISTS (SELECT 1 FROM mdm_internal.graph_members m LEFT JOIN pg_catalog.pg_class c ON c.oid = m.relation_oid WHERE m.graph_binding_id = (SELECT b.graph_binding_id FROM mdm_internal.graph_bindings b WHERE b.entity_id = $1::pg_catalog.uuid AND b.definition_version = $2 ORDER BY b.graph_generation DESC LIMIT 1) AND (c.oid IS NULL OR c.relowner <> $3))",
        &[entity_id.into(), definition_version.into(), selected.oid.into()],
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .unwrap_or(false);
    if !members_valid {
        errors.push(json!({
            "code": "MDM_GRAPH_MEMBER_DRIFT",
            "message": "A graph member is missing or has a different owner"
        }));
    }
    if let Some(root_oid) = root_oid {
        let current_digest = Spi::get_one_with_args::<Vec<u8>>(
            "SELECT graph_digest FROM pgtrickle.graph_contract(ARRAY[$1::regclass])",
            &[root_oid.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
        if current_digest.as_deref().map(hex) != Some(graph_digest.clone()) {
            errors.push(json!({
                "code": "MDM_GRAPH_CONTRACT_DRIFT",
                "message": "The public Graph V1 contract no longer matches the binding"
            }));
        }
    } else {
        errors.push(json!({
            "code": "MDM_GRAPH_ROOT_MISSING",
            "message": "The graph root is missing"
        }));
    }
    Ok(json!({
        "graph_state": if errors.is_empty() { "ready" } else { "blocked" },
        "graph_generation": generation,
        "graph_binding_digest": binding_digest,
        "graph_contract_version": contract_version,
        "graph_digest": graph_digest,
        "member_count": member_count,
        "root_count": root_count,
        "blocking_errors": errors
    }))
}

#[pg_extern(
    name = "describe",
    requires = [describe_entity],
    sql = "CREATE FUNCTION mdm.describe(entity_name text, format text DEFAULT 'summary') RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'describe_wrapper';"
)]
pub(crate) fn describe(entity_name: String, format: Option<String>) -> JsonB {
    catalog::call_helper(
        "describe_entity",
        DescribeRequest {
            entity_name,
            format: format.unwrap_or_else(|| "summary".into()),
        },
    )
    .unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "describe_entity",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.describe_entity(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'describe_entity_wrapper';"
)]
pub(crate) fn describe_entity(request: Internal) -> JsonB {
    let result = (|| {
        let helper_owner = catalog::validate_helper_owner()?;
        // SAFETY: describe is the only caller that constructs DescribeRequest.
        let request = unsafe { request.get::<DescribeRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("describe request is required".into()))?;
        let entity_name = &request.entity_name;
        let format = &request.format;
        let (_, selected) = catalog::validate_caller(&helper_owner)?;
        if !matches!(format.as_str(), "summary" | "definition") {
            return Err(MdmError::DefinitionInvalid(
                "describe format must be summary or definition".into(),
            ));
        }
        let row = Spi::connect(|client| {
            let table = client.select(
                "SELECT e.entity_id::text, e.entity_name::text, e.desired_version, e.active_version, e.execution_role_name, d.expanded_definition, encode(d.definition_digest, 'hex'), encode(a.artifact_digest, 'hex'), b.role_oid, d.logical_candidate_plan, d.semantic_manifest FROM mdm_internal.entities e LEFT JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id JOIN mdm_internal.definitions d ON d.entity_id = e.entity_id AND d.definition_version = e.desired_version LEFT JOIN LATERAL (SELECT artifact_digest FROM mdm_internal.definition_artifacts x WHERE x.entity_id = d.entity_id AND x.definition_version = d.definition_version ORDER BY x.artifact_id DESC LIMIT 1) a ON true WHERE e.entity_name = $1::pg_catalog.name",
                Some(1), &[entity_name.clone().into()]).map_err(|error| MdmError::Spi(error.to_string()))?;
            if table.is_empty() {
                return Err(MdmError::DefinitionInvalid(format!(
                    "entity {entity_name} does not exist"
                )));
            }
            let row = table.first();
            let execution_role = row
                .get::<String>(5)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("execution role is NULL".into()))?;
            let bound_oid = row
                .get::<pg_sys::Oid>(9)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            if execution_role != selected.name || bound_oid != Some(selected.oid) {
                return Err(MdmError::Unauthorized(
                    "entity is bound to another execution role".into(),
                ));
            }
            Ok::<_, MdmError>((
                row.get::<String>(2)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("entity name is NULL".into()))?,
                row.get::<i64>(3)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("desired version is NULL".into()))?,
                row.get::<i64>(4)
                    .map_err(|error| MdmError::Spi(error.to_string()))?,
                row.get::<JsonB>(6)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("definition is NULL".into()))?
                    .0,
                row.get::<String>(7)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("definition digest is NULL".into()))?,
                row.get::<String>(8)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("artifact digest is NULL".into()))?,
                row.get::<JsonB>(10)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("candidate plan is NULL".into()))?
                    .0,
                row.get::<JsonB>(11)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::Spi("semantic manifest is NULL".into()))?
                    .0,
            ))
        })?;
        if format == "definition" {
            return Ok(JsonB(row.3));
        }
        let capabilities = crate::integration::integration_capabilities()?;
        let entity_id = Spi::get_one_with_args::<String>(
            "SELECT entity_id::text FROM mdm_internal.entities WHERE entity_name = $1::pg_catalog.name",
            &[entity_name.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("entity ID is NULL".into()))?;
        let graph = graph_summary(
            &entity_id,
            row.1,
            &selected,
            capabilities.external_graph_refresh.enabled,
        )?;
        let sources = Spi::get_one_with_args::<JsonB>(
            "SELECT COALESCE(jsonb_agg(jsonb_build_object('name', source_name::text, 'relation', relation_name, 'key_contract', key_contract, 'identity_digest', encode(identity_digest, 'hex')) ORDER BY source_name), '[]'::jsonb) FROM mdm_internal.source_identities s JOIN mdm_internal.entities e ON e.entity_id = s.entity_id WHERE e.entity_name = $1::pg_catalog.name",
            &[entity_name.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .map(|value| value.0)
        .unwrap_or_else(|| serde_json::json!([]));
        let decision_metadata = Spi::get_one_with_args::<JsonB>(
            "SELECT jsonb_build_object('decision_epoch', e.decision_epoch, 'publication_revision', e.publication_revision, 'current_decisions', (SELECT count(*) FROM mdm_internal.steward_decisions d WHERE d.entity_id = e.entity_id AND d.is_current)) FROM mdm_internal.entities e WHERE e.entity_name = $1::pg_catalog.name",
            &[entity_name.clone().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .map(|value| value.0)
        .unwrap_or_else(|| serde_json::json!({}));
        let publication_metadata = Spi::get_one_with_args::<JsonB>(
            "SELECT jsonb_build_object('publication_revision', e.publication_revision, 'output_schema', COALESCE((SELECT jsonb_agg(jsonb_build_object('name', f.field_name::text, 'ordinal', f.field_ordinal, 'type', f.field_type_name) ORDER BY f.field_ordinal) FROM mdm_internal.output_fields f WHERE f.entity_id = e.entity_id), '[]'::jsonb), 'entity_count', (SELECT count(*) FROM mdm_internal.identity_registry i WHERE i.entity_id = e.entity_id AND i.status = 'active'), 'review_count', (SELECT count(*) FROM mdm_internal.reviews r WHERE r.entity_id = e.entity_id AND r.status = 'open'), 'identity_policy_version', $2, 'golden_policy_version', $3, 'result_digest', (SELECT encode(p.result_digest, 'hex') FROM mdm_internal.publications p WHERE p.entity_id = e.entity_id AND p.publication_revision = e.publication_revision), 'earliest_retained_explanation_revision', (SELECT min(f.publication_revision) FROM mdm_internal.resolution_facts f WHERE f.entity_id = e.entity_id), 'pending_stewardship', COALESCE((SELECT o.decision_epoch <> e.decision_epoch FROM mdm_internal.publication_observations o WHERE o.entity_id = e.entity_id ORDER BY o.observed_at DESC, o.observation_id DESC LIMIT 1), e.decision_epoch > 0)) FROM mdm_internal.entities e WHERE e.entity_name = $1::pg_catalog.name",
            &[
                entity_name.clone().into(),
                (crate::semantics::STABLE_ID_POLICY_VERSION as i32).into(),
                (crate::semantics::GOLDEN_POLICY_VERSION as i32).into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .map(|value| value.0)
        .unwrap_or_else(|| serde_json::json!({}));
        let selected_cleaners = row.3.get("fields")
            .and_then(Value::as_array)
            .map(|fields| {
                fields.iter().map(|f| serde_json::json!({
                    "field": f.get("name").and_then(Value::as_str).unwrap_or(""),
                    "cleaner": f.get("cleaner").and_then(Value::as_str).unwrap_or(""),
                    "version": 1,
                    "options": f.get("cleaner_options").cloned().unwrap_or_else(|| serde_json::json!({}))
                })).collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(JsonB(serde_json::json!({
            "entity_name": entity_name,
            "desired_version": row.1,
            "active_version": row.2,
            "definition_digest": row.4,
            "artifact_digest": row.5,
            "execution_role": selected.name,
            "source_key_encoding": 2,
            "row_identity_version": 2,
            "cleaners": selected_cleaners,
            "cleaner_versions": crate::cleaners::v1_cleaner_registry(),
            "candidate_plan": row.6,
            "candidate_semantics": row.7.clone(),
            "evidence_semantics": row.7.get("evidence").cloned().unwrap_or_else(|| serde_json::json!({})),
            "comparators": row.7.get("evidence").and_then(|value| value.get("comparators")).cloned().unwrap_or_else(|| serde_json::json!({})),
            "pair_decision_policy": 1,
            "clustering_policy": row.7.get("clustering").and_then(|value| value.get("policy_version")).cloned().unwrap_or_else(|| serde_json::json!(crate::semantics::CLUSTERING_POLICY_VERSION)),
            "clustering_admission": row.7.get("clustering").and_then(|value| value.get("admission")).cloned().unwrap_or_else(|| serde_json::json!({})),
            "resolver_limits": row.3.get("limits").cloned().unwrap_or_else(|| serde_json::json!({})),
            "decisions": decision_metadata,
            "publication": publication_metadata,
            "graph": graph.clone(),
            "graph_state": graph["graph_state"].clone(),
            "graph_generation": graph["graph_generation"].clone(),
            "graph_binding_digest": graph["graph_binding_digest"].clone(),
            "graph_contract_version": graph["graph_contract_version"].clone(),
            "graph_digest": graph["graph_digest"].clone(),
            "member_count": graph["member_count"].clone(),
            "root_count": graph["root_count"].clone(),
            "graph_blocking_errors": graph["blocking_errors"].clone(),
            "graph_executable": true,
            "sources": sources,
            "blocking_errors": graph["blocking_errors"].clone(),
            "definition": row.3
        })))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
