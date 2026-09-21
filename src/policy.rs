use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(crate) const BASIS_DOMAIN_TAG: &str = "pg_mdm/policy-case-basis/v1";
pub(crate) const INTENT_DOMAIN_TAG: &str = "pg_mdm/policy-intent/v1";

#[derive(Debug, Serialize)]
pub(crate) struct PolicySubject {
    pub(crate) id: String,
    pub(crate) kind: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PolicyCaseBasis {
    pub(crate) approved_metadata: Value,
    pub(crate) artifact_digest: String,
    pub(crate) canonical_encoding_version: u8,
    pub(crate) definition_version: u64,
    pub(crate) entity_name: String,
    pub(crate) issue_key: String,
    pub(crate) observed_decision_epoch: u64,
    pub(crate) occurrence: u32,
    pub(crate) reason_code: String,
    pub(crate) semantic_versions: BTreeMap<String, u64>,
    pub(crate) source_boundary_digest: String,
    pub(crate) status: String,
    pub(crate) subjects: Vec<PolicySubject>,
}

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct PolicyActionTuple {
    pub(crate) status: String,
    pub(crate) reason_code: String,
    pub(crate) approved_metadata: Value,
    pub(crate) permitted_actions: Vec<String>,
    pub(crate) assigned_queue: Option<String>,
    pub(crate) due_at: Option<String>,
    pub(crate) escalation_level: u32,
    pub(crate) manual_assignment_protected: bool,
    pub(crate) opening_time_available: bool,
    pub(crate) pending_stewardship: bool,
    pub(crate) review_version: u64,
    pub(crate) definition_version: u64,
    pub(crate) stewardship_epoch: u64,
    pub(crate) evidence_basis_digest: [u8; 32],
}

pub(crate) fn basis_canonical_json(basis: &PolicyCaseBasis) -> Vec<u8> {
    serde_json::to_vec(&serde_json::to_value(basis).expect("policy basis is serializable"))
        .expect("policy basis JSON is serializable")
}

pub(crate) fn basis_digest(basis: &PolicyCaseBasis) -> [u8; 32] {
    let body = basis_canonical_json(basis);
    let mut hasher = Sha256::new();
    write_part(&mut hasher, BASIS_DOMAIN_TAG.as_bytes());
    write_part(&mut hasher, &body);
    hasher.finalize().into()
}

pub(crate) fn next_action_revision(
    previous: u64,
    before: &PolicyActionTuple,
    after: &PolicyActionTuple,
) -> Option<u64> {
    if before == after {
        Some(previous)
    } else {
        previous.checked_add(1)
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum IntentArguments {
    AssignQueue(String),
    SetDueAt(String),
    Escalate(i32),
}

#[derive(Debug, Serialize)]
pub(crate) struct PolicyIntentBody {
    pub(crate) action: String,
    pub(crate) arguments: Value,
    pub(crate) binding_id: String,
    pub(crate) case_key: i64,
    pub(crate) evaluation_ref: String,
    pub(crate) expected_action_revision: i64,
    pub(crate) expected_definition_version: i64,
    pub(crate) expected_evidence_basis_digest: String,
    pub(crate) expected_policy_digest: String,
    pub(crate) expected_publication_revision: i64,
    pub(crate) expected_review_version: i64,
    pub(crate) expected_stewardship_epoch: i64,
    pub(crate) policy_revision: String,
    pub(crate) work_ref: String,
}

pub(crate) fn validate_reference(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.as_bytes().contains(&0) {
        return Err(format!("{label} must be 1..256 UTF-8 bytes"));
    }
    Ok(())
}

pub(crate) fn parse_intent_arguments(
    action: &str,
    arguments: &Value,
) -> Result<IntentArguments, String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "arguments must be a JSON object".to_owned())?;
    match action {
        "ASSIGN_QUEUE" => {
            if object.len() != 1 {
                return Err("ASSIGN_QUEUE arguments must contain only queue".into());
            }
            let queue = object
                .get("queue")
                .and_then(Value::as_str)
                .ok_or_else(|| "queue must be a non-empty string".to_owned())?;
            if queue.is_empty() || queue.len() > 63 || queue.as_bytes().contains(&0) {
                return Err("queue must be 1..63 UTF-8 bytes".into());
            }
            Ok(IntentArguments::AssignQueue(queue.to_owned()))
        }
        "SET_DUE_AT" => {
            if object.len() != 1 {
                return Err("SET_DUE_AT arguments must contain only due_at".into());
            }
            let due_at = object
                .get("due_at")
                .and_then(Value::as_str)
                .ok_or_else(|| "due_at must be a timestamp string".to_owned())?;
            if due_at.is_empty() {
                return Err("due_at must not be empty".into());
            }
            Ok(IntentArguments::SetDueAt(due_at.to_owned()))
        }
        "ESCALATE" => {
            if object.len() != 1 {
                return Err("ESCALATE arguments must contain only level".into());
            }
            let level = object
                .get("level")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| "level must be a positive integer".to_owned())?;
            if level <= 0 {
                return Err("level must be a positive integer".into());
            }
            Ok(IntentArguments::Escalate(level))
        }
        _ => Err("action must be ASSIGN_QUEUE, SET_DUE_AT, or ESCALATE".into()),
    }
}

pub(crate) fn intent_canonical_json(body: &PolicyIntentBody) -> Vec<u8> {
    serde_json::to_vec(&serde_json::to_value(body).expect("policy intent is serializable"))
        .expect("policy intent JSON is serializable")
}

pub(crate) fn intent_digest(body: &PolicyIntentBody) -> [u8; 32] {
    let body = intent_canonical_json(body);
    let mut hasher = Sha256::new();
    write_part(&mut hasher, INTENT_DOMAIN_TAG.as_bytes());
    write_part(&mut hasher, &body);
    hasher.finalize().into()
}

fn write_part(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u32).to_be_bytes());
    hasher.update(bytes);
}
