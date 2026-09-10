use std::collections::BTreeMap;

use pg_mdm::candidate::CandidatePair;
use pg_mdm::pair::{PairDecision, PairResult};
use pg_mdm::resolver::{ResolverInput, ResolverLimits, ResolverRecord, resolve};
use pgrx::Uuid;

fn id(value: u8) -> Uuid {
    Uuid::from_bytes([value; 16])
}

fn edge(left: u8, right: u8, group: &str, sort: u8) -> PairDecision {
    PairDecision {
        pair: CandidatePair {
            left_source_record_id: id(left),
            right_source_record_id: id(right),
            left_sort_key: vec![left],
            right_sort_key: vec![right],
            discovery_channels: vec![group.into()],
        },
        result: PairResult::AutomaticStrong,
        evidence: Vec::new(),
        independent_agreeing_groups: vec![group.into()],
        sort_key: vec![sort],
        reason_codes: Vec::new(),
    }
}

fn permutations(values: &mut [u8], index: usize, output: &mut Vec<Vec<u8>>) {
    if index == values.len() {
        output.push(values.to_vec());
        return;
    }
    for next in index..values.len() {
        values.swap(index, next);
        permutations(values, index + 1, output);
        values.swap(index, next);
    }
}

#[test]
fn all_record_input_orders_have_one_serialized_result() {
    let mut orders = Vec::new();
    permutations(&mut [1, 2, 3, 4], 0, &mut orders);
    let decisions = vec![
        edge(2, 4, "d", 4),
        edge(1, 3, "c", 3),
        edge(3, 4, "b", 2),
        edge(1, 2, "a", 1),
    ];
    let expected = resolve(ResolverInput {
        records: vec![1, 2, 3, 4]
            .into_iter()
            .map(|value| ResolverRecord {
                source_record_id: id(value),
                source_sort_key: vec![value],
                authority: BTreeMap::new(),
            })
            .collect(),
        manual_matches: Vec::new(),
        cannot_links: Vec::new(),
        pair_decisions: decisions.clone(),
        limits: ResolverLimits::default(),
    })
    .unwrap();
    for order in orders {
        let result = resolve(ResolverInput {
            records: order
                .into_iter()
                .map(|value| ResolverRecord {
                    source_record_id: id(value),
                    source_sort_key: vec![value],
                    authority: BTreeMap::new(),
                })
                .collect(),
            manual_matches: Vec::new(),
            cannot_links: Vec::new(),
            pair_decisions: decisions.clone(),
            limits: ResolverLimits::default(),
        })
        .unwrap();
        assert_eq!(result, expected);
    }
}
