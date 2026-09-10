use pg_mdm::candidate::CandidatePair;
use pg_mdm::comparators::{ComparisonClass, exact, levenshtein};
use pg_mdm::definition::MatchRule;
use pg_mdm::evidence::{EvidenceClass, NormalizedEvidenceValue, evaluate_pair};
use pg_mdm::normalization::NormalizedState;
use pgrx::Uuid;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Fixture {
    format_version: u8,
    vectors: Vec<Vector>,
}

#[derive(Deserialize)]
struct Vector {
    comparator: String,
    left: String,
    right: String,
    class: String,
    score: Option<u16>,
    threshold: Option<u16>,
}

#[test]
fn comparator_fixture_matches_fixed_point_contract() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/comparator_v1.json")).unwrap();
    assert_eq!(fixture.format_version, 1);
    for vector in fixture.vectors {
        if vector.comparator == "exact_v1" {
            let result =
                exact::compare(Some(vector.left.as_bytes()), Some(vector.right.as_bytes()));
            assert_eq!(result.score, vector.score);
            assert_eq!(
                result.class,
                if vector.class == "agree" {
                    ComparisonClass::Agree
                } else {
                    ComparisonClass::Disagree
                }
            );
        } else {
            let score = levenshtein::similarity(&vector.left, &vector.right, 1_000_000).unwrap();
            assert_eq!(Some(score), vector.score);
            let result = levenshtein::compare(
                Some(&vector.left),
                Some(&vector.right),
                vector.threshold.unwrap(),
                1_000_000,
            )
            .unwrap();
            assert_eq!(
                result.class,
                if vector.class == "agree" {
                    ComparisonClass::Agree
                } else {
                    ComparisonClass::Disagree
                }
            );
        }
    }
}

#[test]
fn evidence_ignores_unusable_values_and_sorts_rules() {
    let left = Uuid::from_bytes([1; 16]);
    let right = Uuid::from_bytes([2; 16]);
    let pair = CandidatePair {
        left_source_record_id: left,
        right_source_record_id: right,
        left_sort_key: vec![1],
        right_sort_key: vec![2],
        discovery_channels: vec!["email".into()],
    };
    let rules = vec![
        MatchRule {
            name: "z_rule".into(),
            fields: vec!["email".into()],
            comparison: "exact".into(),
            strength: "supporting".into(),
            evidence_group: "contact".into(),
            threshold: None,
            candidate: None,
        },
        MatchRule {
            name: "a_rule".into(),
            fields: vec!["name".into()],
            comparison: "normalized_levenshtein".into(),
            strength: "strong".into(),
            evidence_group: "identity".into(),
            threshold: Some(8_000),
            candidate: Some(json!({"kind":"token"})),
        },
    ];
    let values = vec![
        NormalizedEvidenceValue {
            source_record_id: left,
            field: "email".into(),
            state: NormalizedState::Value,
            normalized: Some("a@example.test".into()),
            canonical_bytes: Some(b"a@example.test".to_vec()),
        },
        NormalizedEvidenceValue {
            source_record_id: right,
            field: "email".into(),
            state: NormalizedState::Value,
            normalized: Some("a@example.test".into()),
            canonical_bytes: Some(b"a@example.test".to_vec()),
        },
        NormalizedEvidenceValue {
            source_record_id: left,
            field: "name".into(),
            state: NormalizedState::Absent,
            normalized: None,
            canonical_bytes: None,
        },
    ];
    let evidence = evaluate_pair(&pair, &rules, &values, 1_000_000).unwrap();
    assert_eq!(evidence[0].rule, "a_rule");
    assert_eq!(evidence[0].class, EvidenceClass::NoEvidence);
    assert_eq!(evidence[1].class, EvidenceClass::Agree);
}
