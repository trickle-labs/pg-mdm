use std::collections::{BTreeMap, BTreeSet};

use pgrx::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::comparators;
use crate::definition::canonical::{canonical_definition, digest, hex, json_bytes};
use crate::definition::source::validate_source;
use crate::definition::{Entity, parse_entity};
use crate::error::MdmError;
use crate::graph_spec;
use crate::presets;
use crate::semantics;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PreparedDefinition {
    pub user_definition: Value,
    pub expanded_definition: Value,
    pub logical_candidate_plan: Value,
    pub semantic_manifest: Value,
    pub definition_digest: Vec<u8>,
    pub artifact_bytes: Vec<u8>,
    pub artifact_digest: Vec<u8>,
    pub sources: Vec<PreparedSource>,
    pub output_names: Vec<OutputName>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PreparedSource {
    pub name: String,
    pub relation_name: String,
    pub relation_oid: u32,
    pub key_contract: Value,
    pub identity_digest: Vec<u8>,
    pub binding_fingerprint: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct OutputName {
    pub name: String,
    pub kind: String,
}

fn identifier(name: &str, label: &str) -> Result<(), MdmError> {
    if name.is_empty() || name.bytes().any(|byte| byte == 0) {
        return Err(MdmError::DefinitionInvalid(format!(
            "{label} must be a non-empty identifier"
        )));
    }
    if name.len() > 63 {
        return Err(MdmError::DefinitionInvalid(format!(
            "{label} exceeds PostgreSQL's 63-byte identifier limit"
        )));
    }
    Ok(())
}

fn unique(names: impl IntoIterator<Item = String>, label: &str) -> Result<(), MdmError> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name.clone()) {
            return Err(MdmError::DefinitionInvalid(format!(
                "duplicate {label} {name}"
            )));
        }
    }
    Ok(())
}

fn map_as_object(value: &Value, label: &str) -> Result<(), MdmError> {
    if !value.is_object() {
        return Err(MdmError::DefinitionInvalid(format!(
            "{label} must be an object"
        )));
    }
    Ok(())
}

pub(crate) fn validate_entity_local(entity: &Entity) -> Result<(), MdmError> {
    identifier(&entity.name, "entity name")?;
    if entity.name.eq_ignore_ascii_case("mdm_id") {
        return Err(MdmError::DefinitionInvalid(
            "entity name is reserved".into(),
        ));
    }
    if entity.sources.is_empty() || entity.fields.is_empty() || entity.matches.is_empty() {
        return Err(MdmError::DefinitionInvalid(
            "sources, fields, and matches cannot be empty".into(),
        ));
    }
    unique(
        entity.sources.iter().map(|item| item.name.clone()),
        "source",
    )?;
    unique(entity.fields.iter().map(|item| item.name.clone()), "field")?;
    unique(entity.matches.iter().map(|item| item.name.clone()), "match")?;
    unique(
        entity.golden_values.iter().map(|item| item.field.clone()),
        "golden field",
    )?;

    for source in &entity.sources {
        identifier(&source.name, "source name")?;
        if !matches!(source.mode.as_str(), "tracked" | "soft_delete") {
            return Err(MdmError::DefinitionInvalid(format!(
                "source {} has unsupported mode",
                source.name
            )));
        }
        if source.relation.is_empty() || source.relation.bytes().any(|byte| byte == 0) {
            return Err(MdmError::DefinitionInvalid(format!(
                "source {} has an invalid relation",
                source.name
            )));
        }
        unique(source.source_id.clone(), "source key column")?;
        map_as_object(
            &Value::Object(source.fields.clone().into_iter().collect()),
            "source fields",
        )?;
        if !source.authority.values().all(Value::is_string) {
            return Err(MdmError::DefinitionInvalid(format!(
                "source {} authority values must be strings",
                source.name
            )));
        }
    }
    for field in &entity.fields {
        identifier(&field.name, "field name")?;
        if !matches!(
            field.logical_type.as_str(),
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
            return Err(MdmError::DefinitionInvalid(format!(
                "field {} has unsupported logical type {}",
                field.name, field.logical_type
            )));
        }
        if !crate::cleaners::is_supported_cleaner(&field.cleaner) {
            return Err(MdmError::DefinitionInvalid(format!(
                "field {} has unsupported cleaner {}",
                field.name, field.cleaner
            )));
        }
        let opts_val = serde_json::to_value(&field.cleaner_options).unwrap_or_else(|_| json!({}));
        crate::cleaners::validate_cleaner_options(&field.cleaner, 1, &opts_val)?;
        if field.cleaner == "date" && field.logical_type != "date" {
            return Err(MdmError::DefinitionInvalid(format!(
                "field {} with date cleaner requires date logical type",
                field.name
            )));
        }
        if matches!(
            field.cleaner.as_str(),
            "text" | "person_name" | "company_name" | "email" | "phone" | "tax_id"
        ) && !matches!(
            field.logical_type.as_str(),
            "text" | "character varying" | "character"
        ) {
            return Err(MdmError::DefinitionInvalid(format!(
                "field {} with {} cleaner requires text logical type",
                field.name, field.cleaner
            )));
        }
        if !matches!(field.display.as_str(), "full" | "masked" | "hashed") {
            return Err(MdmError::DefinitionInvalid(format!(
                "field {} has unsupported display policy",
                field.name
            )));
        }
    }
    let field_names: BTreeSet<&str> = entity
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    for rule in &entity.matches {
        identifier(&rule.name, "match name")?;
        if rule.fields.is_empty()
            || rule
                .fields
                .iter()
                .any(|name| !field_names.contains(name.as_str()))
        {
            return Err(MdmError::DefinitionInvalid(format!(
                "match {} references an unknown or empty field list",
                rule.name
            )));
        }
        if !matches!(
            rule.comparison.as_str(),
            "exact" | "fuzzy" | "normalized_levenshtein"
        ) {
            return Err(MdmError::DefinitionInvalid(format!(
                "match {} has unsupported comparison",
                rule.name
            )));
        }
        if (rule.comparison == "fuzzy" || rule.comparison == "normalized_levenshtein")
            && rule.threshold.is_none()
        {
            return Err(MdmError::DefinitionInvalid(format!(
                "match {} requires a fuzzy threshold",
                rule.name
            )));
        }
        comparators::validate_threshold(&rule.comparison, rule.threshold)
            .map_err(|error| MdmError::DefinitionInvalid(error.to_string()))?;
        if !matches!(rule.strength.as_str(), "identity" | "strong" | "supporting") {
            return Err(MdmError::DefinitionInvalid(format!(
                "match {} has unsupported strength",
                rule.name
            )));
        }
        if rule.evidence_group.is_empty() {
            return Err(MdmError::DefinitionInvalid(format!(
                "match {} has an empty evidence group",
                rule.name
            )));
        }
        if rule.strength == "supporting" && rule.candidate.is_some() {
            return Err(MdmError::DefinitionInvalid(format!(
                "supporting match {} cannot define a candidate channel",
                rule.name
            )));
        }
        if (rule.strength == "strong"
            || (rule.strength == "identity" && rule.comparison != "exact"))
            && rule.candidate.is_none()
        {
            return Err(MdmError::DefinitionInvalid(format!(
                "match {} requires a complete candidate channel",
                rule.name
            )));
        }
        if let Some(candidate) = &rule.candidate {
            let object = candidate.as_object().ok_or_else(|| {
                MdmError::DefinitionInvalid(format!(
                    "match {} candidate must be an object",
                    rule.name
                ))
            })?;
            if object.get("kind").and_then(Value::as_str).is_none() {
                return Err(MdmError::DefinitionInvalid(format!(
                    "match {} candidate.kind is required",
                    rule.name
                )));
            }
        }
    }
    crate::candidate::CandidatePlan::from_entity(entity)?;
    let mut lineage = BTreeMap::<String, String>::new();
    for rule in &entity.matches {
        for field in &rule.fields {
            if let Some(previous) = lineage.insert(field.clone(), rule.evidence_group.clone())
                && previous != rule.evidence_group
            {
                return Err(MdmError::DefinitionInvalid(format!(
                    "field {field} has evidence lineage in multiple groups"
                )));
            }
        }
    }
    let system_columns = [
        "mdm_id",
        "member_count",
        "has_review",
        "last_change_revision",
        "tableoid",
        "xmin",
        "cmin",
        "xmax",
        "cmax",
        "ctid",
    ];
    for golden in &entity.golden_values {
        if !field_names.contains(golden.field.as_str()) {
            return Err(MdmError::DefinitionInvalid(format!(
                "golden value references unknown field {}",
                golden.field
            )));
        }
        if system_columns
            .iter()
            .any(|name| golden.field.eq_ignore_ascii_case(name))
        {
            return Err(MdmError::DefinitionInvalid(format!(
                "golden field {} is reserved",
                golden.field
            )));
        }
        if !matches!(
            golden.policy.as_str(),
            "first_non_null" | "latest" | "most_common" | "prefer_source"
        ) {
            return Err(MdmError::DefinitionInvalid(format!(
                "golden value {} has unsupported policy",
                golden.field
            )));
        }
        if golden.policy == "prefer_source" && golden.sources.as_ref().is_none_or(Vec::is_empty) {
            return Err(MdmError::DefinitionInvalid(format!(
                "prefer_source for {} requires source priorities",
                golden.field
            )));
        }
    }
    semantics::validate_limits(&entity.limits)?;
    Ok(())
}

fn output_names(entity_name: &str) -> Result<Vec<OutputName>, MdmError> {
    let mut names = Vec::new();
    let entity_name = entity_name.to_lowercase();
    for (suffix, kind) in [
        ("", "entity"),
        ("_members", "members"),
        ("_review", "review"),
    ] {
        let name = format!("{entity_name}{suffix}");
        identifier(&name, "output name")?;
        names.push(OutputName {
            name,
            kind: kind.into(),
        });
    }
    Ok(names)
}

fn semantic_manifest() -> Value {
    let mut manifest = json!({
        "format_version": 1,
        "engine_version": 1,
        "source_key_encoding": 2,
        "canonical_encoding_version": 1,
        "cleaners": {
            "company_name": 1,
            "date": 1,
            "email": 1,
            "none": 1,
            "person_name": 1,
            "phone": 1,
            "tax_id": 1,
            "text": 1
        },
        "comparators": {"exact": 1, "normalized_levenshtein": 1, "fuzzy": 1},
        "pair_decision_policy": 1,
        "clustering_policy": 1,
        "stable_id_policy": 1,
        "golden_policies": {"first_non_null": 1, "latest": 1, "most_common": 1, "prefer_source": 1},
        "required_pg_trickle_capability": {
            "name": "external_graph_refresh",
            "major": 1,
            "minimum_minor": 0
        }
    });
    if let (Some(manifest), Some(candidate)) = (
        manifest.as_object_mut(),
        semantics::semantic_manifest().get("candidate"),
    ) {
        manifest.insert("candidate".into(), candidate.clone());
    }
    if let (Some(manifest), Some(clustering)) = (
        manifest.as_object_mut(),
        semantics::semantic_manifest().get("clustering"),
    ) {
        manifest.insert("clustering".into(), clustering.clone());
    }
    manifest
}

fn role_checks(entity: &Entity) -> Result<(), MdmError> {
    let current = Spi::get_one::<String>("SELECT current_user::text")
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Unauthorized("current role is unavailable".into()))?;
    if entity
        .execution_role
        .as_deref()
        .is_some_and(|role| role != current)
    {
        return Err(MdmError::Unauthorized(
            "execution_role must equal current_user".into(),
        ));
    }
    let row = Spi::connect(|client| {
        let table = client.select(
            "SELECT r.oid, r.rolsuper, r.rolbypassrls, EXISTS (SELECT 1 FROM pg_catalog.pg_extension e WHERE e.extname = 'pg_mdm' AND e.extowner = r.oid) FROM pg_catalog.pg_roles r WHERE r.rolname = current_user",
            Some(1),
            &[],
        ).map_err(|error| MdmError::Spi(error.to_string()))?;
        if table.is_empty() {
            return Err(MdmError::Unauthorized("current role does not exist".into()));
        }
        let row = table.first();
        Ok::<_, MdmError>((
            row.get::<bool>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
            row.get::<bool>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
            row.get::<bool>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
        ))
    })?;
    if row.0 || row.1 || row.2 {
        return Err(MdmError::Unauthorized(
            "superuser, BYPASSRLS, and the extension owner cannot create definitions".into(),
        ));
    }
    Ok(())
}

fn require_utf8_database() -> Result<(), MdmError> {
    let utf8 = Spi::get_one::<bool>(
        "SELECT pg_catalog.pg_encoding_to_char(encoding) = 'UTF8' FROM pg_catalog.pg_database WHERE datname = pg_catalog.current_database()",
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .ok_or_else(|| MdmError::DefinitionInvalid("current database is unavailable".into()))?;
    if !utf8 {
        return Err(MdmError::DefinitionInvalid(
            "pg_mdm definitions require a UTF-8 database".into(),
        ));
    }
    Ok(())
}

pub(crate) fn prepare(value: Value) -> Result<PreparedDefinition, MdmError> {
    require_utf8_database()?;
    let user_definition = canonical_definition(value);
    let mut entity = parse_entity(user_definition.clone()).map_err(MdmError::DefinitionInvalid)?;
    validate_entity_local(&entity)?;
    role_checks(&entity)?;
    if entity.execution_role.is_none() {
        entity.execution_role = Some(
            Spi::get_one::<String>("SELECT current_user::text")
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Unauthorized("current role is unavailable".into()))?,
        );
    }
    let candidate_limits = semantics::expand_limits(&mut entity.limits)?;
    let warning_block_records = semantics::validate_limit_value(
        "warning_block_records",
        &entity.limits["warning_block_records"],
    )?;
    entity.preset = presets::expand(entity.preset.clone())?;
    let mut sources = Vec::with_capacity(entity.sources.len());
    for index in 0..entity.sources.len() {
        let source = entity.sources[index].clone();
        let validated = validate_source(&source, &entity)?;
        entity.sources[index].relation = validated.relation_name.clone();
        sources.push(validated);
    }
    let expanded_definition =
        canonical_definition(serde_json::to_value(&entity).expect("definition is serializable"));
    if let Some(sources_for_latest) = entity
        .golden_values
        .iter()
        .find(|golden| golden.policy == "latest")
        && entity
            .sources
            .iter()
            .any(|source| source.row_changed_at.is_none())
    {
        return Err(MdmError::DefinitionInvalid(format!(
            "latest golden field {} requires row_changed_at on every source",
            sources_for_latest.field
        )));
    }
    for golden in &entity.golden_values {
        if let Some(priority) = &golden.sources
            && priority
                .iter()
                .any(|source| !entity.sources.iter().any(|item| item.name == *source))
        {
            return Err(MdmError::DefinitionInvalid(format!(
                "golden value {} references an unknown source",
                golden.field
            )));
        }
    }
    let logical_candidate_plan = canonical_definition(
        crate::candidate::CandidatePlan::from_entity(&entity)?
            .to_json_with_warning(&candidate_limits, warning_block_records),
    );
    let semantic_manifest = canonical_definition(semantic_manifest());
    let definition_digest = digest(
        "pg_mdm/definition/v1",
        &[
            &json_bytes(&expanded_definition),
            &json_bytes(&logical_candidate_plan),
            &json_bytes(&semantic_manifest),
        ],
    );
    let graph = graph_spec::compile(&entity);
    let artifact_bytes = json_bytes(&canonical_definition(graph));
    let schemas = json_bytes(
        &json!({"terminal_relations": ["records", "normalized", "blocks", "pairs", "evidence", "golden"]}),
    );
    let compiler = graph_spec::COMPILER_VERSION.to_be_bytes();
    let format = graph_spec::ARTIFACT_FORMAT_VERSION.to_be_bytes();
    let artifact_digest = digest(
        "pg_mdm/artifact/v1",
        &[
            &definition_digest,
            &compiler,
            &format,
            &artifact_bytes,
            &schemas,
        ],
    );
    Ok(PreparedDefinition {
        user_definition,
        expanded_definition,
        logical_candidate_plan,
        semantic_manifest,
        definition_digest,
        artifact_bytes,
        artifact_digest,
        sources: sources
            .into_iter()
            .map(|source| PreparedSource {
                name: source.name,
                relation_name: source.relation_name,
                relation_oid: source.relation_oid.to_u32(),
                key_contract: source.key_contract,
                identity_digest: source.identity_digest,
                binding_fingerprint: source.binding_fingerprint,
            })
            .collect(),
        output_names: output_names(&entity.name)?,
    })
}

pub(crate) fn digest_hex(bytes: &[u8]) -> String {
    hex(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(groups: (&str, &str)) -> Entity {
        parse_entity(json!({
            "name": "customer",
            "sources": [{
                "name": "crm",
                "relation": "public.crm",
                "source_id": ["id"],
                "mode": "tracked",
                "fields": {"email": "email"},
                "row_changed_at": null,
                "soft_delete_when": null,
                "authority": {}
            }],
            "fields": [{
                "name": "email",
                "type": "text",
                "cleaner": "email",
                "cleaner_options": {},
                "display": "masked"
            }],
            "matches": [{
                "name": "match_a",
                "fields": ["email"],
                "comparison": "exact",
                "strength": "identity",
                "evidence_group": groups.0,
                "threshold": null,
                "candidate": null
            }, {
                "name": "match_b",
                "fields": ["email"],
                "comparison": "exact",
                "strength": "identity",
                "evidence_group": groups.1,
                "threshold": null,
                "candidate": null
            }],
            "golden_values": [],
            "preset": null,
            "limits": {},
            "execution_role": null
        }))
        .expect("test definition parses")
    }

    #[test]
    fn rejects_lineage_split_across_evidence_groups() {
        assert!(validate_entity_local(&definition(("email", "name"))).is_err());
    }

    #[test]
    fn accepts_repeated_lineage_in_one_group() {
        assert!(validate_entity_local(&definition(("email", "email"))).is_ok());
    }

    #[test]
    fn rejects_system_column_as_golden_field() {
        let mut entity = definition(("email", "email"));
        entity.fields[0].name = "cmax".into();
        for rule in &mut entity.matches {
            rule.fields = vec!["cmax".into()];
        }
        entity.golden_values.push(crate::definition::GoldenValue {
            field: "cmax".into(),
            policy: "first_non_null".into(),
            sources: None,
        });
        assert_eq!(
            validate_entity_local(&entity),
            Err(MdmError::DefinitionInvalid(
                "golden field cmax is reserved".into()
            ))
        );
    }

    #[test]
    fn semantic_manifest_pins_requirements_without_live_capability_state() {
        let manifest = semantic_manifest();
        assert_eq!(
            manifest["required_pg_trickle_capability"],
            json!({"name": "external_graph_refresh", "major": 1, "minimum_minor": 0})
        );
        assert!(manifest.get("capabilities").is_none());
    }
}
