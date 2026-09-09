use pgrx::prelude::*;
use pgrx::{JsonB, PgRelation};
use serde_json::{Value, json};

use crate::definition::canonical::canonical_definition;
use crate::definition::validate::validate_entity_local;
use crate::error::MdmError;

fn text(value: &str, label: &str) -> Result<String, MdmError> {
    if value.is_empty() || value.bytes().any(|byte| byte == 0) {
        return Err(MdmError::DefinitionInvalid(format!(
            "{label} must not be empty"
        )));
    }
    Ok(value.to_owned())
}

fn object(value: JsonB, label: &str) -> Result<Value, MdmError> {
    if !value.0.is_object() {
        return Err(MdmError::DefinitionInvalid(format!(
            "{label} must be a JSON object"
        )));
    }
    Ok(value.0)
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[allow(clippy::too_many_arguments)]
#[pg_extern(
    name = "source",
    sql = "CREATE FUNCTION mdm.source(name text, relation regclass, source_id text[], mode text, fields jsonb, row_changed_at text DEFAULT NULL, soft_delete_when jsonb DEFAULT NULL, authority jsonb DEFAULT '{}'::jsonb) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'source_wrapper';"
)]
pub(crate) fn source(
    name: String,
    relation: PgRelation,
    source_id: Vec<String>,
    mode: String,
    fields: JsonB,
    row_changed_at: Option<String>,
    soft_delete_when: Option<JsonB>,
    authority: JsonB,
) -> JsonB {
    let result = (|| {
        let name = text(&name, "source name")?;
        if source_id.is_empty() || source_id.iter().any(|value| value.is_empty()) {
            return Err(MdmError::DefinitionInvalid(
                "source_id must contain at least one column".into(),
            ));
        }
        if !matches!(mode.as_str(), "tracked" | "soft_delete") {
            return Err(MdmError::DefinitionInvalid(
                "mode must be tracked or soft_delete".into(),
            ));
        }
        if mode == "soft_delete" && soft_delete_when.is_none() {
            return Err(MdmError::DefinitionInvalid(
                "soft_delete requires soft_delete_when".into(),
            ));
        }
        if mode == "tracked" && soft_delete_when.is_some() {
            return Err(MdmError::DefinitionInvalid(
                "tracked cannot define soft_delete_when".into(),
            ));
        }
        if let Some(predicate) = &soft_delete_when {
            let predicate = predicate.0.as_object().ok_or_else(|| {
                MdmError::DefinitionInvalid("soft_delete_when must be an object".into())
            })?;
            if predicate.get("column").and_then(Value::as_str).is_none()
                || !matches!(
                    predicate.get("kind").and_then(Value::as_str),
                    Some("is_true" | "is_not_null")
                )
            {
                return Err(MdmError::DefinitionInvalid(
                    "soft_delete_when must contain column and kind is_true/is_not_null".into(),
                ));
            }
        }
        let fields = object(fields, "fields")?;
        let authority = object(authority, "authority")?;
        let relation = format!(
            "{}.{}",
            quote_identifier(relation.namespace()),
            quote_identifier(relation.name())
        );
        Ok(JsonB(canonical_definition(json!({
            "name": name,
            "relation": relation,
            "source_id": source_id,
            "mode": mode,
            "fields": fields,
            "row_changed_at": row_changed_at,
            "soft_delete_when": soft_delete_when.map(|value| value.0),
            "authority": authority
        }))))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[allow(clippy::too_many_arguments)]
#[pg_extern(
    name = "field",
    sql = "CREATE FUNCTION mdm.field(name text, type text, cleaner text, cleaner_options jsonb DEFAULT '{}'::jsonb, display text DEFAULT 'masked') RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'field_wrapper';"
)]
pub(crate) fn field(
    name: String,
    logical_type: String,
    cleaner: String,
    cleaner_options: JsonB,
    display: String,
) -> JsonB {
    let result = (|| {
        if !matches!(display.as_str(), "full" | "masked" | "hashed") {
            return Err(MdmError::DefinitionInvalid(
                "display must be full, masked, or hashed".into(),
            ));
        }
        if !matches!(
            cleaner.as_str(),
            "company_name" | "email" | "tax_id" | "none"
        ) {
            return Err(MdmError::DefinitionInvalid("unsupported cleaner".into()));
        }
        if !matches!(
            logical_type.as_str(),
            "text"
                | "character varying"
                | "character"
                | "date"
                | "timestamp"
                | "timestamp without time zone"
                | "timestamp with time zone"
                | "boolean"
                | "smallint"
                | "integer"
                | "bigint"
                | "numeric"
        ) {
            return Err(MdmError::DefinitionInvalid("unsupported field type".into()));
        }
        Ok(JsonB(canonical_definition(json!({
            "name": text(&name, "field name")?,
            "type": text(&logical_type, "field type")?,
            "cleaner": text(&cleaner, "cleaner")?,
            "cleaner_options": object(cleaner_options, "cleaner_options")?,
            "display": display
        }))))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "match",
    sql = "CREATE FUNCTION mdm.match(name text, fields text[], comparison text, strength text, evidence_group text, threshold integer DEFAULT NULL, candidate jsonb DEFAULT NULL) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'match_wrapper';"
)]
pub(crate) fn match_rule(
    name: String,
    fields: Vec<String>,
    comparison: String,
    strength: String,
    evidence_group: String,
    threshold: Option<i32>,
    candidate: Option<JsonB>,
) -> JsonB {
    let result = (|| {
        if fields.is_empty() || fields.iter().any(String::is_empty) {
            return Err(MdmError::DefinitionInvalid(
                "match fields must not be empty".into(),
            ));
        }
        if !matches!(
            comparison.as_str(),
            "exact" | "fuzzy" | "normalized_levenshtein"
        ) {
            return Err(MdmError::DefinitionInvalid("unsupported comparison".into()));
        }
        if !matches!(strength.as_str(), "identity" | "strong" | "supporting") {
            return Err(MdmError::DefinitionInvalid("unsupported strength".into()));
        }
        if evidence_group.is_empty() {
            return Err(MdmError::DefinitionInvalid(
                "evidence_group must not be empty".into(),
            ));
        }
        if threshold.is_some_and(|value| value < 0) {
            return Err(MdmError::DefinitionInvalid(
                "threshold must not be negative".into(),
            ));
        }
        if strength == "supporting" && candidate.is_some() {
            return Err(MdmError::DefinitionInvalid(
                "supporting matches cannot define candidates".into(),
            ));
        }
        if (strength == "strong" || (strength == "identity" && comparison != "exact"))
            && candidate.is_none()
        {
            return Err(MdmError::DefinitionInvalid(
                "this match requires a candidate channel".into(),
            ));
        }
        if let Some(candidate) = &candidate
            && candidate.0.get("kind").and_then(Value::as_str).is_none()
        {
            return Err(MdmError::DefinitionInvalid(
                "candidate.kind is required".into(),
            ));
        }
        Ok(JsonB(canonical_definition(json!({
            "name": text(&name, "match name")?,
            "fields": fields,
            "comparison": text(&comparison, "comparison")?,
            "strength": text(&strength, "strength")?,
            "evidence_group": text(&evidence_group, "evidence_group")?,
            "threshold": threshold,
            "candidate": candidate.map(|value| value.0)
        }))))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "golden_value",
    sql = "CREATE FUNCTION mdm.golden_value(field text, policy text, sources text[] DEFAULT NULL) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'golden_value_wrapper';"
)]
pub(crate) fn golden_value(field: String, policy: String, sources: Option<Vec<String>>) -> JsonB {
    let result = (|| {
        if !matches!(
            policy.as_str(),
            "first_non_null" | "latest" | "most_common" | "prefer_source"
        ) {
            return Err(MdmError::DefinitionInvalid(
                "unsupported golden policy".into(),
            ));
        }
        if policy == "prefer_source" && sources.as_ref().is_none_or(Vec::is_empty) {
            return Err(MdmError::DefinitionInvalid(
                "prefer_source requires source priorities".into(),
            ));
        }
        Ok(JsonB(canonical_definition(json!({
            "field": text(&field, "golden field")?,
            "policy": text(&policy, "golden policy")?,
            "sources": sources
        }))))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[allow(clippy::too_many_arguments)]
#[pg_extern(
    name = "entity",
    sql = "CREATE FUNCTION mdm.entity(name text, sources jsonb[], fields jsonb[], matches jsonb[], golden_values jsonb[], preset text DEFAULT NULL, limits jsonb DEFAULT '{}'::jsonb, execution_role text DEFAULT NULL) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'entity_wrapper';"
)]
pub(crate) fn entity(
    name: String,
    sources: Vec<JsonB>,
    fields: Vec<JsonB>,
    matches: Vec<JsonB>,
    golden_values: Vec<JsonB>,
    preset: Option<String>,
    limits: JsonB,
    execution_role: Option<String>,
) -> JsonB {
    let result = (|| {
        let array =
            |values: Vec<JsonB>| Value::Array(values.into_iter().map(|value| value.0).collect());
        let limits = object(limits, "limits")?;
        let value = canonical_definition(json!({
            "name": text(&name, "entity name")?,
            "sources": array(sources),
            "fields": array(fields),
            "matches": array(matches),
            "golden_values": array(golden_values),
            "preset": preset,
            "limits": limits,
            "execution_role": execution_role
        }));
        let entity =
            crate::definition::parse_entity(value.clone()).map_err(MdmError::DefinitionInvalid)?;
        validate_entity_local(&entity)?;
        Ok(JsonB(value))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
