use pg_mdm::candidate::CandidatePair;
use pg_mdm::definition::MatchRule;
use pg_mdm::evidence::{EvidenceClass, EvidenceItem};
use pg_mdm::pair::{PairResult, decide_pair_for_rules, sort_automatic_edges};
use pgrx::Uuid;
use serde::Deserialize;

#[derive(Deserialize)]
struct PolicyFixture {
    format_version: u8,
    policy_version: u8,
    precedence: Vec<String>,
}

fn pair() -> CandidatePair {
    CandidatePair {
        left_source_record_id: Uuid::from_bytes([1; 16]),
        right_source_record_id: Uuid::from_bytes([2; 16]),
        left_sort_key: vec![1],
        right_sort_key: vec![2],
        discovery_channels: vec!["x".into()],
    }
}

fn item(rule: &str, group: &str, class: EvidenceClass, score: Option<u16>) -> EvidenceItem {
    EvidenceItem {
        rule: rule.into(),
        evidence_group: group.into(),
        class,
        score,
        comparator: "exact_v1".into(),
        comparator_version: 1,
        left_value_digest: None,
        right_value_digest: None,
    }
}

fn rule(name: &str, strength: &str, group: &str) -> MatchRule {
    MatchRule {
        name: name.into(),
        fields: vec!["field".into()],
        comparison: "exact".into(),
        strength: strength.into(),
        evidence_group: group.into(),
        threshold: None,
        candidate: None,
    }
}

#[test]
fn precedence_fixture_and_manual_overrides() {
    let fixture: PolicyFixture =
        serde_json::from_str(include_str!("fixtures/pair_precedence.json")).unwrap();
    assert_eq!((fixture.format_version, fixture.policy_version), (1, 1));
    assert_eq!(fixture.precedence[0], "PROHIBITED");
    let rules = vec![
        rule("identity", "identity", "id"),
        rule("strong", "strong", "strong"),
    ];
    let evidence = vec![
        item("identity", "id", EvidenceClass::Agree, None),
        item("strong", "strong", EvidenceClass::Agree, Some(9000)),
    ];
    assert_eq!(
        decide_pair_for_rules(pair(), evidence.clone(), &rules, true, None).result,
        PairResult::AuthoritativeConflict
    );
    assert_eq!(
        decide_pair_for_rules(pair(), evidence.clone(), &rules, true, Some("MATCH")).result,
        PairResult::ManualMatch
    );
    assert_eq!(
        decide_pair_for_rules(pair(), evidence, &rules, false, Some("NOT_MATCH")).result,
        PairResult::Prohibited
    );
}

#[test]
fn automatic_sort_is_total_and_identity_first() {
    let identity = decide_pair_for_rules(
        pair(),
        vec![item("id", "id", EvidenceClass::Agree, None)],
        &[rule("id", "identity", "id")],
        false,
        None,
    );
    let strong = decide_pair_for_rules(
        pair(),
        vec![item("s", "s", EvidenceClass::Agree, Some(10_000))],
        &[rule("s", "strong", "s")],
        false,
        None,
    );
    let mut edges = vec![strong, identity];
    sort_automatic_edges(&mut edges).unwrap();
    assert_eq!(edges[0].result, PairResult::AutomaticIdentity);
}

#[test]
fn evidence_and_tied_edges_are_stable_under_input_reordering() {
    let rules = vec![rule("a", "strong", "a"), rule("b", "strong", "b")];
    let evidence = vec![
        item("b", "b", EvidenceClass::Agree, Some(8_000)),
        item("a", "a", EvidenceClass::Agree, Some(9_000)),
    ];
    let forward = decide_pair_for_rules(pair(), evidence.clone(), &rules, false, None);
    let backward = decide_pair_for_rules(
        pair(),
        evidence.into_iter().rev().collect(),
        &rules,
        false,
        None,
    );
    assert_eq!(forward, backward);
    assert_eq!(forward.evidence[0].rule, "a");

    let mut left = pair();
    left.left_source_record_id = Uuid::from_bytes([1; 16]);
    left.right_source_record_id = Uuid::from_bytes([4; 16]);
    left.left_sort_key = vec![1];
    left.right_sort_key = vec![4];
    let mut right = pair();
    right.left_source_record_id = Uuid::from_bytes([2; 16]);
    right.right_source_record_id = Uuid::from_bytes([3; 16]);
    right.left_sort_key = vec![2];
    right.right_sort_key = vec![3];
    let one_rule = [rule("strong", "strong", "group")];
    let item = [item("strong", "group", EvidenceClass::Agree, Some(9_000))];
    let first = decide_pair_for_rules(left, item.to_vec(), &one_rule, false, None);
    let second = decide_pair_for_rules(right, item.to_vec(), &one_rule, false, None);
    let mut forward = vec![first.clone(), second.clone()];
    let mut backward = vec![second, first];
    sort_automatic_edges(&mut forward).unwrap();
    sort_automatic_edges(&mut backward).unwrap();
    assert_eq!(forward, backward);
    assert_eq!(forward[0].pair.left_sort_key, vec![1]);
}
