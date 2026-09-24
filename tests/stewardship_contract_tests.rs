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
fn approved_stewardship_contract_and_vectors_match_fixture() {
    let contract: Value = serde_json::from_str(CONTRACT).expect("shared contract parses");
    assert_eq!(contract["contract"], "MDM-STEWARDSHIP/1");
    assert_eq!(contract["status"], "approved");
    assert_eq!(contract["approvals"]["pg_mdm_owner"]["status"], "approved");
    assert_eq!(
        contract["approvals"]["pg_mdm_owner"]["role"],
        "pg-mdm M0 contract owner"
    );
    assert_eq!(
        contract["approvals"]["pg_react_owner"]["status"],
        "approved"
    );
    assert_eq!(
        contract["approvals"]["pg_react_owner"]["role"],
        "pg-react R0 adapter owner"
    );
    let fixture: Value = serde_json::from_str(FIXTURE).expect("shared fixture parses");
    assert_eq!(fixture["contract"], contract["contract"]);
    assert!(fixture["revision"].as_u64().unwrap() <= contract["revision"].as_u64().unwrap());
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
        ("policy_domain_tag", "policy_vector"),
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
    assert_eq!(
        fixture["intent_vector"]["body"]["expected_policy_digest"],
        fixture["policy_vector"]["sha256"]
    );
    assert_eq!(
        contract["freshness"]["review_version_meaning"],
        "The exact concurrency_version of the mdm_out.<entity>_review row for this case's review_id."
    );
    assert!(
        contract["freshness"]["pending_stewardship_rule"]
            .as_str()
            .unwrap()
            .contains("PENDING_STEWARDSHIP")
    );
    assert!(
        contract["intent"]["outcomes"]
            .as_array()
            .unwrap()
            .contains(&Value::String("PENDING_STEWARDSHIP".into()))
    );

    let receipt_columns = contract["sql"]["receipts"]["columns"]
        .as_array()
        .expect("receipt schema has columns");
    assert!(receipt_columns.iter().any(|column| {
        column[0] == "actor"
            && column[1] == "name"
            && column[2].as_str().unwrap().contains("current_user")
    }));
    let bindings = &contract["sql"]["bindings"];
    assert_eq!(bindings["relation"], "mdm_steward.policy_bindings_v1");
    assert_eq!(
        bindings["administrator_surface"]["create"],
        "mdm_admin.create_policy_binding(entity_name text, automation_role_name text, policy_digest bytea, allowed_actions text[], allowed_queues text[], max_due_interval interval, max_escalation_level integer) returns (binding_id uuid, binding_version bigint). Validates queue strings and stores canonical values in allowed_queues name[]. Creates a binding and active runtime row at version one."
    );
    assert!(
        bindings["administrator_surface"]["authorization"]
            .as_str()
            .unwrap()
            .contains("entity execution role")
    );
    assert!(
        bindings["invariants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule
                .as_str()
                .unwrap()
                .contains("one unreplaced binding exists per entity and automation role"))
    );
    let conflict = &contract["intent"]["idempotency"]["changed_body"];
    assert_eq!(conflict["receipt_id"], Value::Null);
    assert_eq!(conflict["outcome"], "IDEMPOTENCY_CONFLICT");
    assert_eq!(conflict["reason_code"], "REQUEST_KEY_BODY_MISMATCH");
    assert_eq!(conflict["insert_conflict_receipt"], false);
    assert_eq!(conflict["existing_receipt"], "unchanged");
}
