use pgrx::Uuid;

use crate::constraint::{DecisionEdge, DecisionKind};
use crate::error::MdmError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionWrite {
    pub entity_id: Uuid,
    pub left_source_record_id: Uuid,
    pub right_source_record_id: Uuid,
    pub decision: DecisionKind,
    pub expected_version: i64,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionResult {
    pub operation_id: Uuid,
    pub decision_id: Uuid,
    pub decision_version: i64,
    pub decision_epoch: i64,
}

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
