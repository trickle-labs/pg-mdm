use std::collections::{BTreeMap, BTreeSet};

use pg_mdm::candidate::CandidatePair;
use pg_mdm::definition::{Entity, GoldenValue, Source};
use pg_mdm::evaluation::{
    EvaluationInput, EvaluationRecord, EvaluationResult, GoldenRow, resolve_and_compare,
    semantic_golden, semantic_identity, semantic_resolution_facts, semantic_reviews,
};
use pg_mdm::identity::IdentityState;
use pg_mdm::normalization::NormalizedState;
use pg_mdm::pair::{PairDecision, PairResult};
use pg_mdm::resolver::{ResolverInput, ResolverLimits, ResolverRecord, resolve};
use pg_mdm::review::Review;
use pgrx::Uuid;
use proptest::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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

fn entity() -> Entity {
    Entity {
        name: "property".into(),
        sources: vec![Source {
            name: "source".into(),
            relation: "public.source".into(),
            source_id: vec!["id".into()],
            mode: "tracked".into(),
            fields: BTreeMap::new(),
            row_changed_at: None,
            soft_delete_when: None,
            authority: BTreeMap::new(),
        }],
        fields: Vec::new(),
        matches: Vec::new(),
        golden_values: vec![GoldenValue {
            field: "name".into(),
            policy: "prefer_source".into(),
            sources: Some(vec!["source".into()]),
        }],
        preset: None,
        limits: BTreeMap::new(),
        execution_role: None,
    }
}

fn records() -> Vec<EvaluationRecord> {
    (1..=4)
        .map(|value| EvaluationRecord {
            source_record_id: id(value),
            source_name: "source".into(),
            source_sort_key: vec![value],
        })
        .collect()
}

fn golden_rows() -> Vec<GoldenRow> {
    (1..=4)
        .map(|value| GoldenRow {
            source_record_id: id(value),
            source_name: "source".into(),
            field: "name".into(),
            raw_value: Some(json!(format!("record-{value}"))),
            row_changed_at: Some(i64::from(value)),
            state: NormalizedState::Value,
            normalized: Some(format!("record-{value}")),
            canonical_bytes: Some(vec![value]),
        })
        .collect()
}

fn projection(identity: &IdentityState, golden: Value, reviews: &[Review], facts: Value) -> Value {
    json!({
        "identity": semantic_identity(identity),
        "golden": golden,
        "reviews": semantic_reviews(reviews),
        "resolution_facts": facts,
    })
}

#[allow(clippy::too_many_arguments)]
fn evaluate(
    entity: &Entity,
    records: &[EvaluationRecord],
    pairs: Vec<PairDecision>,
    old_identity: &IdentityState,
    old_reviews: &[Review],
    old_golden: &Value,
    old_facts: &Value,
    golden_rows: &[GoldenRow],
    revision: i64,
) -> EvaluationResult {
    let mut ordinal = 0_u8;
    let mut allocator = || {
        ordinal += 1;
        let mut bytes = [0; 16];
        bytes[0] = 0xff;
        bytes[1] = revision as u8;
        bytes[2] = ordinal;
        Uuid::from_bytes(bytes)
    };
    resolve_and_compare(EvaluationInput {
        entity,
        definition_version: 1,
        publication_revision: revision,
        records,
        manual_matches: Vec::new(),
        cannot_links: Vec::new(),
        pair_decisions: pairs,
        limits: ResolverLimits::default(),
        old_identity,
        old_reviews,
        old_golden,
        old_resolution_facts: old_facts,
        golden_rows,
        overrides: &BTreeMap::new(),
        allocator: &mut allocator,
    })
    .unwrap()
}

fn selected_mdm_ids(identity: &IdentityState, selected: &BTreeSet<Uuid>) -> BTreeSet<Uuid> {
    identity
        .memberships
        .iter()
        .filter(|row| selected.contains(&row.source_record_id))
        .map(|row| row.mdm_id)
        .collect()
}

fn scope_identity(identity: &IdentityState, selected: &BTreeSet<Uuid>) -> IdentityState {
    let mdm_ids = selected_mdm_ids(identity, selected);
    IdentityState {
        registry: identity
            .registry
            .iter()
            .filter(|row| mdm_ids.contains(&row.mdm_id))
            .cloned()
            .collect(),
        memberships: identity
            .memberships
            .iter()
            .filter(|row| selected.contains(&row.source_record_id))
            .cloned()
            .collect(),
        aliases: Vec::new(),
        splits: Vec::new(),
    }
}

fn mentions(value: &Value, ids: &BTreeSet<String>) -> bool {
    match value {
        Value::String(value) => ids.contains(value),
        Value::Array(values) => values.iter().any(|value| mentions(value, ids)),
        Value::Object(values) => values.values().any(|value| mentions(value, ids)),
        _ => false,
    }
}

fn scope_reviews(
    reviews: &[Review],
    selected: &BTreeSet<Uuid>,
    mdm_ids: &BTreeSet<Uuid>,
) -> Vec<Review> {
    let ids = selected
        .iter()
        .chain(mdm_ids)
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    reviews
        .iter()
        .filter(|review| mentions(&review.subjects, &ids))
        .cloned()
        .collect()
}

fn scope_golden(value: &Value, mdm_ids: &BTreeSet<Uuid>) -> Value {
    let mdm_ids = mdm_ids
        .iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    Value::Array(
        value
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row[0].as_str().is_some_and(|value| mdm_ids.contains(value)))
            .cloned()
            .collect(),
    )
}

fn fact_in_scope(row: &Value, selected: &BTreeSet<Uuid>) -> bool {
    let selected = selected
        .iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    row[0]
        .as_str()
        .and_then(|key| key.split_once(':'))
        .is_some_and(|(left, right)| selected.contains(left) && selected.contains(right))
}

fn scope_facts(value: &Value, selected: &BTreeSet<Uuid>) -> Value {
    Value::Array(
        value
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| fact_in_scope(row, selected))
            .cloned()
            .collect(),
    )
}

fn splice_identity(
    old: &IdentityState,
    selected: &BTreeSet<Uuid>,
    scoped: &IdentityState,
) -> IdentityState {
    let replaced = selected_mdm_ids(old, selected);
    let mut registry = old
        .registry
        .iter()
        .filter(|row| !replaced.contains(&row.mdm_id))
        .map(|row| (row.mdm_id, row.clone()))
        .collect::<BTreeMap<_, _>>();
    registry.extend(scoped.registry.iter().map(|row| (row.mdm_id, row.clone())));
    let mut aliases = old.aliases.clone();
    for row in &scoped.aliases {
        if !aliases.contains(row) {
            aliases.push(row.clone());
        }
    }
    let mut splits = old.splits.clone();
    for row in &scoped.splits {
        if !splits.contains(row) {
            splits.push(row.clone());
        }
    }
    IdentityState {
        registry: registry.into_values().collect(),
        memberships: old
            .memberships
            .iter()
            .filter(|row| !selected.contains(&row.source_record_id))
            .cloned()
            .chain(scoped.memberships.iter().cloned())
            .collect(),
        aliases,
        splits,
    }
}

fn splice_json(old: &Value, scoped: Value, replace: impl Fn(&Value) -> bool) -> Value {
    let mut rows = old
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| !replace(row))
        .cloned()
        .chain(scoped.as_array().unwrap().iter().cloned())
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| serde_json::to_string(row).unwrap());
    Value::Array(rows)
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

proptest! {
    #[test]
    fn affected_splice_matches_full_evaluation_for_generated_edge_histories(
        history in prop::collection::vec(any::<u8>(), 1..24),
    ) {
        let entity = entity();
        let records = records();
        let golden_rows = golden_rows();
        let mut old_identity = IdentityState::default();
        let mut old_reviews = Vec::new();
        let mut old_golden = json!([]);
        let mut old_facts = json!([]);
        let mut revision = 1;
        let initial = evaluate(
            &entity,
            &records,
            Vec::new(),
            &old_identity,
            &old_reviews,
            &old_golden,
            &old_facts,
            &golden_rows,
            revision,
        );
        old_identity = initial.identity;
        old_reviews = initial.reviews;
        old_golden = semantic_golden(&initial.golden);
        old_facts = semantic_resolution_facts(&initial.resolution);
        let endpoints = [(1, 2), (1, 3), (1, 4), (2, 3), (2, 4), (3, 4)];
        let mut previous = 0_u8;

        for mask in history.into_iter().map(|value| value & 0x3f) {
            if mask == previous {
                continue;
            }
            revision += 1;
            let pairs = endpoints
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(index, &(left, right))| edge(left, right, &format!("group-{index}"), index as u8))
                .collect::<Vec<_>>();
            let seeds = endpoints
                .iter()
                .enumerate()
                .filter(|(index, _)| (mask ^ previous) & (1 << index) != 0)
                .flat_map(|(_, &(left, right))| [id(left), id(right)])
                .collect::<BTreeSet<_>>();
            let full = evaluate(
                &entity,
                &records,
                pairs.clone(),
                &old_identity,
                &old_reviews,
                &old_golden,
                &old_facts,
                &golden_rows,
                revision,
            );
            let selected = pg_mdm::affected::build(
                seeds,
                &old_identity.memberships,
                &[],
                &pairs,
            )
            .unwrap()
            .into_records();
            let old_mdm_ids = selected_mdm_ids(&old_identity, &selected);
            let scoped_identity = scope_identity(&old_identity, &selected);
            let scoped_reviews = scope_reviews(&old_reviews, &selected, &old_mdm_ids);
            let scoped_golden = scope_golden(&old_golden, &old_mdm_ids);
            let scoped_facts = scope_facts(&old_facts, &selected);
            let scoped_records = records
                .iter()
                .filter(|row| selected.contains(&row.source_record_id))
                .cloned()
                .collect::<Vec<_>>();
            let scoped_pairs = pairs
                .into_iter()
                .filter(|pair| {
                    selected.contains(&pair.pair.left_source_record_id)
                        && selected.contains(&pair.pair.right_source_record_id)
                })
                .collect();
            let scoped_golden_rows = golden_rows
                .iter()
                .filter(|row| selected.contains(&row.source_record_id))
                .cloned()
                .collect::<Vec<_>>();
            let affected = evaluate(
                &entity,
                &scoped_records,
                scoped_pairs,
                &scoped_identity,
                &scoped_reviews,
                &scoped_golden,
                &scoped_facts,
                &scoped_golden_rows,
                revision,
            );
            let merged_identity = splice_identity(&old_identity, &selected, &affected.identity);
            let mut replacement_mdm_ids = old_mdm_ids;
            replacement_mdm_ids.extend(affected.identity.registry.iter().map(|row| row.mdm_id));
            let replacement_mdm_id_strings = replacement_mdm_ids
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>();
            let merged_golden = splice_json(
                &old_golden,
                semantic_golden(&affected.golden),
                |row| {
                    row[0]
                        .as_str()
                        .is_some_and(|value| replacement_mdm_id_strings.contains(value))
                },
            );
            let review_ids = selected
                .iter()
                .chain(&replacement_mdm_ids)
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>();
            let merged_reviews = old_reviews
                .iter()
                .filter(|review| !mentions(&review.subjects, &review_ids))
                .cloned()
                .chain(affected.reviews.iter().cloned())
                .collect::<Vec<_>>();
            let merged_facts = splice_json(
                &old_facts,
                semantic_resolution_facts(&affected.resolution),
                |row| fact_in_scope(row, &selected),
            );
            let full_projection = projection(
                &full.identity,
                semantic_golden(&full.golden),
                &full.reviews,
                semantic_resolution_facts(&full.resolution),
            );
            let affected_projection = projection(
                &merged_identity,
                merged_golden,
                &merged_reviews,
                merged_facts,
            );
            prop_assert_eq!(&affected_projection, &full_projection);
            prop_assert_eq!(
                Sha256::digest(serde_json::to_vec(&affected_projection).unwrap()),
                Sha256::digest(serde_json::to_vec(&full_projection).unwrap()),
            );

            old_identity = full.identity;
            old_reviews = full.reviews;
            old_golden = semantic_golden(&full.golden);
            old_facts = semantic_resolution_facts(&full.resolution);
            previous = mask;
        }
    }
}
