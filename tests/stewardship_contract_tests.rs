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

const PROPOSAL: &str = include_str!("../contracts/proposals/MDM-STEWARDSHIP-1-v4.json");

#[test]
fn pending_publication_age_contract_is_explicitly_proposed() {
    let proposal: Value = serde_json::from_str(PROPOSAL).expect("contract proposal parses");
    assert_eq!(proposal["contract"], "MDM-STEWARDSHIP/1");
    assert_eq!(proposal["revision"], 4);
    assert_eq!(proposal["status"], "proposed");
    assert_eq!(proposal["sql"]["relation"], "mdm_steward.policy_cases_v1");
    assert_eq!(proposal["sql"]["append_column"]["name"], "pending_since");
    assert_eq!(proposal["sql"]["append_column"]["type"], "timestamptz");
    assert_eq!(
        proposal["sql"]["append_receipt_column"]["name"],
        "publication_tracking_started_at"
    );
    assert_eq!(
        proposal["sql"]["new_relation"]["relation"],
        "mdm_steward.policy_receipt_publications_v1"
    );
    assert_eq!(
        proposal["sql"]["new_relation"]["columns"][4],
        serde_json::json!([
            "publication_state",
            "text",
            "PUBLISHED or SUPERSEDED_BEFORE_PUBLICATION"
        ])
    );
    assert_eq!(proposal["approvals"]["pg_mdm_owner"]["status"], "approved");
    assert_eq!(
        proposal["approvals"]["pg_mdm_owner"]["approved_on"],
        "2026-11-04"
    );
    assert_eq!(
        proposal["approvals"]["pg_react_owner"]["status"],
        "approved"
    );
    assert_eq!(
        proposal["approvals"]["pg_react_owner"]["approved_on"],
        "2026-11-04"
    );
    assert_eq!(
        proposal["lifecycle"]["legacy_rows"],
        "Existing pending rows receive NULL because their transition time cannot be reconstructed; consumers report age as unknown."
    );
    assert!(proposal["lifecycle"]["accepted_controls"]
        .as_str()
        .expect("accepted-control lifecycle is present")
        .contains("Legacy receipts predating publication tracking remain UNKNOWN_LEGACY with unknown age."));
}
