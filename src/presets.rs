use serde_json::{Value, json};

use crate::error::MdmError;

pub(crate) fn expand(preset: Option<Value>) -> Result<Option<Value>, MdmError> {
    let Some(preset) = preset else {
        return Ok(None);
    };
    let name = preset
        .as_str()
        .or_else(|| preset.get("name").and_then(Value::as_str))
        .ok_or_else(|| MdmError::DefinitionInvalid("preset must be a built-in name".into()))?;
    if !matches!(name, "person" | "company" | "product") {
        return Err(MdmError::DefinitionInvalid(format!(
            "unknown built-in preset {name}"
        )));
    }
    Ok(Some(json!({"name": name, "version": 1})))
}
