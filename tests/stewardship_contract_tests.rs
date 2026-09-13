use serde_json::Value;
use sha2::{Digest, Sha256};

const CONTRACT: &str = include_str!("../contracts/MDM-STEWARDSHIP-1.json");
const FIXTURE: &str = include_str!("../contracts/MDM-STEWARDSHIP-1-fixture.json");

fn verify_vector(domain: &str, vector: &Value) -> String {
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
    digest
}

#[test]
fn proposed_stewardship_digest_vectors_are_self_consistent() {
    let contract: Value = serde_json::from_str(CONTRACT).expect("shared contract parses");
    assert_eq!(contract["contract"], "MDM-STEWARDSHIP/1");
    assert_eq!(contract["status"], "proposed_for_joint_review");
    assert_eq!(contract["approvals"]["pg_mdm_owner"], "pending");
    assert_eq!(contract["approvals"]["pg_react_owner"], "pending");
    let fixture: Value = serde_json::from_str(FIXTURE).expect("shared fixture parses");
    assert_eq!(fixture["contract"], contract["contract"]);
    assert_eq!(fixture["revision"], contract["revision"]);
    assert_eq!(
        contract["shared_conformance_cases"],
        fixture["shared_conformance_cases"]
    );
    assert_eq!(
        contract["conformance_fixture"]["file"],
        "MDM-STEWARDSHIP-1-fixture.json"
    );
    let fixture_digest = Sha256::digest(FIXTURE.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(contract["conformance_fixture"]["sha256"], fixture_digest);

    let encoding = &contract["canonical_encoding"];
    for (domain_key, vector_key) in [
        ("basis_domain_tag", "basis_vector"),
        ("intent_domain_tag", "intent_vector"),
        ("request_key_domain_tag", "request_key_vector"),
    ] {
        verify_vector(
            encoding[domain_key].as_str().unwrap(),
            &encoding[vector_key],
        );
        assert_eq!(encoding[vector_key], fixture[vector_key]);
    }
    assert_eq!(
        fixture["intent_vector"]["request_key"],
        fixture["request_key_vector"]["sha256"]
    );

    let receipt_columns = contract["sql"]["receipts"]["columns"]
        .as_array()
        .expect("receipt schema has columns");
    assert!(receipt_columns.iter().any(|column| column[0] == "actor"));
    let conflict = &contract["intent"]["idempotency"]["changed_body"];
    assert_eq!(conflict["receipt_id"], Value::Null);
    assert_eq!(conflict["outcome"], "IDEMPOTENCY_CONFLICT");
    assert_eq!(conflict["reason_code"], "REQUEST_KEY_BODY_MISMATCH");
    assert_eq!(conflict["insert_conflict_receipt"], false);
    assert_eq!(conflict["existing_receipt"], "unchanged");
}
