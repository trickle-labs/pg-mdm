pub mod exact;
pub mod levenshtein;

use crate::error::MdmError;

pub const SCORE_MAX: u16 = 10_000;
pub const EXACT_VERSION: u16 = 1;
pub const LEVENSHTEIN_VERSION: u16 = 1;
pub const DEFAULT_MAX_COMPARATOR_WORK: usize = 1_000_000;
pub const ABSOLUTE_MAX_COMPARATOR_WORK: usize = 100_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonClass {
    Agree,
    Disagree,
    NoEvidence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComparisonResult {
    pub class: ComparisonClass,
    pub score: Option<u16>,
}

pub fn comparator_name(comparison: &str) -> Option<&'static str> {
    match comparison {
        "exact" => Some("exact_v1"),
        "fuzzy" | "normalized_levenshtein" => Some("normalized_levenshtein_v1"),
        _ => None,
    }
}

pub fn comparator_version(comparison: &str) -> Option<u16> {
    match comparison {
        "exact" => Some(EXACT_VERSION),
        "fuzzy" | "normalized_levenshtein" => Some(LEVENSHTEIN_VERSION),
        _ => None,
    }
}

pub fn validate_threshold(
    comparison: &str,
    threshold: Option<i32>,
) -> Result<Option<u16>, MdmError> {
    let Some(version) = comparator_version(comparison) else {
        return Err(MdmError::ComparatorInvalid(format!(
            "unsupported comparator {comparison}"
        )));
    };
    let threshold = threshold
        .map(|value| {
            u16::try_from(value).map_err(|_| {
                MdmError::ComparatorInvalid(format!(
                    "threshold for {comparison}_v{version} must be between 0 and {SCORE_MAX}"
                ))
            })
        })
        .transpose()?;
    if threshold.is_some_and(|value| value > SCORE_MAX) {
        return Err(MdmError::ComparatorInvalid(format!(
            "threshold for {comparison}_v{version} must be between 0 and {SCORE_MAX}"
        )));
    }
    if comparison == "exact" && threshold.is_some() {
        return Err(MdmError::ComparatorInvalid(
            "exact_v1 does not accept a fuzzy threshold".into(),
        ));
    }
    Ok(threshold)
}

pub fn validate_work_limit(value: usize) -> Result<usize, MdmError> {
    if value == 0 || value > ABSOLUTE_MAX_COMPARATOR_WORK {
        return Err(MdmError::ComparatorInvalid(format!(
            "max_comparator_work must be between 1 and {ABSOLUTE_MAX_COMPARATOR_WORK}"
        )));
    }
    Ok(value)
}
