use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(crate) const BASIS_DOMAIN_TAG: &str = "pg_mdm/policy-case-basis/v1";

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

fn write_part(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u32).to_be_bytes());
    hasher.update(bytes);
}
