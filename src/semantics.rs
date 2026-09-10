use serde_json::{Value, json};

use crate::candidate::CandidateLimits;
use crate::error::MdmError;

pub const DEFAULT_MAX_BLOCK_RECORDS: usize = 10_000;
pub const DEFAULT_MAX_CANDIDATE_PAIRS: usize = 1_000_000;
pub const DEFAULT_WARNING_BLOCK_RECORDS: usize = 5_000;
pub const ABSOLUTE_MAX_BLOCK_RECORDS: usize = 1_000_000;
pub const ABSOLUTE_MAX_CANDIDATE_PAIRS: usize = 100_000_000;

pub fn candidate_limits() -> CandidateLimits {
    CandidateLimits {
        max_block_records: DEFAULT_MAX_BLOCK_RECORDS,
        max_candidate_pairs: DEFAULT_MAX_CANDIDATE_PAIRS,
    }
}

pub fn validate_limit_value(name: &str, value: &Value) -> Result<usize, MdmError> {
    let value = value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| {
            MdmError::DefinitionInvalid(format!("limit {name} must be a positive integer"))
        })?;
    if value == 0 {
        return Err(MdmError::DefinitionInvalid(format!(
            "limit {name} must be greater than zero"
        )));
    }
    let ceiling = match name {
        "max_block_records" | "warning_block_records" => ABSOLUTE_MAX_BLOCK_RECORDS,
        "max_candidate_pairs" => ABSOLUTE_MAX_CANDIDATE_PAIRS,
        _ => {
            return Err(MdmError::DefinitionInvalid(format!(
                "unsupported candidate limit {name}"
            )));
        }
    };
    if value > ceiling {
        return Err(MdmError::DefinitionInvalid(format!(
            "limit {name} exceeds the compiled absolute ceiling {ceiling}"
        )));
    }
    Ok(value)
}

pub fn validate_limits(limits: &std::collections::BTreeMap<String, Value>) -> Result<(), MdmError> {
    for (name, value) in limits {
        validate_limit_value(name, value)?;
    }
    let defaults = candidate_limits();
    let max_block = limits
        .get("max_block_records")
        .map(|value| validate_limit_value("max_block_records", value))
        .transpose()?
        .unwrap_or(defaults.max_block_records);
    let warning = limits
        .get("warning_block_records")
        .map(|value| validate_limit_value("warning_block_records", value))
        .transpose()?
        .unwrap_or(DEFAULT_WARNING_BLOCK_RECORDS.min(max_block));
    if warning > max_block {
        return Err(MdmError::DefinitionInvalid(
            "warning_block_records must not exceed max_block_records".into(),
        ));
    }
    Ok(())
}

pub fn expand_limits(
    limits: &mut std::collections::BTreeMap<String, Value>,
) -> Result<CandidateLimits, MdmError> {
    let defaults = candidate_limits();
    let max_block = limits
        .get("max_block_records")
        .map(|value| validate_limit_value("max_block_records", value))
        .transpose()?
        .unwrap_or(defaults.max_block_records);
    limits
        .entry("max_block_records".into())
        .or_insert_with(|| json!(max_block));
    limits
        .entry("max_candidate_pairs".into())
        .or_insert_with(|| json!(defaults.max_candidate_pairs));
    limits
        .entry("warning_block_records".into())
        .or_insert_with(|| json!(DEFAULT_WARNING_BLOCK_RECORDS.min(max_block)));
    validate_limits(limits)?;
    Ok(CandidateLimits {
        max_block_records: validate_limit_value("max_block_records", &limits["max_block_records"])?,
        max_candidate_pairs: validate_limit_value(
            "max_candidate_pairs",
            &limits["max_candidate_pairs"],
        )?,
    })
}

pub fn semantic_manifest() -> Value {
    json!({
        "candidate": {
            "version": 1,
            "defaults": {
                "max_block_records": DEFAULT_MAX_BLOCK_RECORDS,
                "max_candidate_pairs": DEFAULT_MAX_CANDIDATE_PAIRS,
                "warning_block_records": DEFAULT_WARNING_BLOCK_RECORDS
            },
            "absolute_ceilings": {
                "max_block_records": ABSOLUTE_MAX_BLOCK_RECORDS,
                "max_candidate_pairs": ABSOLUTE_MAX_CANDIDATE_PAIRS
            },
            "channels": ["exact", "composite_exact", "prefix", "token"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn lowering_the_block_limit_lowers_the_implicit_warning_too() {
        let mut limits = BTreeMap::from([("max_block_records".into(), json!(10))]);
        let expanded = expand_limits(&mut limits).unwrap();
        assert_eq!(expanded.max_block_records, 10);
        assert_eq!(limits["warning_block_records"], json!(10));
    }

    #[test]
    fn limits_cannot_escape_the_compiled_ceiling() {
        let mut limits = BTreeMap::from([(
            "max_candidate_pairs".into(),
            json!(ABSOLUTE_MAX_CANDIDATE_PAIRS + 1),
        )]);
        assert!(validate_limits(&limits).is_err());
        limits.insert("max_candidate_pairs".into(), json!(1));
        assert!(validate_limits(&limits).is_ok());
    }
}
