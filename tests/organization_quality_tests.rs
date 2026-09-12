use std::collections::{BTreeMap, BTreeSet};

use pg_mdm::candidate::{self, CandidateLimits, CandidatePlan, NormalizedRecord};
use pg_mdm::constraint::{DecisionEdge, DecisionKind};
use pg_mdm::definition::MatchRule;
use pg_mdm::evidence::{self, NormalizedEvidenceValue};
use pg_mdm::normalization::{
    LogicalType, NormalizedState, make_canonical_bytes, normalize_text_pure,
};
use pg_mdm::pair::{self, PairDecision, PairResult};
use pg_mdm::resolver::{self, Resolution, ResolverInput, ResolverLimits, ResolverRecord};
use pgrx::Uuid;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const FIXTURE: &str = include_str!("fixtures/organization_domain_v1.json");

#[derive(Deserialize)]
struct Fixture {
    format_version: u64,
    metadata: Value,
    candidate_plan: Value,
    field_cleaners: BTreeMap<String, String>,
    match_rules: Vec<MatchRule>,
    partitions: BTreeMap<String, Vec<Record>>,
    cannot_link_pairs: Vec<[String; 2]>,
    safety_cases: Vec<SafetyCase>,
    lifecycle_checks: Vec<LifecycleCheck>,
}

#[derive(Clone, Deserialize)]
struct Record {
    id: String,
    source: String,
    label: Option<String>,
    fields: BTreeMap<String, String>,
    authority: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct SafetyCase {
    partition: String,
    case_id: String,
    kind: String,
    bridge_pairs: Option<Vec<[String; 2]>>,
    must_not_merge: Vec<[String; 2]>,
}

#[derive(Deserialize)]
struct LifecycleCheck {
    partition: String,
    case_id: String,
    deleted_record_id: String,
    expected_reactivated_label: String,
}

struct Run {
    candidate_pairs: Vec<candidate::CandidatePair>,
    pair_decisions: Vec<PairDecision>,
    resolution: Resolution,
}

fn id(value: &str) -> Uuid {
    let hex = value.replace('-', "");
    let bytes = (0..16)
        .map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap())
        .collect::<Vec<_>>();
    Uuid::from_slice(&bytes).expect("fixture UUID")
}

fn pair_key(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_owned(), right.to_owned())
    } else {
        (right.to_owned(), left.to_owned())
    }
}

fn authority_conflict(left: &Record, right: &Record) -> bool {
    left.authority.iter().any(|(field, left_value)| {
        right
            .authority
            .get(field)
            .is_some_and(|right_value| left_value != right_value)
    })
}

fn run(fixture: &Fixture, records: &[Record], inactive: &BTreeSet<String>) -> Run {
    let plan = CandidatePlan::from_json(&fixture.candidate_plan).expect("valid candidate plan");
    let records = records
        .iter()
        .filter(|record| !inactive.contains(&record.id))
        .collect::<Vec<_>>();
    let mut normalized = Vec::<NormalizedRecord>::new();
    let mut evidence_values = Vec::<NormalizedEvidenceValue>::new();
    let mut resolver_records = Vec::<ResolverRecord>::new();
    let mut records_by_id = BTreeMap::new();
    for (ordinal, record) in records.iter().enumerate() {
        let record_id = id(&record.id);
        let sort_key = (ordinal as u64).to_be_bytes().to_vec();
        records_by_id.insert(record.id.clone(), *record);
        let mut authority = record.authority.clone();
        resolver_records.push(ResolverRecord {
            source_record_id: record_id,
            source_sort_key: sort_key.clone(),
            authority: std::mem::take(&mut authority),
        });
        for (field, raw) in &record.fields {
            let cleaner = fixture
                .field_cleaners
                .get(field)
                .map(String::as_str)
                .unwrap_or("text");
            let value = normalize_text_pure(Some(raw), cleaner, 1, "present", &json!({}))
                .expect("fixture field normalizes");
            let Some(normalized_value) = value.normalized else {
                continue;
            };
            let canonical_bytes = value
                .canonical_bytes
                .unwrap_or_else(|| make_canonical_bytes(LogicalType::Text, &normalized_value));
            normalized.push(NormalizedRecord {
                source_record_id: record_id,
                source_sort_key: sort_key.clone(),
                field: field.clone(),
                normalized: normalized_value.clone(),
                canonical_bytes: canonical_bytes.clone(),
            });
            evidence_values.push(NormalizedEvidenceValue {
                source_record_id: record_id,
                field: field.clone(),
                state: NormalizedState::Value,
                normalized: Some(normalized_value),
                canonical_bytes: Some(canonical_bytes),
            });
        }
    }
    let candidate_pairs = candidate::generate_candidates(
        &plan,
        &normalized,
        CandidateLimits {
            max_block_records: 100,
            max_candidate_pairs: 10_000,
        },
    )
    .expect("fixture candidates generate");
    let pair_decisions = candidate_pairs
        .iter()
        .map(|pair| {
            let left = records_by_id
                .values()
                .find(|record| id(&record.id) == pair.left_source_record_id)
                .expect("left endpoint exists");
            let right = records_by_id
                .values()
                .find(|record| id(&record.id) == pair.right_source_record_id)
                .expect("right endpoint exists");
            let evidence =
                evidence::evaluate_pair(pair, &fixture.match_rules, &evidence_values, 10_000)
                    .expect("fixture evidence evaluates");
            pair::decide_pair_for_rules(
                pair.clone(),
                evidence,
                &fixture.match_rules,
                authority_conflict(left, right),
                None,
            )
        })
        .collect::<Vec<_>>();
    let cannot_links = fixture
        .cannot_link_pairs
        .iter()
        .filter(|pair| records_by_id.contains_key(&pair[0]) && records_by_id.contains_key(&pair[1]))
        .enumerate()
        .map(|(index, pair)| DecisionEdge {
            decision_id: Uuid::from_bytes([240 + index as u8; 16]),
            left_source_record_id: id(&pair[0]),
            right_source_record_id: id(&pair[1]),
            decision: DecisionKind::NotMatch,
        })
        .collect();
    let resolution = resolver::resolve(ResolverInput {
        records: resolver_records,
        manual_matches: Vec::new(),
        cannot_links,
        pair_decisions: pair_decisions.clone(),
        limits: ResolverLimits::default(),
    })
    .expect("fixture resolves");
    Run {
        candidate_pairs,
        pair_decisions,
        resolution,
    }
}

fn clusters(run: &Run) -> BTreeMap<Vec<u8>, BTreeSet<Uuid>> {
    let mut result = BTreeMap::new();
    for membership in &run.resolution.memberships {
        result
            .entry(membership.component_key.clone())
            .or_insert_with(BTreeSet::new)
            .insert(membership.source_record_id);
    }
    result
}

fn same_cluster(run: &Run, left: &str, right: &str) -> bool {
    let left = id(left);
    let right = id(right);
    clusters(run)
        .values()
        .any(|members| members.contains(&left) && members.contains(&right))
}

fn partition_metrics(records: &[Record], run: &Run) -> Value {
    let truth = records
        .iter()
        .filter_map(|record| record.label.as_ref().map(|label| (id(&record.id), label)))
        .collect::<BTreeMap<_, _>>();
    let candidates = run
        .candidate_pairs
        .iter()
        .map(|pair| {
            pair_key(
                &pair.left_source_record_id.to_string(),
                &pair.right_source_record_id.to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut expected = BTreeSet::new();
    let rows = records
        .iter()
        .filter(|record| record.label.is_some())
        .collect::<Vec<_>>();
    for (index, left) in rows.iter().enumerate() {
        for right in rows.iter().skip(index + 1) {
            if left.label == right.label {
                expected.insert(pair_key(&left.id, &right.id));
            }
        }
    }
    let candidate_matches = expected.intersection(&candidates).count();
    let predicted = clusters(run);
    let mut predicted_pairs = BTreeSet::new();
    for members in predicted.values() {
        let values = members.iter().map(ToString::to_string).collect::<Vec<_>>();
        for (index, left) in values.iter().enumerate() {
            for right in values.iter().skip(index + 1) {
                predicted_pairs.insert(pair_key(left, right));
            }
        }
    }
    let mut false_merges = 0;
    let mut missed_matches = 0;
    for pair in &predicted_pairs {
        if let (Some(left), Some(right)) = (truth.get(&id(&pair.0)), truth.get(&id(&pair.1)))
            && left != right
        {
            false_merges += 1;
        }
    }
    for pair in &expected {
        if !predicted_pairs.contains(pair) {
            missed_matches += 1;
        }
    }
    let mut reason_distribution = BTreeMap::<String, usize>::new();
    for decision in &run.pair_decisions {
        if decision.result == PairResult::Review {
            *reason_distribution
                .entry("SUPPORTING_ONLY".into())
                .or_default() += 1;
        }
    }
    for fact in &run.resolution.rejected {
        *reason_distribution
            .entry(fact.reason_code.clone())
            .or_default() += 1;
    }
    let review_count = run
        .pair_decisions
        .iter()
        .filter(|decision| decision.result == PairResult::Review)
        .count();
    let source_counts =
        records
            .iter()
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, record| {
                *counts.entry(&record.source).or_default() += 1;
                counts
            });
    json!({
        "records": records.len(),
        "sources": source_counts,
        "candidate_pairs": run.candidate_pairs.len(),
        "labeled_positive_pairs": expected.len(),
        "candidate_positive_pairs": candidate_matches,
        "candidate_recall": if expected.is_empty() { 1.0 } else { candidate_matches as f64 / expected.len() as f64 },
        "false_merges": false_merges,
        "missed_matches": missed_matches,
        "cluster_errors": false_merges + missed_matches,
        "review_count": review_count,
        "review_reason_distribution": reason_distribution
    })
}

#[test]
fn organization_fixture_report_uses_production_candidate_and_resolution_paths() {
    let fixture: Fixture = serde_json::from_str(FIXTURE).expect("organization fixture parses");
    assert_eq!(fixture.format_version, 1);
    assert_eq!(
        fixture.metadata["thresholds"]["approval_status"],
        "pending release-owner sign-off"
    );

    let mut partition_reports = serde_json::Map::new();
    let mut runs = BTreeMap::new();
    for (name, records) in &fixture.partitions {
        let current = run(&fixture, records, &BTreeSet::new());
        partition_reports.insert(name.clone(), partition_metrics(records, &current));
        runs.insert(name.clone(), current);
    }

    let mut safety_report = Vec::new();
    let mut safety_false_merges = 0usize;
    for case in &fixture.safety_cases {
        let current = runs.get(&case.partition).expect("safety partition exists");
        let false_merges = case
            .must_not_merge
            .iter()
            .filter(|pair| same_cluster(current, &pair[0], &pair[1]))
            .count();
        safety_false_merges += false_merges;
        match case.kind.as_str() {
            "shared_contact" => assert!(
                current.pair_decisions.iter().any(|decision| {
                    pair_key(
                        &decision.pair.left_source_record_id.to_string(),
                        &decision.pair.right_source_record_id.to_string(),
                    ) == pair_key(&case.must_not_merge[0][0], &case.must_not_merge[0][1])
                        && decision.result == PairResult::Review
                }),
                "{} must remain a review",
                case.case_id
            ),
            "authoritative_conflict" => assert!(
                current.pair_decisions.iter().any(|decision| {
                    pair_key(
                        &decision.pair.left_source_record_id.to_string(),
                        &decision.pair.right_source_record_id.to_string(),
                    ) == pair_key(&case.must_not_merge[0][0], &case.must_not_merge[0][1])
                        && decision.result == PairResult::AuthoritativeConflict
                }),
                "{} must retain the authority conflict",
                case.case_id
            ),
            "cannot_link" => assert!(
                current.resolution.rejected.iter().any(|fact| {
                    fact.reason_code == "CANNOT_LINK"
                        && pair_key(
                            &fact.edge.left_source_record_id.to_string(),
                            &fact.edge.right_source_record_id.to_string(),
                        ) == pair_key(&case.must_not_merge[0][0], &case.must_not_merge[0][1])
                }),
                "{} must be rejected by the cannot-link",
                case.case_id
            ),
            "business_context" => assert!(
                current.pair_decisions.iter().any(|decision| {
                    pair_key(
                        &decision.pair.left_source_record_id.to_string(),
                        &decision.pair.right_source_record_id.to_string(),
                    ) == pair_key(&case.must_not_merge[0][0], &case.must_not_merge[0][1])
                        && decision.result != PairResult::AutomaticIdentity
                }),
                "{} must not treat an unscoped identifier as identity",
                case.case_id
            ),
            "weak_chain" => {
                let bridge = case
                    .bridge_pairs
                    .as_ref()
                    .expect("weak chain has labeled bridge pairs");
                for pair in bridge {
                    assert!(
                        current.candidate_pairs.iter().any(|candidate| {
                            pair_key(
                                &candidate.left_source_record_id.to_string(),
                                &candidate.right_source_record_id.to_string(),
                            ) == pair_key(&pair[0], &pair[1])
                        }),
                        "{} is missing candidate bridge {}-{}",
                        case.case_id,
                        pair[0],
                        pair[1]
                    );
                }
            }
            other => panic!("unknown safety case kind {other}"),
        }
        safety_report.push(json!({
            "partition": case.partition,
            "case_id": case.case_id,
            "kind": case.kind,
            "false_merges": false_merges
        }));
    }
    assert_eq!(
        safety_false_merges, 0,
        "protected safety cases must have zero false merges: {safety_report:?}"
    );

    let mut lifecycle_report = Vec::new();
    for lifecycle in &fixture.lifecycle_checks {
        let records = fixture
            .partitions
            .get(&lifecycle.partition)
            .expect("lifecycle partition exists");
        let inactive = BTreeSet::from([lifecycle.deleted_record_id.clone()]);
        let deleted = run(&fixture, records, &inactive);
        assert!(
            !deleted
                .resolution
                .memberships
                .iter()
                .any(|membership| membership.source_record_id == id(&lifecycle.deleted_record_id))
        );
        let reactivated = run(&fixture, records, &BTreeSet::new());
        let expected_ids = records
            .iter()
            .filter(|record| record.label.as_deref() == Some(&lifecycle.expected_reactivated_label))
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>();
        assert!(expected_ids.len() >= 2);
        assert!(expected_ids.iter().skip(1).all(|id_value| same_cluster(
            &reactivated,
            expected_ids[0],
            id_value
        )));
        lifecycle_report.push(json!({
            "case_id": lifecycle.case_id,
            "after_delete_active_records": deleted.resolution.memberships.len(),
            "after_reactivation_records": expected_ids.len(),
            "reactivated_entity": lifecycle.expected_reactivated_label
        }));
    }

    let fixture_digest = Sha256::digest(FIXTURE.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let report = json!({
        "format_version": fixture.format_version,
        "fixture_sha256": fixture_digest,
        "thresholds_frozen": false,
        "partitions": partition_reports,
        "safety_cases": safety_report,
        "safety_false_merges": safety_false_merges,
        "lifecycle_checks": lifecycle_report
    });
    println!(
        "organization fixture report: {}",
        serde_json::to_string_pretty(&report).unwrap()
    );
}
