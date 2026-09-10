use super::{ComparisonClass, ComparisonResult, SCORE_MAX};

pub fn compare(left: Option<&[u8]>, right: Option<&[u8]>) -> ComparisonResult {
    match (left, right) {
        (Some(left), Some(right)) => ComparisonResult {
            class: if left == right {
                ComparisonClass::Agree
            } else {
                ComparisonClass::Disagree
            },
            score: None,
        },
        _ => ComparisonResult {
            class: ComparisonClass::NoEvidence,
            score: None,
        },
    }
}

pub fn score(left: Option<&[u8]>, right: Option<&[u8]>) -> Option<u16> {
    match compare(left, right).class {
        ComparisonClass::Agree => Some(SCORE_MAX),
        ComparisonClass::Disagree => Some(0),
        ComparisonClass::NoEvidence => None,
    }
}
