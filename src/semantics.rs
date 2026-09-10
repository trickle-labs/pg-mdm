use serde_json::{Value, json};

use crate::candidate::CandidateLimits;
use crate::comparators::{self, DEFAULT_MAX_COMPARATOR_WORK};
use crate::error::MdmError;
use crate::resolver::ResolverLimits;

pub const DEFAULT_MAX_BLOCK_RECORDS: usize = 10_000;
pub const DEFAULT_MAX_CANDIDATE_PAIRS: usize = 1_000_000;
pub const DEFAULT_WARNING_BLOCK_RECORDS: usize = 5_000;
pub const ABSOLUTE_MAX_BLOCK_RECORDS: usize = 1_000_000;
pub const ABSOLUTE_MAX_CANDIDATE_PAIRS: usize = 100_000_000;
pub const DEFAULT_MAX_DECISION_CLOSURE: usize = 10_000;
pub const ABSOLUTE_MAX_DECISION_CLOSURE: usize = 1_000_000;
pub const DEFAULT_MAX_ACTIVE_RECORDS: usize = 100_000;
pub const DEFAULT_MAX_AUTOMATIC_EDGES: usize = 1_000_000;
pub const DEFAULT_MAX_RECORDS_PER_COMPONENT: usize = 100_000;
pub const DEFAULT_MAX_COMPONENT_CHECKS: usize = 1_000_000;
pub const ABSOLUTE_MAX_ACTIVE_RECORDS: usize = 10_000_000;
pub const ABSOLUTE_MAX_AUTOMATIC_EDGES: usize = 100_000_000;
pub const ABSOLUTE_MAX_RECORDS_PER_COMPONENT: usize = 10_000_000;
pub const ABSOLUTE_MAX_COMPONENT_CHECKS: usize = 100_000_000;
pub const CLUSTERING_POLICY_VERSION: u16 = 1;

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
        "max_comparator_work" => comparators::ABSOLUTE_MAX_COMPARATOR_WORK,
        "max_decision_closure" => ABSOLUTE_MAX_DECISION_CLOSURE,
        "max_active_records" => ABSOLUTE_MAX_ACTIVE_RECORDS,
        "max_automatic_edges" => ABSOLUTE_MAX_AUTOMATIC_EDGES,
        "max_records_per_component" => ABSOLUTE_MAX_RECORDS_PER_COMPONENT,
        "max_component_checks" => ABSOLUTE_MAX_COMPONENT_CHECKS,
        _ => {
            return Err(MdmError::DefinitionInvalid(format!(
                "unsupported limit {name}"
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
    limits
        .entry("max_comparator_work".into())
        .or_insert_with(|| json!(DEFAULT_MAX_COMPARATOR_WORK));
    limits
        .entry("max_decision_closure".into())
        .or_insert_with(|| json!(DEFAULT_MAX_DECISION_CLOSURE));
    let resolver = ResolverLimits::default();
    limits
        .entry("max_active_records".into())
        .or_insert_with(|| json!(resolver.max_active_records));
    limits
        .entry("max_automatic_edges".into())
        .or_insert_with(|| json!(resolver.max_automatic_edges));
    limits
        .entry("max_records_per_component".into())
        .or_insert_with(|| json!(resolver.max_records_per_component));
    limits
        .entry("max_component_checks".into())
        .or_insert_with(|| json!(resolver.max_component_checks));
    validate_limits(limits)?;
    Ok(CandidateLimits {
        max_block_records: validate_limit_value("max_block_records", &limits["max_block_records"])?,
        max_candidate_pairs: validate_limit_value(
            "max_candidate_pairs",
            &limits["max_candidate_pairs"],
        )?,
    })
}

pub fn resolver_limits(
    limits: &std::collections::BTreeMap<String, Value>,
) -> Result<ResolverLimits, MdmError> {
    let defaults = ResolverLimits::default();
    let get = |name: &'static str, default| {
        limits
            .get(name)
            .map(|value| validate_limit_value(name, value))
            .transpose()
            .map(|value| value.unwrap_or(default))
    };
    Ok(ResolverLimits {
        max_active_records: get("max_active_records", defaults.max_active_records)?,
        max_automatic_edges: get("max_automatic_edges", defaults.max_automatic_edges)?,
        max_records_per_component: get(
            "max_records_per_component",
            defaults.max_records_per_component,
        )?,
        max_component_checks: get("max_component_checks", defaults.max_component_checks)?,
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
        },
        "evidence": {
            "version": 1,
            "comparators": {
                "exact_v1": 1,
                "normalized_levenshtein_v1": 1
            },
            "default_max_comparator_work": DEFAULT_MAX_COMPARATOR_WORK,
            "absolute_max_comparator_work": comparators::ABSOLUTE_MAX_COMPARATOR_WORK
        },
        "decisions": {
            "policy_version": 1,
            "default_max_decision_closure": DEFAULT_MAX_DECISION_CLOSURE,
            "absolute_max_decision_closure": ABSOLUTE_MAX_DECISION_CLOSURE
        },
        "clustering": {
            "policy_version": CLUSTERING_POLICY_VERSION,
            "defaults": {
                "max_active_records": DEFAULT_MAX_ACTIVE_RECORDS,
                "max_automatic_edges": DEFAULT_MAX_AUTOMATIC_EDGES,
                "max_records_per_component": DEFAULT_MAX_RECORDS_PER_COMPONENT,
                "max_component_checks": DEFAULT_MAX_COMPONENT_CHECKS
            },
            "absolute_ceilings": {
                "max_active_records": ABSOLUTE_MAX_ACTIVE_RECORDS,
                "max_automatic_edges": ABSOLUTE_MAX_AUTOMATIC_EDGES,
                "max_records_per_component": ABSOLUTE_MAX_RECORDS_PER_COMPONENT,
                "max_component_checks": ABSOLUTE_MAX_COMPONENT_CHECKS
            },
            "admission": {
                "singleton_singleton": ["identity", "strong"],
                "singleton_established": ["identity", "strong_plus_independent_group"],
                "established_established": ["shared_authority", "two_independent_strong_connections"]
            }
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
