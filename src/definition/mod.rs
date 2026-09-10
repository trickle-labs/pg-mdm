pub mod canonical;
pub mod source;
pub mod validate;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Source {
    pub name: String,
    pub relation: String,
    pub source_id: Vec<String>,
    pub mode: String,
    pub fields: BTreeMap<String, Value>,
    pub row_changed_at: Option<String>,
    pub soft_delete_when: Option<Value>,
    pub authority: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Field {
    pub name: String,
    #[serde(rename = "type")]
    pub logical_type: String,
    pub cleaner: String,
    pub cleaner_options: BTreeMap<String, Value>,
    pub display: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MatchRule {
    pub name: String,
    pub fields: Vec<String>,
    pub comparison: String,
    pub strength: String,
    pub evidence_group: String,
    pub threshold: Option<i32>,
    pub candidate: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GoldenValue {
    pub field: String,
    pub policy: String,
    pub sources: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Entity {
    pub name: String,
    pub sources: Vec<Source>,
    pub fields: Vec<Field>,
    pub matches: Vec<MatchRule>,
    pub golden_values: Vec<GoldenValue>,
    pub preset: Option<Value>,
    pub limits: BTreeMap<String, Value>,
    pub execution_role: Option<String>,
}

pub fn parse_entity(value: Value) -> Result<Entity, String> {
    serde_json::from_value(value).map_err(|error| format!("invalid entity definition: {error}"))
}
