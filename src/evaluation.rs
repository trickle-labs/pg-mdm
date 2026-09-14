use std::collections::{BTreeMap, BTreeSet};

use pgrx::Uuid;
use serde_json::{Value, json};

use crate::constraint::DecisionEdge;
use crate::definition::Entity;
use crate::error::MdmError;
use crate::golden::{GoldenCandidate, GoldenOverride, GoldenSelection};
use crate::identity::{self, IdAllocator, IdentityState};
use crate::normalization::NormalizedState;
use crate::pair::PairDecision;
use crate::resolver::{self, Resolution, ResolverInput, ResolverLimits, ResolverRecord};
use crate::review::{Review, ReviewCandidate, ReviewStatus, Subject};

#[derive(Clone, Debug)]
pub(crate) struct EvaluationRecord {
    pub source_record_id: Uuid,
    pub source_name: String,
    pub source_sort_key: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct GoldenRow {
    pub source_record_id: Uuid,
    pub source_name: String,
    pub field: String,
    pub raw_value: Option<Value>,
    pub row_changed_at: Option<i64>,
    pub state: NormalizedState,
    pub normalized: Option<String>,
    pub canonical_bytes: Option<Vec<u8>>,
}

pub(crate) struct EvaluationResult {
    pub resolution: Resolution,
    pub identity: IdentityState,
    pub golden: BTreeMap<(Uuid, String), GoldenSelection>,
    pub reviews: Vec<Review>,
    pub changed: bool,
}

pub(crate) struct EvaluationInput<'a> {
    pub entity: &'a Entity,
    pub definition_version: i64,
    pub publication_revision: i64,
    pub records: &'a [EvaluationRecord],
    pub manual_matches: Vec<DecisionEdge>,
    pub cannot_links: Vec<DecisionEdge>,
    pub pair_decisions: Vec<PairDecision>,
    pub limits: ResolverLimits,
    pub old_identity: &'a IdentityState,
    pub old_reviews: &'a [Review],
    pub old_golden: &'a Value,
    pub old_resolution_facts: &'a Value,
    pub golden_rows: &'a [GoldenRow],
    pub overrides: &'a BTreeMap<String, Vec<GoldenOverride>>,
    pub allocator: &'a mut dyn IdAllocator,
}

pub(crate) fn source_authority(entity: &Entity, source_name: &str) -> BTreeMap<String, String> {
    entity
        .sources
        .iter()
        .find(|source| source.name == source_name)
        .map(|source| {
            source
                .authority
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.into())))
                .collect()
        })
        .unwrap_or_default()
}

fn source_priority(entity: &Entity, source_name: &str) -> u32 {
    entity
        .sources
        .iter()
        .position(|source| source.name == source_name)
        .unwrap_or(entity.sources.len()) as u32
}

pub(crate) fn select_golden(
    entity: &Entity,
    memberships: &IdentityState,
    rows: &[GoldenRow],
    overrides: &BTreeMap<String, Vec<GoldenOverride>>,
) -> Result<BTreeMap<(Uuid, String), GoldenSelection>, MdmError> {
    let by_record = memberships
        .memberships
        .iter()
        .filter(|membership| membership.active)
        .map(|membership| (membership.source_record_id, membership.mdm_id))
        .collect::<BTreeMap<_, _>>();
    let mut candidates: BTreeMap<(Uuid, String), Vec<GoldenCandidate>> = BTreeMap::new();
    for row in rows {
        let Some(&mdm_id) = by_record.get(&row.source_record_id) else {
            continue;
        };
        let Some(definition) = entity
            .golden_values
            .iter()
            .find(|definition| definition.field == row.field)
        else {
            continue;
        };
        if definition
            .sources
            .as_ref()
            .is_some_and(|sources| !sources.iter().any(|source| source == &row.source_name))
        {
            continue;
        }
        candidates
            .entry((mdm_id, row.field.clone()))
            .or_default()
            .push(GoldenCandidate {
                source_record_id: row.source_record_id,
                source_name: row.source_name.clone(),
                source_priority: source_priority(entity, &row.source_name),
                row_changed_at: row.row_changed_at,
                authoritative: source_authority(entity, &row.source_name).contains_key(&row.field),
                source_sort_key: memberships
                    .memberships
                    .iter()
                    .find(|membership| membership.source_record_id == row.source_record_id)
                    .map(|membership| membership.source_sort_key.clone())
                    .unwrap_or_default(),
                raw_value: row.raw_value.clone(),
                state: row.state,
                normalized: row.normalized.clone(),
                canonical_bytes: row.canonical_bytes.clone(),
            });
    }
    let mut result = BTreeMap::new();
    for definition in &entity.golden_values {
        for identity in memberships
            .registry
            .iter()
            .filter(|identity| identity.status == identity::IdentityStatus::Active)
        {
            let key = (identity.mdm_id, definition.field.clone());
            let selection = crate::golden::select_golden(
                &definition.policy,
                candidates.get(&key).map(Vec::as_slice).unwrap_or(&[]),
                overrides
                    .get(&definition.field)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            )?;
            result.insert(key, selection);
        }
    }
    Ok(result)
}

fn issue_candidates(
    definition_version: i64,
    rejected: &[crate::resolver::UnionFact],
    golden: &BTreeMap<(Uuid, String), GoldenSelection>,
) -> Vec<ReviewCandidate> {
    let mut result = rejected
        .iter()
        .map(|fact| {
            let subjects = vec![
                Subject::uuid("source_record", fact.edge.left_source_record_id),
                Subject::uuid("source_record", fact.edge.right_source_record_id),
            ];
            ReviewCandidate::new(
                definition_version,
                "warning",
                fact.reason_code.clone(),
                &subjects,
                json!({"left_source_record_id": fact.edge.left_source_record_id.to_string(), "right_source_record_id": fact.edge.right_source_record_id.to_string()}),
                json!({"reason_code": fact.reason_code, "evidence_groups": fact.evidence_groups}),
            )
        })
        .collect::<Vec<_>>();
    for ((mdm_id, field), selection) in golden {
        for issue in &selection.issues {
            let subject = Subject::new(
                "golden",
                [mdm_id.as_bytes().as_slice(), field.as_bytes()].concat(),
            );
            result.push(ReviewCandidate::new(
                definition_version,
                "warning",
                issue.as_str(),
                &[subject],
                json!({"mdm_id": mdm_id.to_string(), "field": field}),
                json!({"issue": issue.as_str()}),
            ));
        }
    }
    result
}

pub(crate) fn semantic_identity(state: &IdentityState) -> Value {
    let mut registry = state.registry.iter().collect::<Vec<_>>();
    registry.sort_by_key(|row| row.mdm_id);
    let mut memberships = state.memberships.iter().collect::<Vec<_>>();
    memberships.sort_by(|left, right| {
        left.source_sort_key
            .cmp(&right.source_sort_key)
            .then_with(|| left.source_record_id.cmp(&right.source_record_id))
    });
    let mut aliases = state.aliases.iter().collect::<Vec<_>>();
    aliases.sort_by_key(|row| {
        (
            row.alias_mdm_id,
            row.publication_revision,
            row.canonical_mdm_id,
        )
    });
    let mut splits = state.splits.iter().collect::<Vec<_>>();
    splits.sort_by_key(|row| {
        (
            row.parent_mdm_id,
            row.publication_revision,
            row.child_mdm_id,
        )
    });
    json!({
        "registry": registry.iter().map(|row| json!([row.mdm_id.to_string(), row.status])).collect::<Vec<_>>(),
        "memberships": memberships.iter().map(|row| json!([row.source_record_id.to_string(), row.source_sort_key, row.mdm_id.to_string(), row.active, row.first_membership_revision, row.last_membership_revision, row.membership_reason])).collect::<Vec<_>>(),
        "aliases": aliases.iter().map(|row| json!([row.alias_mdm_id.to_string(), row.canonical_mdm_id.to_string()])).collect::<Vec<_>>(),
        "splits": splits.iter().map(|row| json!([row.parent_mdm_id.to_string(), row.child_mdm_id.to_string()])).collect::<Vec<_>>()
    })
}

pub(crate) fn semantic_reviews(reviews: &[Review]) -> Value {
    let mut ordered = reviews.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|row| (row.issue_key, row.occurrence));
    json!(
        ordered
            .into_iter()
            .map(|row| json!([
                row.issue_key,
                row.occurrence,
                match row.status {
                    ReviewStatus::Open => "open",
                    ReviewStatus::Resolved => "resolved",
                },
                row.severity,
                row.reason_code,
                row.subjects,
                row.masked_summary
            ]))
            .collect::<Vec<_>>()
    )
}

pub(crate) fn semantic_golden(golden: &BTreeMap<(Uuid, String), GoldenSelection>) -> Value {
    json!(
        golden
            .iter()
            .map(|((mdm_id, field), row)| json!([
                mdm_id.to_string(),
                field,
                row.value,
                row.normalized,
                row.status.as_str(),
                row.winning_source_record_id.map(|id| id.to_string()),
                row.policy,
                row.policy_version,
                row.tie_break,
                row.contributors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            ]))
            .collect::<Vec<_>>()
    )
}

pub(crate) fn semantic_resolution_facts(resolution: &Resolution) -> Value {
    let mut facts = resolution
        .accepted
        .iter()
        .map(|fact| (fact, "accepted"))
        .chain(resolution.rejected.iter().map(|fact| (fact, "rejected")))
        .map(|(fact, kind)| {
            (
                format!(
                    "{}:{}",
                    fact.edge.left_source_record_id, fact.edge.right_source_record_id
                ),
                kind.to_owned(),
                fact.reason_code.clone(),
                fact.evidence_groups.clone(),
            )
        })
        .collect::<Vec<_>>();
    facts.sort();
    json!(facts)
}

pub(crate) fn semantic_projection(
    identity: &IdentityState,
    golden: &BTreeMap<(Uuid, String), GoldenSelection>,
    reviews: &[Review],
    resolution: &Resolution,
) -> Value {
    json!({
        "identity": semantic_identity(identity),
        "golden": semantic_golden(golden),
        "reviews": semantic_reviews(reviews),
        "resolution_facts": semantic_resolution_facts(resolution)
    })
}

pub(crate) fn resolve_and_compare(
    input: EvaluationInput<'_>,
) -> Result<EvaluationResult, MdmError> {
    let EvaluationInput {
        entity,
        definition_version,
        publication_revision,
        records,
        manual_matches,
        cannot_links,
        pair_decisions,
        limits,
        old_identity,
        old_reviews,
        old_golden,
        old_resolution_facts,
        golden_rows,
        overrides,
        allocator,
    } = input;
    let active = records
        .iter()
        .map(|record| record.source_record_id)
        .collect::<BTreeSet<_>>();
    let resolver_records = records
        .iter()
        .map(|record| ResolverRecord {
            source_record_id: record.source_record_id,
            source_sort_key: record.source_sort_key.clone(),
            authority: source_authority(entity, &record.source_name),
        })
        .collect();
    let manual_matches = manual_matches
        .into_iter()
        .filter(|edge| {
            active.contains(&edge.left_source_record_id)
                && active.contains(&edge.right_source_record_id)
        })
        .collect();
    let cannot_links = cannot_links
        .into_iter()
        .filter(|edge| {
            active.contains(&edge.left_source_record_id)
                && active.contains(&edge.right_source_record_id)
        })
        .collect();
    let pair_decisions = pair_decisions
        .into_iter()
        .filter(|decision| {
            active.contains(&decision.pair.left_source_record_id)
                && active.contains(&decision.pair.right_source_record_id)
        })
        .collect();
    let resolution = resolver::resolve(ResolverInput {
        records: resolver_records,
        manual_matches,
        cannot_links,
        pair_decisions,
        limits,
    })?;
    let next_identity =
        identity::reconcile(old_identity, &resolution, publication_revision, allocator)?;
    let next_golden = select_golden(entity, &next_identity, golden_rows, overrides)?;
    let candidates = issue_candidates(definition_version, &resolution.rejected, &next_golden);
    let next_reviews =
        crate::review::reconcile(old_reviews, &candidates, publication_revision, allocator);
    let changed = publication_revision == 1
        || semantic_identity(old_identity) != semantic_identity(&next_identity)
        || semantic_reviews(old_reviews) != semantic_reviews(&next_reviews)
        || semantic_golden(&next_golden) != *old_golden
        || semantic_resolution_facts(&resolution) != *old_resolution_facts;
    Ok(EvaluationResult {
        resolution,
        identity: next_identity,
        golden: next_golden,
        reviews: next_reviews,
        changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::golden::GoldenStatus;
    use crate::identity::{IdentityMembership, IdentityRecord, IdentityStatus};
    use crate::resolver::{PairKey, UnionFact, UnionOutcome};

    fn fact(left: u8, right: u8, outcome: UnionOutcome, reason: &str) -> UnionFact {
        UnionFact {
            left_component_key: vec![left],
            right_component_key: vec![right],
            edge: PairKey {
                left_source_record_id: Uuid::from_bytes([left; 16]),
                right_source_record_id: Uuid::from_bytes([right; 16]),
                left_sort_key: vec![left],
                right_sort_key: vec![right],
            },
            outcome,
            reason_code: reason.into(),
            evidence_groups: vec!["name".into()],
        }
    }

    #[test]
    fn resolution_fact_projection_is_canonical_and_explanation_sensitive() {
        let accepted = fact(1, 2, UnionOutcome::Accepted, "ALREADY_CONNECTED");
        let rejected = fact(2, 3, UnionOutcome::Rejected, "CANNOT_LINK");
        let projected = semantic_resolution_facts(&Resolution {
            memberships: Vec::new(),
            accepted: vec![accepted.clone()],
            rejected: vec![rejected.clone()],
        });
        assert_eq!(
            projected,
            json!([
                [
                    "01010101-0101-0101-0101-010101010101:02020202-0202-0202-0202-020202020202",
                    "accepted",
                    "ALREADY_CONNECTED",
                    ["name"]
                ],
                [
                    "02020202-0202-0202-0202-020202020202:03030303-0303-0303-0303-030303030303",
                    "rejected",
                    "CANNOT_LINK",
                    ["name"]
                ]
            ])
        );
        assert_eq!(
            semantic_resolution_facts(&Resolution {
                memberships: Vec::new(),
                accepted: vec![accepted],
                rejected: vec![rejected],
            }),
            projected
        );
        assert_ne!(
            semantic_resolution_facts(&Resolution {
                memberships: Vec::new(),
                accepted: Vec::new(),
                rejected: vec![fact(2, 3, UnionOutcome::Rejected, "AUTHORITATIVE_CONFLICT")],
            }),
            json!([
                [
                    "01010101-0101-0101-0101-010101010101:02020202-0202-0202-0202-020202020202",
                    "accepted",
                    "ALREADY_CONNECTED",
                    ["name"]
                ],
                [
                    "02020202-0202-0202-0202-020202020202:03030303-0303-0303-0303-030303030303",
                    "rejected",
                    "CANNOT_LINK",
                    ["name"]
                ]
            ])
        );
    }

    #[test]
    fn publication_projection_keeps_provenance_and_ignores_bookkeeping() {
        let id = |value| Uuid::from_bytes([value; 16]);
        let identity = IdentityState {
            registry: vec![IdentityRecord {
                mdm_id: id(1),
                created_revision: 1,
                retired_revision: None,
                status: IdentityStatus::Active,
            }],
            memberships: vec![IdentityMembership {
                source_record_id: id(2),
                source_sort_key: vec![1],
                mdm_id: id(1),
                active: true,
                first_membership_revision: 1,
                last_membership_revision: 1,
                membership_reason: "new".into(),
                last_change_revision: 1,
            }],
            ..IdentityState::default()
        };
        let golden = |tie_break: &str| {
            BTreeMap::from([(
                (id(1), "name".into()),
                GoldenSelection {
                    value: Some(json!("Acme")),
                    normalized: Some("acme".into()),
                    canonical_bytes: Some(b"acme".to_vec()),
                    status: GoldenStatus::Selected,
                    winning_source_record_id: Some(id(2)),
                    policy: "priority".into(),
                    policy_version: 1,
                    tie_break: tie_break.into(),
                    contributors: vec![id(2)],
                    issues: Vec::new(),
                },
            )])
        };
        let review = Review {
            review_id: id(3),
            issue_key: [3; 32],
            occurrence: 1,
            status: ReviewStatus::Open,
            severity: "warning".into(),
            reason_code: "CANNOT_LINK".into(),
            subjects: json!([]),
            masked_summary: json!({}),
            opened_revision: 1,
            resolved_revision: None,
            last_change_revision: 1,
            concurrency_version: 1,
        };
        let resolution = Resolution {
            memberships: Vec::new(),
            accepted: vec![fact(1, 2, UnionOutcome::Accepted, "ALREADY_CONNECTED")],
            rejected: Vec::new(),
        };
        let expected_review_key = vec![3u8; 32];
        let baseline = semantic_projection(
            &identity,
            &golden("source_priority"),
            std::slice::from_ref(&review),
            &resolution,
        );
        assert_eq!(
            baseline,
            json!({
                "identity": {
                    "registry": [["01010101-0101-0101-0101-010101010101", "active"]],
                    "memberships": [["02020202-0202-0202-0202-020202020202", [1], "01010101-0101-0101-0101-010101010101", true, 1, 1, "new"]],
                    "aliases": [],
                    "splits": []
                },
                "golden": [["01010101-0101-0101-0101-010101010101", "name", "Acme", "acme", "selected", "02020202-0202-0202-0202-020202020202", "priority", 1, "source_priority", ["02020202-0202-0202-0202-020202020202"]]],
                "reviews": [[expected_review_key, 1, "open", "warning", "CANNOT_LINK", [], {}]],
                "resolution_facts": [["01010101-0101-0101-0101-010101010101:02020202-0202-0202-0202-020202020202", "accepted", "ALREADY_CONNECTED", ["name"]]]
            })
        );

        let mut bookkeeping_identity = identity.clone();
        bookkeeping_identity.registry[0].created_revision = 99;
        bookkeeping_identity.memberships[0].last_change_revision = 99;
        let mut bookkeeping_review = review.clone();
        bookkeeping_review.last_change_revision = 99;
        bookkeeping_review.concurrency_version = 99;
        assert_eq!(
            semantic_projection(
                &bookkeeping_identity,
                &golden("source_priority"),
                &[bookkeeping_review],
                &resolution,
            ),
            baseline
        );
        assert_ne!(
            semantic_projection(&identity, &golden("record_key"), &[review], &resolution),
            baseline
        );
    }
}
