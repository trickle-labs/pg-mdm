use pgrx::prelude::*;
use pgrx::spi::SpiClient;
use pgrx::{Internal, JsonB};
use serde_json::{Map, Value, json};

use crate::catalog;
use crate::error::MdmError;

struct ExplainRequest {
    entity_name: String,
    subject: Value,
    publication_revision: Option<i64>,
    max_facts: i32,
}

fn validate_subject(subject: &Value) -> Result<&Map<String, Value>, MdmError> {
    let object = subject
        .as_object()
        .ok_or_else(|| MdmError::ExplanationInvalid("subject must be an object".into()))?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| MdmError::ExplanationInvalid("subject.kind is required".into()))?;
    if !matches!(
        kind,
        "source_record" | "pair" | "mdm_id" | "golden" | "review" | "publication"
    ) {
        return Err(MdmError::ExplanationInvalid(format!(
            "unknown subject kind {kind}"
        )));
    }
    let allowed = match kind {
        "source_record" => &["kind", "id"][..],
        "pair" => &["kind", "left_id", "right_id"][..],
        "mdm_id" => &["kind", "id"][..],
        "golden" => &["kind", "mdm_id", "field"][..],
        "review" => &["kind", "id"][..],
        "publication" => &["kind", "revision"][..],
        _ => &[][..],
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(MdmError::ExplanationInvalid(
            "subject contains an unknown key".into(),
        ));
    }
    let required = match kind {
        "source_record" | "mdm_id" | "review" => &["id"][..],
        "pair" => &["left_id", "right_id"][..],
        "golden" => &["mdm_id", "field"][..],
        "publication" => &["revision"][..],
        _ => &[][..],
    };
    if required.iter().any(|key| !object.contains_key(*key)) {
        return Err(MdmError::ExplanationInvalid(
            "subject is missing a required key".into(),
        ));
    }
    Ok(object)
}

fn identity_state(
    client: &SpiClient<'_>,
    entity_id: &str,
    mdm_id: &str,
) -> Result<Value, MdmError> {
    let rows = client
        .select(
            "SELECT status FROM mdm_internal.identity_registry WHERE entity_id = $1::pg_catalog.uuid AND mdm_id = $2::pg_catalog.uuid",
            Some(1),
            &[entity_id.to_owned().into(), mdm_id.to_owned().into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let status = rows
        .first()
        .get::<String>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    Ok(json!({"scope": "current", "status": status}))
}

fn pair_facts(
    client: &SpiClient<'_>,
    entity_id: &str,
    subject: &Map<String, Value>,
    publication_revision: Option<i64>,
    max_facts: i32,
) -> Result<(Vec<Value>, bool), MdmError> {
    let left_id = subject
        .get("left_id")
        .and_then(Value::as_str)
        .ok_or_else(|| MdmError::ExplanationInvalid("pair.left_id must be a UUID string".into()))?;
    let right_id = subject
        .get("right_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            MdmError::ExplanationInvalid("pair.right_id must be a UUID string".into())
        })?;
    let rows = client
        .select(
            "SELECT publication_revision, fact_kind, fact->>'reason_code', fact->'evidence_groups' FROM mdm_internal.resolution_facts WHERE entity_id = $1::pg_catalog.uuid AND subject_kind = 'pair' AND subject_key IN (pg_catalog.convert_to(($2::pg_catalog.uuid)::text || ':' || ($3::pg_catalog.uuid)::text, 'UTF8'), pg_catalog.convert_to(($3::pg_catalog.uuid)::text || ':' || ($2::pg_catalog.uuid)::text, 'UTF8')) AND ($4::bigint IS NULL OR publication_revision = $4) ORDER BY publication_revision DESC, fact_number LIMIT $5",
            None,
            &[
                entity_id.to_owned().into(),
                left_id.to_owned().into(),
                right_id.to_owned().into(),
                publication_revision.into(),
                (max_facts as i64 + 1).into(),
            ],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let mut facts = rows
        .into_iter()
        .map(|row| {
            let revision = row
                .get::<i64>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("resolution fact revision is NULL".into()))?;
            let outcome = row
                .get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("resolution fact outcome is NULL".into()))?;
            let reason_code = row
                .get::<String>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let evidence_groups = row
                .get::<JsonB>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .map(|value| value.0)
                .unwrap_or_else(|| json!([]));
            Ok(json!({
                "publication_revision": revision,
                "outcome": outcome,
                "reason_code": reason_code,
                "evidence_groups": evidence_groups
            }))
        })
        .collect::<Result<Vec<_>, MdmError>>()?;
    let truncated = facts.len() > max_facts as usize;
    facts.truncate(max_facts as usize);
    Ok((facts, truncated))
}

#[pg_extern(
    name = "explain",
    requires = [explain_entity],
    sql = "CREATE FUNCTION mdm.explain(entity_name text, subject jsonb, publication_revision bigint DEFAULT NULL, max_facts integer DEFAULT 100) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'explain_wrapper';"
)]
pub(crate) fn explain(
    entity_name: String,
    subject: JsonB,
    publication_revision: Option<i64>,
    max_facts: default!(i32, 100),
) -> JsonB {
    catalog::call_helper(
        "explain_entity",
        ExplainRequest {
            entity_name,
            subject: subject.0,
            publication_revision,
            max_facts,
        },
    )
    .unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "explain_entity",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.explain_entity(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'explain_entity_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn explain_entity(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only mdm.explain constructs ExplainRequest.
        let request = unsafe { request.get::<ExplainRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("explain request is required".into()))?;
        if !(1..=500).contains(&request.max_facts) {
            return Err(MdmError::ExplanationInvalid(
                "max_facts must be between 1 and 500".into(),
            ));
        }
        let subject = validate_subject(&request.subject)?;
        let helper_owner = catalog::validate_helper_owner()?;
        let (_, selected) = catalog::validate_caller(&helper_owner)?;
        Spi::connect(|client| {
            let entity = client.select(
                "SELECT e.entity_id::text, e.execution_role_name FROM mdm_internal.entities e LEFT JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id WHERE e.entity_name = $1::pg_catalog.name AND e.execution_role_name = $2 AND b.role_oid = (SELECT oid FROM pg_catalog.pg_roles WHERE rolname = pg_catalog.current_user)",
                Some(1), &[request.entity_name.clone().into(), selected.name.clone().into()]
            ).map_err(|e| MdmError::Spi(e.to_string()))?;
            if entity.is_empty() {
                return Err(MdmError::Unauthorized(
                    "entity is bound to another execution role or does not exist".into(),
                ));
            }
            let entity_id = entity
                .first()
                .get::<String>(1)
                .map_err(|e| MdmError::Spi(e.to_string()))?
                .ok_or_else(|| MdmError::Spi("entity ID is NULL".into()))?;
            let range = client.select(
                "SELECT min(publication_revision), max(publication_revision) FROM mdm_internal.resolution_facts WHERE entity_id = $1::pg_catalog.uuid",
                Some(1), &[entity_id.clone().into()]
            ).map_err(|e| MdmError::Spi(e.to_string()))?;
            let range_row = range.first();
            let earliest = range_row
                .get::<i64>(1)
                .map_err(|e| MdmError::Spi(e.to_string()))?;
            let latest = range_row
                .get::<i64>(2)
                .map_err(|e| MdmError::Spi(e.to_string()))?;
            if let Some(revision) = request.publication_revision
                && (earliest.is_none()
                    || latest.is_none()
                    || earliest.is_some_and(|value| revision < value)
                    || latest.is_some_and(|value| revision > value))
            {
                return Err(MdmError::ExplanationNotRetained(revision.to_string()));
            }
            let kind = subject
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| MdmError::ExplanationInvalid("subject.kind is required".into()))?;
            let subject_key = subject
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| subject.get("mdm_id").and_then(Value::as_str))
                .map(str::to_owned)
                .or_else(|| {
                    subject
                        .get("revision")
                        .and_then(Value::as_i64)
                        .map(|value| value.to_string())
                });
            let (facts, truncated) = if kind == "pair" {
                pair_facts(
                    client,
                    &entity_id,
                    subject,
                    request.publication_revision,
                    request.max_facts,
                )?
            } else if let Some(key) = subject_key {
                let facts = client.select(
                    "SELECT fact FROM mdm_internal.resolution_facts WHERE entity_id = $1::pg_catalog.uuid AND subject_kind = $2 AND subject_key = pg_catalog.convert_to($3, 'UTF8') ORDER BY publication_revision DESC, fact_number LIMIT $4",
                    Some(1), &[entity_id.clone().into(), kind.into(), key.into(), (request.max_facts as i64 + 1).into()]
                ).map_err(|e| MdmError::Spi(e.to_string()))?.map(|row| row.get::<JsonB>(1).map(|value| value.map(|v| v.0)).map_err(|e| MdmError::Spi(e.to_string()))).collect::<Result<Vec<_>, _>>()?.into_iter().flatten().collect::<Vec<_>>();
                let truncated = facts.len() > request.max_facts as usize;
                (
                    facts.into_iter().take(request.max_facts as usize).collect(),
                    truncated,
                )
            } else {
                (Vec::new(), false)
            };
            let identity = if kind == "mdm_id" {
                let mdm_id = subject.get("id").and_then(Value::as_str).ok_or_else(|| {
                    MdmError::ExplanationInvalid("mdm_id.id must be a UUID string".into())
                })?;
                identity_state(client, &entity_id, mdm_id)?
            } else {
                Value::Null
            };
            Ok(JsonB(json!({
                "facts": facts,
                "truncated": truncated,
                "identity": identity,
                "retained_revision_range": {"earliest": earliest, "latest": latest}
            })))
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
