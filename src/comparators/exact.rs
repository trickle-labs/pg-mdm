use super::{ComparisonClass, ComparisonResult};

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
