use std::collections::BTreeMap;

use pg_mdm::candidate::CandidatePair;
use pg_mdm::constraint::DecisionKind;
use pg_mdm::decision::edge;
use pg_mdm::pair::{PairDecision, PairResult};
use pg_mdm::resolver::{
    CannotLink, ManualEdge, ResolverInput, ResolverLimits, ResolverRecord, resolve,
};
use pgrx::Uuid;

fn id(value: u8) -> Uuid {
    Uuid::from_bytes([value; 16])
}

fn records(count: u8) -> Vec<ResolverRecord> {
    (1..=count)
        .map(|value| ResolverRecord {
            source_record_id: id(value),
            source_sort_key: vec![value],
            authority: BTreeMap::new(),
        })
        .collect()
}

fn decision(left: u8, right: u8, result: PairResult, group: &str, sort: u8) -> PairDecision {
    PairDecision {
        pair: CandidatePair {
            left_source_record_id: id(left),
            right_source_record_id: id(right),
            left_sort_key: vec![left],
            right_sort_key: vec![right],
            discovery_channels: vec![group.into()],
        },
        result,
        evidence: Vec::new(),
        independent_agreeing_groups: vec![group.into()],
        sort_key: vec![sort],
        reason_codes: Vec::new(),
    }
}

fn input(
    records: Vec<ResolverRecord>,
    manual_matches: Vec<ManualEdge>,
    cannot_links: Vec<CannotLink>,
    pair_decisions: Vec<PairDecision>,
) -> ResolverInput {
    ResolverInput {
        records,
        manual_matches,
        cannot_links,
        pair_decisions,
        limits: ResolverLimits::default(),
    }
}

#[test]
fn admission_is_conservative_and_deterministic() {
    let result = resolve(input(
        records(3),
        Vec::new(),
        Vec::new(),
        vec![
            decision(1, 2, PairResult::AutomaticStrong, "email", 1),
            decision(2, 3, PairResult::AutomaticStrong, "email", 2),
        ],
    ))
    .unwrap();
    assert_eq!(result.memberships[0].component_key, vec![1]);
    assert_eq!(result.memberships[1].component_key, vec![1]);
    assert_eq!(result.memberships[2].component_key, vec![3]);
    assert_eq!(
        result.rejected[0].reason_code,
        "SINGLETON_NEEDS_INDEPENDENT_GROUP"
    );
}

#[test]
fn established_components_need_two_independent_pairs() {
    let result = resolve(input(
        records(4),
        Vec::new(),
        Vec::new(),
        vec![
            decision(1, 2, PairResult::AutomaticStrong, "a", 1),
            decision(3, 4, PairResult::AutomaticStrong, "b", 2),
            decision(1, 3, PairResult::AutomaticStrong, "c", 3),
            decision(2, 4, PairResult::AutomaticStrong, "d", 4),
        ],
    ))
    .unwrap();
    assert!(result.rejected.is_empty());
    assert!(
        result
            .memberships
            .iter()
            .all(|membership| membership.component_key == vec![1])
    );
}

#[test]
fn cannot_links_reject_automatic_edges_and_manual_contradictions_fail() {
    let not_match = edge(id(9), id(1), id(2), DecisionKind::NotMatch);
    let result = resolve(input(
        records(2),
        Vec::new(),
        vec![not_match.clone()],
        vec![decision(1, 2, PairResult::AutomaticIdentity, "id", 1)],
    ))
    .unwrap();
    assert_eq!(result.rejected[0].reason_code, "CANNOT_LINK");

    let manual = edge(id(8), id(1), id(2), DecisionKind::Match);
    let error = resolve(input(records(2), vec![manual], vec![not_match], Vec::new())).unwrap_err();
    assert_eq!(error.code(), "MDM_DECISION_CONTRADICTION");
}

#[test]
fn limits_fail_closed() {
    let limits = ResolverLimits {
        max_active_records: 1,
        ..ResolverLimits::default()
    };
    let mut request = input(records(2), Vec::new(), Vec::new(), Vec::new());
    request.limits = limits;
    let error = resolve(request).unwrap_err();
    assert_eq!(error.code(), "MDM_RESOLVER_LIMIT");
}

#[test]
fn authority_conflicts_are_component_level() {
    let mut records = records(2);
    records[0].authority.insert("tax_id".into(), "a".into());
    records[1].authority.insert("tax_id".into(), "b".into());
    let result = resolve(input(
        records,
        Vec::new(),
        Vec::new(),
        vec![decision(1, 2, PairResult::AutomaticIdentity, "id", 1)],
    ))
    .unwrap();
    assert_eq!(result.rejected[0].reason_code, "AUTHORITATIVE_CONFLICT");
}

#[test]
fn a_second_group_on_the_strong_bridge_is_independent_for_a_singleton() {
    let mut bridge = decision(2, 3, PairResult::AutomaticStrong, "strong", 2);
    bridge.independent_agreeing_groups.push("support".into());
    let result = resolve(input(
        records(3),
        Vec::new(),
        Vec::new(),
        vec![
            decision(1, 2, PairResult::AutomaticStrong, "first", 1),
            bridge,
        ],
    ))
    .unwrap();
    assert_eq!(result.rejected.len(), 0);
    assert!(
        result
            .memberships
            .iter()
            .all(|membership| membership.component_key == vec![1])
    );
}

#[test]
fn manual_matches_bypass_automatic_admission() {
    let manual = edge(id(8), id(1), id(2), DecisionKind::Match);
    let result = resolve(input(
        records(3),
        vec![manual],
        Vec::new(),
        vec![decision(2, 3, PairResult::AutomaticStrong, "email", 1)],
    ))
    .unwrap();
    assert_eq!(result.accepted[0].reason_code, "MANUAL_MATCH");
    assert_eq!(
        result.rejected[0].reason_code,
        "SINGLETON_NEEDS_INDEPENDENT_GROUP"
    );
    assert_eq!(result.memberships[0].component_key, vec![1]);
}
