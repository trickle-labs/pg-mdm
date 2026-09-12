use pgrx::Uuid;

use crate::constraint::{DecisionEdge, DecisionKind};
use crate::error::MdmError;

pub fn validate_reason(reason: &str) -> Result<(), MdmError> {
    if reason.trim().is_empty() {
        return Err(MdmError::DecisionInvalid(
            "reason must contain non-whitespace text".into(),
        ));
    }
    Ok(())
}

pub fn next_version(current: Option<i64>, expected: i64) -> Result<i64, MdmError> {
    let current = current.unwrap_or(0);
    if current != expected {
        return Err(MdmError::DecisionVersionConflict(format!(
            "expected version {expected}, current version {current}"
        )));
    }
    current
        .checked_add(1)
        .ok_or_else(|| MdmError::DecisionVersionConflict("decision version exhausted".into()))
}

pub fn edge(
    decision_id: Uuid,
    left_source_record_id: Uuid,
    right_source_record_id: Uuid,
    decision: DecisionKind,
) -> DecisionEdge {
    DecisionEdge {
        decision_id,
        left_source_record_id,
        right_source_record_id,
        decision,
    }
    .canonical()
}
