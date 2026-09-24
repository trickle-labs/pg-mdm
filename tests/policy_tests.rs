#[path = "../src/policy.rs"]
mod policy;

use policy::{
    IntentArguments, PolicyActionTuple, PolicyCaseBasis, PolicyIntentBody, PolicySubject,
    basis_canonical_json, basis_digest, intent_canonical_json, intent_digest, next_action_revision,
    parse_intent_arguments, queue_is_allowed, validate_reference,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn basis() -> PolicyCaseBasis {
    PolicyCaseBasis {
        approved_metadata: json!({}),
        artifact_digest: "3".repeat(64),
        canonical_encoding_version: 1,
        definition_version: 3,
        entity_name: "customer".into(),
        issue_key: "1".repeat(64),
        observed_decision_epoch: 8,
        occurrence: 1,
        reason_code: "POSSIBLE_DUPLICATE".into(),
        semantic_versions: BTreeMap::from([
            ("candidate".into(), 1),
            ("clustering".into(), 1),
            ("normalization".into(), 1),
        ]),
        source_boundary_digest: "2".repeat(64),
        status: "open".into(),
        subjects: vec![
            PolicySubject {
                id: "40000000-0000-4000-8000-000000000001".into(),
                kind: "source_record".into(),
            },
            PolicySubject {
                id: "40000000-0000-4000-8000-000000000002".into(),
                kind: "source_record".into(),
            },
        ],
    }
}

fn action() -> PolicyActionTuple {
    PolicyActionTuple {
        status: "open".into(),
        reason_code: "POSSIBLE_DUPLICATE".into(),
        approved_metadata: json!({}),
        permitted_actions: vec![
            "ASSIGN_QUEUE".into(),
            "SET_DUE_AT".into(),
            "ESCALATE".into(),
        ],
        assigned_queue: None,
        due_at: None,
        escalation_level: 0,
        manual_assignment_protected: false,
        opening_time_available: true,
        pending_stewardship: false,
        review_version: 4,
        definition_version: 3,
        stewardship_epoch: 8,
        evidence_basis_digest: [0xab; 32],
    }
}

#[test]
fn basis_matches_signed_fixture_bytes_and_digest() {
    let fixture: Value =
        serde_json::from_str(include_str!("../contracts/MDM-STEWARDSHIP-1-fixture.json")).unwrap();
    let vector = &fixture["basis_vector"];
    assert_eq!(
        String::from_utf8(basis_canonical_json(&basis())).unwrap(),
        vector["canonical_json_utf8"]
    );
    let expected = vector["sha256"].as_str().unwrap();
    let actual = basis_digest(&basis())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(actual, expected);
}

#[test]
fn action_revision_changes_only_for_a_changed_tuple() {
    let before = action();
    assert_eq!(next_action_revision(2, &before, &before), Some(2));

    let mut after = action();
    after.pending_stewardship = true;
    assert_eq!(next_action_revision(2, &before, &after), Some(3));
    assert_eq!(next_action_revision(u64::MAX, &before, &after), None);
}

#[test]
fn intent_matches_signed_fixture_bytes_and_digest() {
    let fixture: Value =
        serde_json::from_str(include_str!("../contracts/MDM-STEWARDSHIP-1-fixture.json")).unwrap();
    let vector = &fixture["intent_vector"];
    let body = PolicyIntentBody {
        action: "ASSIGN_QUEUE".into(),
        arguments: json!({"queue": "priority"}),
        binding_id: "30000000-0000-4000-8000-000000000001".into(),
        case_key: 7,
        evaluation_ref: "eval-1".into(),
        expected_action_revision: 2,
        expected_definition_version: 3,
        expected_evidence_basis_digest: "ab".repeat(32),
        expected_policy_digest: "741ea9560a69ba3185eaa34760ba38d473aa43daa8e8c530c6c6c2ce867c2614"
            .into(),
        expected_publication_revision: 5,
        expected_review_version: 4,
        expected_stewardship_epoch: 8,
        policy_revision: "policy-1".into(),
        work_ref: "work-1".into(),
    };
    assert_eq!(
        String::from_utf8(intent_canonical_json(&body)).unwrap(),
        vector["canonical_json_utf8"]
    );
    let actual = intent_digest(&body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(actual, vector["sha256"]);
    assert_eq!(
        parse_intent_arguments("ASSIGN_QUEUE", &json!({"queue": "priority"})),
        Ok(IntentArguments::AssignQueue("priority".into()))
    );
    assert!(validate_reference("work-1", "work_ref").is_ok());
    assert!(queue_is_allowed(
        "priority",
        &["priority".to_owned(), "standard".to_owned()]
    ));
    assert!(!queue_is_allowed(
        "blocked",
        &["priority".to_owned(), "standard".to_owned()]
    ));
    assert!(!queue_is_allowed("priority", &[]));
}
