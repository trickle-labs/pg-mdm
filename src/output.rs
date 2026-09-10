use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::error::MdmError;

const SYSTEM_COLUMNS: [&str; 10] = [
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputField {
    pub name: String,
    pub ordinal: u16,
    pub type_name: String,
}

pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('\"', "\"\""))
}

pub fn output_table_ddl(
    entity_name: &str,
    fields: &[OutputField],
) -> Result<Vec<String>, MdmError> {
    if entity_name.is_empty()
        || entity_name.len() > 63
        || entity_name.len() + "_members".len() > 63
        || entity_name.len() + "_review".len() > 63
        || entity_name.contains('\0')
    {
        return Err(MdmError::OutputInvalid(
            "entity output name is invalid".into(),
        ));
    }
    validate_output_fields(fields)?;
    if fields.iter().any(|field| {
        field
            .type_name
            .chars()
            .any(|character| !(character.is_ascii_alphanumeric() || "_ .\"[]".contains(character)))
            || field.type_name.contains("--")
    }) {
        return Err(MdmError::OutputInvalid(
            "field type name is not a portable PostgreSQL type name".into(),
        ));
    }
    let golden = fields
        .iter()
        .map(|field| format!("    {} {}", quote_identifier(&field.name), field.type_name))
        .collect::<Vec<_>>();
    let entity = quote_identifier(entity_name);
    let members = quote_identifier(&format!("{entity_name}_members"));
    let review = quote_identifier(&format!("{entity_name}_review"));
    Ok(vec![
        format!(
            "CREATE TABLE mdm_out.{entity} (\n    mdm_id uuid PRIMARY KEY,\n{},\n    member_count bigint NOT NULL,\n    has_review boolean NOT NULL,\n    last_change_revision bigint NOT NULL\n);",
            golden.join(",\n")
        ),
        format!(
            "CREATE TABLE mdm_out.{members} (\n    source_record_id uuid PRIMARY KEY,\n    source_name name NOT NULL,\n    source_id jsonb NOT NULL,\n    mdm_id uuid NOT NULL,\n    active boolean NOT NULL,\n    first_membership_revision bigint NOT NULL,\n    last_membership_revision bigint NOT NULL,\n    membership_reason text NOT NULL,\n    last_change_revision bigint NOT NULL\n);"
        ),
        format!(
            "CREATE TABLE mdm_out.{review} (\n    review_id uuid PRIMARY KEY,\n    issue_key bytea NOT NULL,\n    occurrence integer NOT NULL,\n    status text NOT NULL,\n    severity text NOT NULL,\n    reason_code text NOT NULL,\n    subjects jsonb NOT NULL,\n    masked_summary jsonb NOT NULL,\n    opened_revision bigint NOT NULL,\n    resolved_revision bigint,\n    last_change_revision bigint NOT NULL,\n    concurrency_version bigint NOT NULL\n);"
        ),
    ])
}

pub fn validate_output_fields(fields: &[OutputField]) -> Result<(), MdmError> {
    let mut names = std::collections::BTreeSet::new();
    for (index, field) in fields.iter().enumerate() {
        if field.name.is_empty() || field.name.len() > 63 {
            return Err(MdmError::OutputInvalid(format!(
                "field name {} must be 1 through 63 UTF-8 bytes",
                field.name
            )));
        }
        if SYSTEM_COLUMNS
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(&field.name))
            || !names.insert(field.name.to_ascii_lowercase())
        {
            return Err(MdmError::OutputInvalid(format!(
                "field name {} collides with an output column",
                field.name
            )));
        }
        if field.ordinal as usize != index + 1 {
            return Err(MdmError::OutputInvalid(format!(
                "field {} has non-contiguous ordinal {}",
                field.name, field.ordinal
            )));
        }
        if field.type_name.trim().is_empty() {
            return Err(MdmError::OutputInvalid(format!(
                "field {} has no PostgreSQL type",
                field.name
            )));
        }
    }
    Ok(())
}

pub fn validate_append_only(
    previous: &[OutputField],
    desired: &[OutputField],
) -> Result<(), MdmError> {
    validate_output_fields(previous)?;
    validate_output_fields(desired)?;
    if desired.len() < previous.len() || previous.iter().zip(desired).any(|(old, new)| old != new) {
        return Err(MdmError::OutputInvalid(
            "published golden fields are append-only and cannot be removed, renamed, reordered, or retyped"
                .into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowDiff {
    pub inserts: Vec<(Vec<u8>, Value)>,
    pub updates: Vec<(Vec<u8>, Value)>,
    pub deletes: Vec<Vec<u8>>,
}

pub fn diff_rows(old: &BTreeMap<Vec<u8>, Value>, new: &BTreeMap<Vec<u8>, Value>) -> RowDiff {
    let mut diff = RowDiff {
        inserts: Vec::new(),
        updates: Vec::new(),
        deletes: Vec::new(),
    };
    for (key, value) in new {
        match old.get(key) {
            None => diff.inserts.push((key.clone(), value.clone())),
            Some(previous) if previous != value => diff.updates.push((key.clone(), value.clone())),
            Some(_) => {}
        }
    }
    for key in old.keys().filter(|key| !new.contains_key(*key)) {
        diff.deletes.push(key.clone());
    }
    diff
}

pub fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        Value::Object(values) => {
            let mut sorted = Map::new();
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in entries {
                sorted.insert(key.clone(), canonical_json(value));
            }
            Value::Object(sorted)
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn field(name: &str, ordinal: u16, type_name: &str) -> OutputField {
        OutputField {
            name: name.into(),
            ordinal,
            type_name: type_name.into(),
        }
    }

    #[test]
    fn output_fields_are_append_only() {
        let old = vec![field("name", 1, "text")];
        assert!(
            validate_append_only(&old, &[field("name", 1, "text"), field("email", 2, "text")])
                .is_ok()
        );
        assert!(validate_append_only(&old, &[field("email", 1, "text")]).is_err());
    }

    #[test]
    fn output_ddl_quotes_names_and_preserves_field_order() {
        let ddl = output_table_ddl("customer", &[field("full name", 1, "text")]).unwrap();
        assert!(ddl[0].contains("\"full name\" text"));
        assert!(ddl[1].contains("customer_members"));
        assert!(ddl[2].contains("customer_review"));
    }

    #[test]
    fn row_diff_skips_unchanged_payloads() {
        let old = BTreeMap::from([(vec![1], json!({"v": 1})), (vec![2], json!({"v": 2}))]);
        let new = BTreeMap::from([(vec![1], json!({"v": 1})), (vec![3], json!({"v": 3}))]);
        let diff = diff_rows(&old, &new);
        assert_eq!(diff.inserts.len(), 1);
        assert_eq!(diff.updates.len(), 0);
        assert_eq!(diff.deletes, vec![vec![2]]);
    }

    #[test]
    fn canonical_json_sorts_object_keys() {
        assert_eq!(
            canonical_json(&json!({"b": 1, "a": 2})).to_string(),
            r#"{"a":2,"b":1}"#
        );
    }
}
