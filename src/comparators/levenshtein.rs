use super::{ComparisonClass, ComparisonResult};
use crate::error::MdmError;
use pgrx::prelude::*;

// ponytail: O(n*m) fuzzy comparison, replace it with a bounded bit-parallel
// implementation only if comparator CPU dominates supported workloads.
pub fn similarity(left: &str, right: &str, max_work: usize) -> Result<u16, MdmError> {
    let left_len = left.chars().count();
    let right_len = right.chars().count();
    if left_len == 0 && right_len == 0 {
        return Ok(10_000);
    }
    let work = left_len
        .checked_mul(right_len)
        .ok_or(MdmError::ComparatorWorkLimit {
            work: usize::MAX,
            limit: max_work,
        })?;
    if work > max_work {
        return Err(MdmError::ComparatorWorkLimit {
            work,
            limit: max_work,
        });
    }
    if left_len == 0 || right_len == 0 {
        return Ok(0);
    }

    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();

    let (short, long) = if left_len <= right_len {
        (&left, &right)
    } else {
        (&right, &left)
    };
    let mut previous: Vec<usize> = (0..=short.len()).collect();
    let mut current = vec![0; short.len() + 1];
    for (long_index, long_char) in long.iter().enumerate() {
        current[0] = long_index + 1;
        for (short_index, short_char) in short.iter().enumerate() {
            current[short_index + 1] = if long_char == short_char {
                previous[short_index]
            } else {
                1 + previous[short_index]
                    .min(previous[short_index + 1])
                    .min(current[short_index])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[short.len()];
    let max_len = left_len.max(right_len);
    Ok((((max_len - distance) as u128 * 10_000) / max_len as u128) as u16)
}

pub fn compare(
    left: Option<&str>,
    right: Option<&str>,
    threshold: u16,
    max_work: usize,
) -> Result<ComparisonResult, MdmError> {
    let (Some(left), Some(right)) = (left, right) else {
        return Ok(ComparisonResult {
            class: ComparisonClass::NoEvidence,
            score: None,
        });
    };
    let score = similarity(left, right, max_work)?;
    Ok(ComparisonResult {
        class: if score >= threshold {
            ComparisonClass::Agree
        } else {
            ComparisonClass::Disagree
        },
        score: Some(score),
    })
}

#[pg_extern(
    name = "normalized_levenshtein_score",
    requires = ["pg_mdm_foundation"],
    sql = "CREATE FUNCTION mdm_internal.normalized_levenshtein_score(left_value text, right_value text, max_work bigint) RETURNS integer IMMUTABLE PARALLEL SAFE SET search_path = pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'normalized_levenshtein_score_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn normalized_levenshtein_score(
    left_value: Option<String>,
    right_value: Option<String>,
    max_work: i64,
) -> i32 {
    let (Some(left_value), Some(right_value)) = (left_value, right_value) else {
        return 0;
    };
    let limit = usize::try_from(max_work).unwrap_or(0);
    match similarity(&left_value, &right_value, limit) {
        Ok(score) => i32::from(score),
        Err(error) => pgrx::error!("{}: {}", error.code(), error),
    }
}
