use serde_json::Value;
use sha2::{Digest, Sha256};

const CONTRACT: &str = include_str!("../contracts/MDM-STEWARDSHIP-1.json");

fn verify_vector(domain: &str, vector: &Value) {
    let canonical = serde_json::to_string(&vector["body"]).expect("canonical JSON serializes");
    assert_eq!(vector["canonical_json_utf8"], canonical);
    let tag = domain.as_bytes();
    let body = canonical.as_bytes();
    let mut hasher = Sha256::new();
    hasher.update((tag.len() as u32).to_be_bytes());
    hasher.update(tag);
    hasher.update((body.len() as u32).to_be_bytes());
    hasher.update(body);
    let digest = hasher.finalize();
    let digest = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(vector["sha256"], digest);
}

#[test]
fn proposed_stewardship_digest_vectors_are_self_consistent() {
    let contract: Value = serde_json::from_str(CONTRACT).expect("shared contract parses");
    assert_eq!(contract["contract"], "MDM-STEWARDSHIP/1");
    assert_eq!(contract["status"], "proposed_for_joint_review");
    assert_eq!(contract["approvals"]["pg_mdm_owner"], "pending");
    assert_eq!(contract["approvals"]["pg_react_owner"], "pending");
    let encoding = &contract["canonical_encoding"];
    verify_vector(
        encoding["basis_domain_tag"].as_str().unwrap(),
        &encoding["basis_vector"],
    );
    verify_vector(
        encoding["intent_domain_tag"].as_str().unwrap(),
        &encoding["intent_vector"],
    );
}
