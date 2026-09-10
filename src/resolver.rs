use std::collections::{BTreeMap, BTreeSet};

use pgrx::Uuid;

use crate::constraint::{DecisionEdge, DecisionKind};
use crate::error::MdmError;
use crate::pair::{PairDecision, PairResult};
use crate::semantics;

pub mod admission;
pub mod component;
pub mod union_find;

#[cfg(test)]
pub mod oracle;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolverRecord {
    pub source_record_id: Uuid,
    pub source_sort_key: Vec<u8>,
    pub authority: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolverLimits {
    pub max_active_records: usize,
    pub max_automatic_edges: usize,
    pub max_records_per_component: usize,
    pub max_component_checks: usize,
}

impl Default for ResolverLimits {
    fn default() -> Self {
        Self {
            max_active_records: semantics::DEFAULT_MAX_ACTIVE_RECORDS,
            max_automatic_edges: semantics::DEFAULT_MAX_AUTOMATIC_EDGES,
            max_records_per_component: semantics::DEFAULT_MAX_RECORDS_PER_COMPONENT,
            max_component_checks: semantics::DEFAULT_MAX_COMPONENT_CHECKS,
        }
    }
}

impl ResolverLimits {
    pub fn validate(&self) -> Result<(), MdmError> {
        for (resource, value, ceiling) in [
            (
                "max_active_records",
                self.max_active_records,
                semantics::ABSOLUTE_MAX_ACTIVE_RECORDS,
            ),
            (
                "max_automatic_edges",
                self.max_automatic_edges,
                semantics::ABSOLUTE_MAX_AUTOMATIC_EDGES,
            ),
            (
                "max_records_per_component",
                self.max_records_per_component,
                semantics::ABSOLUTE_MAX_RECORDS_PER_COMPONENT,
            ),
            (
                "max_component_checks",
                self.max_component_checks,
                semantics::ABSOLUTE_MAX_COMPONENT_CHECKS,
            ),
        ] {
            if value == 0 || value > ceiling {
                return Err(MdmError::ResolverInvalid(format!(
                    "{resource} must be between 1 and {ceiling}"
                )));
            }
        }
        Ok(())
    }
}

pub type ManualEdge = DecisionEdge;
pub type CannotLink = DecisionEdge;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairKey {
    pub left_source_record_id: Uuid,
    pub right_source_record_id: Uuid,
    pub left_sort_key: Vec<u8>,
    pub right_sort_key: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnionOutcome {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnionFact {
    pub left_component_key: Vec<u8>,
    pub right_component_key: Vec<u8>,
    pub edge: PairKey,
    pub outcome: UnionOutcome,
    pub reason_code: String,
    pub evidence_groups: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Membership {
    pub source_record_id: Uuid,
    pub source_sort_key: Vec<u8>,
    pub component_key: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub memberships: Vec<Membership>,
    pub accepted: Vec<UnionFact>,
    pub rejected: Vec<UnionFact>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolverInput {
    pub records: Vec<ResolverRecord>,
    pub manual_matches: Vec<ManualEdge>,
    pub cannot_links: Vec<CannotLink>,
    pub pair_decisions: Vec<PairDecision>,
    pub limits: ResolverLimits,
}

fn canonical_endpoints(
    left: Uuid,
    right: Uuid,
    records: &BTreeMap<Uuid, ResolverRecord>,
) -> Result<(Uuid, Uuid), MdmError> {
    let left_record = records
        .get(&left)
        .ok_or_else(|| MdmError::ResolverInvalid(format!("unknown source record {left}")))?;
    let right_record = records
        .get(&right)
        .ok_or_else(|| MdmError::ResolverInvalid(format!("unknown source record {right}")))?;
    Ok(
        if (&left_record.source_sort_key, left) <= (&right_record.source_sort_key, right) {
            (left, right)
        } else {
            (right, left)
        },
    )
}

fn pair_key(
    left: Uuid,
    right: Uuid,
    records: &BTreeMap<Uuid, ResolverRecord>,
) -> Result<PairKey, MdmError> {
    let (left, right) = canonical_endpoints(left, right, records)?;
    Ok(PairKey {
        left_source_record_id: left,
        right_source_record_id: right,
        left_sort_key: records[&left].source_sort_key.clone(),
        right_sort_key: records[&right].source_sort_key.clone(),
    })
}

fn canonical_manual_edges(
    edges: &[ManualEdge],
    records: &BTreeMap<Uuid, ResolverRecord>,
    kind: DecisionKind,
) -> Result<Vec<ManualEdge>, MdmError> {
    let mut result = Vec::with_capacity(edges.len());
    for edge in edges {
        if edge.decision != kind {
            return Err(MdmError::ResolverInvalid(format!(
                "expected {} edge, got {}",
                kind.as_str(),
                edge.decision.as_str()
            )));
        }
        if edge.left_source_record_id == edge.right_source_record_id {
            return Err(MdmError::ResolverInvalid(
                "a resolver edge cannot reference the same source record twice".into(),
            ));
        }
        let (left, right) = canonical_endpoints(
            edge.left_source_record_id,
            edge.right_source_record_id,
            records,
        )?;
        let mut edge = edge.clone();
        edge.left_source_record_id = left;
        edge.right_source_record_id = right;
        result.push(edge);
    }
    result.sort_by(|left, right| {
        let left_key = pair_key(
            left.left_source_record_id,
            left.right_source_record_id,
            records,
        )
        .expect("validated manual edge");
        let right_key = pair_key(
            right.left_source_record_id,
            right.right_source_record_id,
            records,
        )
        .expect("validated manual edge");
        (
            left_key.left_sort_key,
            left_key.right_sort_key,
            left.decision_id,
        )
            .cmp(&(
                right_key.left_sort_key,
                right_key.right_sort_key,
                right.decision_id,
            ))
    });
    Ok(result)
}

fn authority_conflict(left: &component::Component, right: &component::Component) -> bool {
    left.authority.iter().any(|(field, left_values)| {
        right.authority.get(field).is_some_and(|right_values| {
            left_values.iter().any(|left_value| {
                right_values
                    .iter()
                    .any(|right_value| left_value != right_value)
            })
        })
    })
}

fn shared_authority(left: &component::Component, right: &component::Component) -> bool {
    left.authority.iter().any(|(field, left_values)| {
        right.authority.get(field).is_some_and(|right_values| {
            left_values.iter().any(|value| right_values.contains(value))
        })
    })
}

fn groups_for_pair<'a>(
    decisions: &'a BTreeMap<(Uuid, Uuid), Vec<&'a PairDecision>>,
    left: Uuid,
    right: Uuid,
    records: &BTreeMap<Uuid, ResolverRecord>,
) -> Vec<&'a PairDecision> {
    let pair = canonical_endpoints(left, right, records).expect("validated pair endpoint");
    decisions.get(&pair).cloned().unwrap_or_default()
}

fn singleton_admission(
    edge: &PairDecision,
    singleton: Uuid,
    established: &component::Component,
    decisions: &BTreeMap<(Uuid, Uuid), Vec<&PairDecision>>,
    records: &BTreeMap<Uuid, ResolverRecord>,
) -> Result<(), &'static str> {
    if edge.result == PairResult::AutomaticIdentity {
        return Ok(());
    }
    if edge.independent_agreeing_groups.len() > 1 {
        return Ok(());
    }
    let edge_groups = edge
        .independent_agreeing_groups
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let has_independent_group = established.members.iter().any(|member| {
        groups_for_pair(decisions, singleton, *member, records)
            .into_iter()
            .filter(|decision| {
                matches!(
                    decision.result,
                    PairResult::AutomaticStrong | PairResult::Review
                )
            })
            .flat_map(|decision| decision.independent_agreeing_groups.iter())
            .any(|group| !edge_groups.contains(group))
    });
    if has_independent_group {
        Ok(())
    } else {
        Err("SINGLETON_NEEDS_INDEPENDENT_GROUP")
    }
}

fn established_admission(
    left: &component::Component,
    right: &component::Component,
    decisions: &BTreeMap<(Uuid, Uuid), Vec<&PairDecision>>,
    records: &BTreeMap<Uuid, ResolverRecord>,
) -> Result<(), &'static str> {
    if shared_authority(left, right) {
        return Ok(());
    }
    let mut connections = BTreeMap::<(Uuid, Uuid), BTreeSet<String>>::new();
    for left_member in &left.members {
        for right_member in &right.members {
            for decision in groups_for_pair(decisions, *left_member, *right_member, records) {
                if decision.result == PairResult::AutomaticStrong {
                    let pair = canonical_endpoints(*left_member, *right_member, records)
                        .expect("validated pair endpoint");
                    connections
                        .entry(pair)
                        .or_default()
                        .extend(decision.independent_agreeing_groups.iter().cloned());
                }
            }
        }
    }
    if connections.len() < 2 {
        if connections.values().any(|groups| groups.len() > 1) {
            return Err("ESTABLISHED_STRONG_CONNECTIONS_REUSE_PAIR");
        }
        return Err("ESTABLISHED_NEEDS_TWO_STRONG_CONNECTIONS");
    }
    let connections = connections.into_iter().collect::<Vec<_>>();
    for (index, (_, left_groups)) in connections.iter().enumerate() {
        for (_, right_groups) in connections.iter().skip(index + 1) {
            if left_groups.is_disjoint(right_groups) {
                return Ok(());
            }
        }
    }
    Err("ESTABLISHED_STRONG_CONNECTIONS_NOT_INDEPENDENT")
}

fn cannot_cross(
    left: &component::Component,
    right: &component::Component,
    cannot_links: &BTreeSet<(Uuid, Uuid)>,
) -> bool {
    left.members.iter().any(|left_member| {
        right.members.iter().any(|right_member| {
            cannot_links.contains(&(*left_member, *right_member))
                || cannot_links.contains(&(*right_member, *left_member))
        })
    })
}

fn merge_components(
    union_find: &mut union_find::UnionFind,
    components: &mut [component::Component],
    left: usize,
    right: usize,
    sort_keys: &[Vec<u8>],
) -> usize {
    let left_root = union_find.find(left);
    let right_root = union_find.find(right);
    if left_root == right_root {
        return left_root;
    }
    let root = union_find.union(left_root, right_root, sort_keys);
    let other = if root == left_root {
        right_root
    } else {
        left_root
    };
    let other_component = std::mem::take(&mut components[other]);
    components[root].merge(other_component);
    root
}

fn fact(
    edge: PairKey,
    left_component_key: Vec<u8>,
    right_component_key: Vec<u8>,
    outcome: UnionOutcome,
    reason_code: impl Into<String>,
    mut evidence_groups: Vec<String>,
) -> UnionFact {
    let (left_component_key, right_component_key) = if left_component_key <= right_component_key {
        (left_component_key, right_component_key)
    } else {
        (right_component_key, left_component_key)
    };
    evidence_groups.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    evidence_groups.dedup();
    UnionFact {
        left_component_key,
        right_component_key,
        edge,
        outcome,
        reason_code: reason_code.into(),
        evidence_groups,
    }
}

pub fn resolve(input: ResolverInput) -> Result<Resolution, MdmError> {
    input.limits.validate()?;
    if input.records.len() > input.limits.max_active_records {
        return Err(MdmError::ResolverLimit {
            resource: "max_active_records",
            observed: input.records.len(),
            limit: input.limits.max_active_records,
        });
    }

    let mut records = BTreeMap::new();
    let mut sort_keys = BTreeSet::new();
    for record in input.records {
        if !sort_keys.insert(record.source_sort_key.clone()) {
            return Err(MdmError::ResolverInvalid(
                "source-record sort keys must be unique".into(),
            ));
        }
        if records.insert(record.source_record_id, record).is_some() {
            return Err(MdmError::ResolverInvalid(
                "source record IDs must be unique".into(),
            ));
        }
    }
    let ordered_records = records.values().collect::<Vec<_>>();
    let sort_keys = ordered_records
        .iter()
        .map(|record| record.source_sort_key.clone())
        .collect::<Vec<_>>();
    let indexes = ordered_records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.source_record_id, index))
        .collect::<BTreeMap<_, _>>();

    let manual = canonical_manual_edges(&input.manual_matches, &records, DecisionKind::Match)?;
    let cannot = canonical_manual_edges(&input.cannot_links, &records, DecisionKind::NotMatch)?;
    let mut cannot_links = cannot
        .iter()
        .map(|edge| (edge.left_source_record_id, edge.right_source_record_id))
        .collect::<BTreeSet<_>>();
    for decision in &input.pair_decisions {
        if decision.result == PairResult::Prohibited {
            let pair = canonical_endpoints(
                decision.pair.left_source_record_id,
                decision.pair.right_source_record_id,
                &records,
            )?;
            cannot_links.insert(pair);
        }
    }

    let mut union_find = union_find::UnionFind::new(records.len());
    let mut components = ordered_records
        .iter()
        .map(|record| component::Component::singleton(record))
        .collect::<Vec<_>>();
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for edge in manual {
        let left = indexes[&edge.left_source_record_id];
        let right = indexes[&edge.right_source_record_id];
        let left_root = union_find.find(left);
        let right_root = union_find.find(right);
        if left_root == right_root {
            continue;
        }
        if cannot_cross(
            &components[left_root],
            &components[right_root],
            &cannot_links,
        ) {
            return Err(MdmError::DecisionContradiction(format!(
                "manual MATCH {}:{}-{} crosses a NOT_MATCH",
                edge.decision_id, edge.left_source_record_id, edge.right_source_record_id
            )));
        }
        let left_key = components[left_root].key().to_vec();
        let right_key = components[right_root].key().to_vec();
        let edge_key = pair_key(
            edge.left_source_record_id,
            edge.right_source_record_id,
            &records,
        )?;
        merge_components(&mut union_find, &mut components, left, right, &sort_keys);
        accepted.push(fact(
            edge_key,
            left_key,
            right_key,
            UnionOutcome::Accepted,
            "MANUAL_MATCH",
            Vec::new(),
        ));
    }

    for &(left_id, right_id) in &cannot_links {
        let left = indexes[&left_id];
        let right = indexes[&right_id];
        if union_find.find(left) == union_find.find(right) {
            return Err(MdmError::DecisionContradiction(format!(
                "NOT_MATCH {}-{} is inside the manual MATCH closure",
                left_id, right_id
            )));
        }
    }

    let mut decisions = input
        .pair_decisions
        .iter()
        .filter(|decision| decision.result.is_automatic_edge())
        .collect::<Vec<_>>();
    if decisions.len() > input.limits.max_automatic_edges {
        return Err(MdmError::ResolverLimit {
            resource: "max_automatic_edges",
            observed: decisions.len(),
            limit: input.limits.max_automatic_edges,
        });
    }
    decisions.sort_by(|left, right| {
        left.sort_key
            .cmp(&right.sort_key)
            .then_with(|| left.pair.left_sort_key.cmp(&right.pair.left_sort_key))
            .then_with(|| left.pair.right_sort_key.cmp(&right.pair.right_sort_key))
            .then_with(|| {
                left.pair
                    .left_source_record_id
                    .cmp(&right.pair.left_source_record_id)
            })
            .then_with(|| {
                left.pair
                    .right_source_record_id
                    .cmp(&right.pair.right_source_record_id)
            })
    });
    let all_decisions = input
        .pair_decisions
        .iter()
        .filter(|decision| {
            decision.pair.left_source_record_id != decision.pair.right_source_record_id
        })
        .map(|decision| {
            let pair = canonical_endpoints(
                decision.pair.left_source_record_id,
                decision.pair.right_source_record_id,
                &records,
            )?;
            Ok::<_, MdmError>((pair, decision))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .fold(
            BTreeMap::<(Uuid, Uuid), Vec<&PairDecision>>::new(),
            |mut map, (pair, decision)| {
                map.entry(pair).or_default().push(decision);
                map
            },
        );

    let mut component_checks = 0usize;
    for decision in decisions {
        component_checks = component_checks.saturating_add(1);
        if component_checks > input.limits.max_component_checks {
            return Err(MdmError::ResolverLimit {
                resource: "max_component_checks",
                observed: component_checks,
                limit: input.limits.max_component_checks,
            });
        }
        let left = indexes
            .get(&decision.pair.left_source_record_id)
            .copied()
            .ok_or_else(|| {
                MdmError::ResolverInvalid(
                    "automatic edge references an unknown source record".into(),
                )
            })?;
        let right = indexes
            .get(&decision.pair.right_source_record_id)
            .copied()
            .ok_or_else(|| {
                MdmError::ResolverInvalid(
                    "automatic edge references an unknown source record".into(),
                )
            })?;
        let left_root = union_find.find(left);
        let right_root = union_find.find(right);
        let edge_key = pair_key(
            decision.pair.left_source_record_id,
            decision.pair.right_source_record_id,
            &records,
        )?;
        let left_key = components[left_root].key().to_vec();
        let right_key = components[right_root].key().to_vec();
        if left_root == right_root {
            accepted.push(fact(
                edge_key,
                left_key,
                right_key,
                UnionOutcome::Accepted,
                "ALREADY_CONNECTED",
                decision.independent_agreeing_groups.clone(),
            ));
            continue;
        }
        let (left_key, right_key) = if left_key <= right_key {
            (left_key, right_key)
        } else {
            (right_key, left_key)
        };
        let reject = if cannot_cross(
            &components[left_root],
            &components[right_root],
            &cannot_links,
        ) {
            Some("CANNOT_LINK")
        } else if authority_conflict(&components[left_root], &components[right_root]) {
            Some("AUTHORITATIVE_CONFLICT")
        } else if components[left_root].is_singleton() && components[right_root].is_singleton() {
            None
        } else if components[left_root].is_singleton() {
            singleton_admission(
                decision,
                decision.pair.left_source_record_id,
                &components[right_root],
                &all_decisions,
                &records,
            )
            .err()
        } else if components[right_root].is_singleton() {
            singleton_admission(
                decision,
                decision.pair.right_source_record_id,
                &components[left_root],
                &all_decisions,
                &records,
            )
            .err()
        } else {
            established_admission(
                &components[left_root],
                &components[right_root],
                &all_decisions,
                &records,
            )
            .err()
        };
        if let Some(reason) = reject {
            rejected_fact(
                &mut rejected,
                edge_key,
                left_key,
                right_key,
                reason,
                decision,
            );
            continue;
        }
        let new_size = components[left_root].members.len() + components[right_root].members.len();
        if new_size > input.limits.max_records_per_component {
            return Err(MdmError::ResolverLimit {
                resource: "max_records_per_component",
                observed: new_size,
                limit: input.limits.max_records_per_component,
            });
        }
        merge_components(&mut union_find, &mut components, left, right, &sort_keys);
        accepted.push(fact(
            edge_key,
            left_key,
            right_key,
            UnionOutcome::Accepted,
            decision.result.as_str(),
            decision.independent_agreeing_groups.clone(),
        ));
    }

    let mut memberships = ordered_records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let root = union_find.find(index);
            Membership {
                source_record_id: record.source_record_id,
                source_sort_key: record.source_sort_key.clone(),
                component_key: components[root].key().to_vec(),
            }
        })
        .collect::<Vec<_>>();
    memberships.sort_by(|left, right| left.source_sort_key.cmp(&right.source_sort_key));
    if memberships.len() != records.len()
        || memberships
            .iter()
            .map(|membership| membership.source_record_id)
            .collect::<BTreeSet<_>>()
            .len()
            != records.len()
    {
        return Err(MdmError::ResolverInvariant(
            "every active source record must have exactly one membership".into(),
        ));
    }
    for (left_id, right_id) in cannot_links {
        let left = memberships
            .iter()
            .find(|membership| membership.source_record_id == left_id)
            .expect("cannot-link endpoint was validated");
        let right = memberships
            .iter()
            .find(|membership| membership.source_record_id == right_id)
            .expect("cannot-link endpoint was validated");
        if left.component_key == right.component_key {
            return Err(MdmError::ResolverInvariant(
                "a successful component contains a cannot-link".into(),
            ));
        }
    }
    Ok(Resolution {
        memberships,
        accepted,
        rejected,
    })
}

fn rejected_fact(
    rejected: &mut Vec<UnionFact>,
    edge: PairKey,
    left_key: Vec<u8>,
    right_key: Vec<u8>,
    reason: &str,
    decision: &PairDecision,
) {
    rejected.push(fact(
        edge,
        left_key,
        right_key,
        UnionOutcome::Rejected,
        reason,
        decision.independent_agreeing_groups.clone(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidatePair;

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

    #[test]
    fn set_oracle_matches_union_find_on_a_small_graph() {
        let input = ResolverInput {
            records: (1..=4)
                .rev()
                .map(|value| ResolverRecord {
                    source_record_id: id(value),
                    source_sort_key: vec![value],
                    authority: BTreeMap::new(),
                })
                .collect(),
            manual_matches: Vec::new(),
            cannot_links: Vec::new(),
            pair_decisions: vec![
                edge(2, 4, "d", 4),
                edge(1, 3, "c", 3),
                edge(3, 4, "b", 2),
                edge(1, 2, "a", 1),
            ],
            limits: ResolverLimits::default(),
        };
        assert_eq!(
            resolve(input.clone()).unwrap(),
            oracle::resolve(input).unwrap()
        );
    }
}
