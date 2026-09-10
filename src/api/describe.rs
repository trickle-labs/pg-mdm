use pgrx::prelude::*;
use pgrx::{Internal, JsonB};
use serde_json::Value;

use crate::catalog;
use crate::error::MdmError;

struct DescribeRequest {
    entity_name: String,
    format: String,
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
            "graph": capabilities.external_graph_refresh,
            "graph_executable": false,
            "sources": sources,
            "blocking_errors": if capabilities.external_graph_refresh.enabled { Vec::<Value>::new() } else { vec![serde_json::json!({"code":"MDM_PGT_CAPABILITY_DISABLED","message":"Graph V1 is disabled; definitions remain developmental"})] },
            "definition": row.3
        })))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
