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
fn fuzzy_threshold_and_work_limits_are_inclusive_at_the_boundary() {
    let equal_work = levenshtein::similarity("ab", "cd", 4).unwrap();
    assert_eq!(equal_work, 0);
    assert!(levenshtein::similarity("ab", "cd", 3).is_err());
    assert_eq!(
        levenshtein::compare(Some("ab"), Some("ac"), 5_000, 4)
            .unwrap()
            .class,
        ComparisonClass::Agree
    );
    assert_eq!(
        levenshtein::compare(Some("ab"), Some("ac"), 5_001, 4)
            .unwrap()
            .class,
        ComparisonClass::Disagree
    );
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
    let reordered_rules = rules.iter().cloned().rev().collect::<Vec<_>>();
    assert_eq!(
        evidence,
        evaluate_pair(&pair, &reordered_rules, &values, 1_000_000).unwrap()
    );
    assert_eq!(evidence[0].rule, "a_rule");
    assert_eq!(evidence[0].class, EvidenceClass::NoEvidence);
    assert_eq!(evidence[1].class, EvidenceClass::Agree);
}

#[test]
fn only_value_state_can_produce_matching_evidence() {
    let left = Uuid::from_bytes([1; 16]);
    let right = Uuid::from_bytes([2; 16]);
    let pair = CandidatePair {
        left_source_record_id: left,
        right_source_record_id: right,
        left_sort_key: vec![1],
        right_sort_key: vec![2],
        discovery_channels: vec!["email".into()],
    };
    let rules = [MatchRule {
        name: "email".into(),
        fields: vec!["email".into()],
        comparison: "exact".into(),
        strength: "identity".into(),
        evidence_group: "identity".into(),
        threshold: None,
        candidate: None,
    }];

    for state in [
        NormalizedState::Absent,
        NormalizedState::Empty,
        NormalizedState::Invalid,
        NormalizedState::Unknown,
        NormalizedState::Redacted,
        NormalizedState::Unsupported,
    ] {
        let values = [left, right].map(|source_record_id| NormalizedEvidenceValue {
            source_record_id,
            field: "email".into(),
            state,
            normalized: Some("same@example.test".into()),
            canonical_bytes: Some(b"same@example.test".to_vec()),
        });
        let evidence = evaluate_pair(&pair, &rules, &values, 1_000_000).unwrap();
        assert_eq!(
            evidence[0].class,
            EvidenceClass::NoEvidence,
            "state {state:?}"
        );
        assert!(evidence[0].left_value_digest.is_none());
        assert!(evidence[0].right_value_digest.is_none());
    }
}
