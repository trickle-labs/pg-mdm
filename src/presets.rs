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
    if let Some(version) = preset.get("version")
        && version.as_u64() != Some(1)
    {
        return Err(MdmError::DefinitionInvalid(format!(
            "unsupported version {version} for built-in preset {name}"
        )));
    }
    Ok(Some(json!({"name": name, "version": 1})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_bare_presets_and_preserves_supported_versions() {
        for name in ["person", "company", "product"] {
            let pinned = json!({"name": name, "version": 1});
            assert_eq!(expand(Some(json!(name))).unwrap(), Some(pinned.clone()));
            assert_eq!(expand(Some(pinned.clone())).unwrap(), Some(pinned));
        }
        for version in [json!(0), json!(2), json!(-1), json!("1"), json!(null)] {
            assert!(expand(Some(json!({"name": "person", "version": version}))).is_err());
        }
    }
}
