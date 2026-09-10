use std::collections::{BTreeMap, BTreeSet};

use pgrx::Uuid;

use super::{
    ManualEdge, PairKey, Resolution, ResolverInput, ResolverRecord, UnionFact, UnionOutcome,
};
use crate::constraint::DecisionKind;
use crate::error::MdmError;
use crate::pair::{PairDecision, PairResult};

#[derive(Clone, Debug, Default)]
struct SetComponent {
    members: BTreeSet<Uuid>,
    authority: BTreeMap<String, BTreeSet<String>>,
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

fn component(record: &ResolverRecord) -> SetComponent {
    let mut authority = BTreeMap::new();
    for (field, value) in &record.authority {
        if !value.is_empty() {
            authority
                .entry(field.clone())
                .or_insert_with(BTreeSet::new)
                .insert(value.clone());
        }
    }
    SetComponent {
        members: BTreeSet::from([record.source_record_id]),
        authority,
    }
}

fn component_index(components: &[SetComponent], record: Uuid) -> usize {
    components
        .iter()
        .position(|component| component.members.contains(&record))
        .expect("validated source record")
}

fn component_key(component: &SetComponent, records: &BTreeMap<Uuid, ResolverRecord>) -> Vec<u8> {
    component
        .members
        .iter()
        .map(|record| records[record].source_sort_key.clone())
        .min()
        .unwrap_or_default()
}

fn merge(left: &mut SetComponent, right: SetComponent) {
    left.members.extend(right.members);
    for (field, values) in right.authority {
        left.authority.entry(field).or_default().extend(values);
    }
}

fn cannot_cross(
    left: &SetComponent,
    right: &SetComponent,
    cannot_links: &BTreeSet<(Uuid, Uuid)>,
) -> bool {
    left.members.iter().any(|left_member| {
        right.members.iter().any(|right_member| {
            cannot_links.contains(&(*left_member, *right_member))
                || cannot_links.contains(&(*right_member, *left_member))
        })
    })
}

fn authority_conflict(left: &SetComponent, right: &SetComponent) -> bool {
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

fn shared_authority(left: &SetComponent, right: &SetComponent) -> bool {
    left.authority.iter().any(|(field, left_values)| {
        right.authority.get(field).is_some_and(|right_values| {
            left_values.iter().any(|value| right_values.contains(value))
        })
    })
}

fn groups<'a>(
    decisions: &'a BTreeMap<(Uuid, Uuid), Vec<&'a PairDecision>>,
    left: Uuid,
    right: Uuid,
    records: &BTreeMap<Uuid, ResolverRecord>,
) -> Vec<&'a PairDecision> {
    let pair = canonical_endpoints(left, right, records).expect("validated pair endpoint");
    decisions.get(&pair).cloned().unwrap_or_default()
}

fn singleton_admission(
    decision: &PairDecision,
    singleton: Uuid,
    established: &SetComponent,
    decisions: &BTreeMap<(Uuid, Uuid), Vec<&PairDecision>>,
    records: &BTreeMap<Uuid, ResolverRecord>,
) -> bool {
    if decision.result == PairResult::AutomaticIdentity
        || decision.independent_agreeing_groups.len() > 1
    {
        return true;
    }
    let edge_groups = decision
        .independent_agreeing_groups
        .iter()
        .collect::<BTreeSet<_>>();
    established.members.iter().any(|member| {
        groups(decisions, singleton, *member, records)
            .into_iter()
            .filter(|candidate| {
                matches!(
                    candidate.result,
                    PairResult::AutomaticStrong | PairResult::Review
                )
            })
            .flat_map(|candidate| candidate.independent_agreeing_groups.iter())
            .any(|group| !edge_groups.contains(&group))
    })
}

fn established_admission(
    left: &SetComponent,
    right: &SetComponent,
    decisions: &BTreeMap<(Uuid, Uuid), Vec<&PairDecision>>,
    records: &BTreeMap<Uuid, ResolverRecord>,
) -> Option<&'static str> {
    if shared_authority(left, right) {
        return None;
    }
    let mut connections = BTreeMap::<(Uuid, Uuid), BTreeSet<String>>::new();
    for left_member in &left.members {
        for right_member in &right.members {
            for decision in groups(decisions, *left_member, *right_member, records) {
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
        return Some(if connections.values().any(|groups| groups.len() > 1) {
            "ESTABLISHED_STRONG_CONNECTIONS_REUSE_PAIR"
        } else {
            "ESTABLISHED_NEEDS_TWO_STRONG_CONNECTIONS"
        });
    }
    let connections = connections.into_iter().collect::<Vec<_>>();
    if connections
        .iter()
        .enumerate()
        .any(|(index, (_, left_groups))| {
            connections
                .iter()
                .skip(index + 1)
                .any(|(_, right_groups)| left_groups.is_disjoint(right_groups))
        })
    {
        None
    } else {
        Some("ESTABLISHED_STRONG_CONNECTIONS_NOT_INDEPENDENT")
    }
}

fn fact(
    edge: PairKey,
    left_key: Vec<u8>,
    right_key: Vec<u8>,
    outcome: UnionOutcome,
    reason: impl Into<String>,
    mut groups: Vec<String>,
) -> UnionFact {
    let (left_key, right_key) = if left_key <= right_key {
        (left_key, right_key)
    } else {
        (right_key, left_key)
    };
    groups.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    groups.dedup();
    UnionFact {
        left_component_key: left_key,
        right_component_key: right_key,
        edge,
        outcome,
        reason_code: reason.into(),
        evidence_groups: groups,
    }
}

fn canonical_manual(
    edges: &[ManualEdge],
    records: &BTreeMap<Uuid, ResolverRecord>,
    kind: DecisionKind,
) -> Result<Vec<ManualEdge>, MdmError> {
    let mut result = Vec::new();
    for edge in edges {
        if edge.decision != kind || edge.left_source_record_id == edge.right_source_record_id {
            return Err(MdmError::ResolverInvalid(
                "invalid manual resolver edge".into(),
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
    result.sort_by_key(|edge| {
        let left = &records[&edge.left_source_record_id].source_sort_key;
        let right = &records[&edge.right_source_record_id].source_sort_key;
        (left.clone(), right.clone(), edge.decision_id)
    });
    Ok(result)
}

pub(crate) fn resolve(input: ResolverInput) -> Result<Resolution, MdmError> {
    input.limits.validate()?;
    if input.records.len() > input.limits.max_active_records {
        return Err(MdmError::ResolverLimit {
            resource: "max_active_records",
            observed: input.records.len(),
            limit: input.limits.max_active_records,
        });
    }
    let records = input
        .records
        .into_iter()
        .map(|record| (record.source_record_id, record))
        .collect::<BTreeMap<_, _>>();
    let manual = canonical_manual(&input.manual_matches, &records, DecisionKind::Match)?;
    let cannot = canonical_manual(&input.cannot_links, &records, DecisionKind::NotMatch)?;
    let mut cannot_links = cannot
        .iter()
        .map(|edge| (edge.left_source_record_id, edge.right_source_record_id))
        .collect::<BTreeSet<_>>();
    for decision in &input.pair_decisions {
        if decision.result == PairResult::Prohibited {
            cannot_links.insert(canonical_endpoints(
                decision.pair.left_source_record_id,
                decision.pair.right_source_record_id,
                &records,
            )?);
        }
    }
    let mut components = records.values().map(component).collect::<Vec<_>>();
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();

    for edge in manual {
        let left = component_index(&components, edge.left_source_record_id);
        let right = component_index(&components, edge.right_source_record_id);
        if left == right {
            continue;
        }
        if cannot_cross(&components[left], &components[right], &cannot_links) {
            return Err(MdmError::DecisionContradiction(
                "manual MATCH crosses a NOT_MATCH".into(),
            ));
        }
        let left_key = component_key(&components[left], &records);
        let right_key = component_key(&components[right], &records);
        let edge_key = pair_key(
            edge.left_source_record_id,
            edge.right_source_record_id,
            &records,
        )?;
        let right_component = std::mem::take(&mut components[right]);
        merge(&mut components[left], right_component);
        accepted.push(fact(
            edge_key,
            left_key,
            right_key,
            UnionOutcome::Accepted,
            "MANUAL_MATCH",
            Vec::new(),
        ));
    }
    for (left_id, right_id) in &cannot_links {
        if component_index(&components, *left_id) == component_index(&components, *right_id) {
            return Err(MdmError::DecisionContradiction(
                "NOT_MATCH is in manual closure".into(),
            ));
        }
    }

    let mut all_decisions = BTreeMap::<(Uuid, Uuid), Vec<&PairDecision>>::new();
    for decision in &input.pair_decisions {
        if decision.pair.left_source_record_id != decision.pair.right_source_record_id {
            all_decisions
                .entry(canonical_endpoints(
                    decision.pair.left_source_record_id,
                    decision.pair.right_source_record_id,
                    &records,
                )?)
                .or_default()
                .push(decision);
        }
    }
    let mut automatic = input
        .pair_decisions
        .iter()
        .filter(|decision| decision.result.is_automatic_edge())
        .collect::<Vec<_>>();
    if automatic.len() > input.limits.max_automatic_edges {
        return Err(MdmError::ResolverLimit {
            resource: "max_automatic_edges",
            observed: automatic.len(),
            limit: input.limits.max_automatic_edges,
        });
    }
    automatic.sort_by(|left, right| {
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
    for decision in automatic {
        let left = component_index(&components, decision.pair.left_source_record_id);
        let right = component_index(&components, decision.pair.right_source_record_id);
        let edge_key = pair_key(
            decision.pair.left_source_record_id,
            decision.pair.right_source_record_id,
            &records,
        )?;
        let left_key = component_key(&components[left], &records);
        let right_key = component_key(&components[right], &records);
        if left == right {
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
        let reason = if cannot_cross(&components[left], &components[right], &cannot_links) {
            Some("CANNOT_LINK")
        } else if authority_conflict(&components[left], &components[right]) {
            Some("AUTHORITATIVE_CONFLICT")
        } else if components[left].members.len() == 1 && components[right].members.len() == 1 {
            None
        } else if components[left].members.len() == 1 {
            (!singleton_admission(
                decision,
                decision.pair.left_source_record_id,
                &components[right],
                &all_decisions,
                &records,
            ))
            .then_some("SINGLETON_NEEDS_INDEPENDENT_GROUP")
        } else if components[right].members.len() == 1 {
            (!singleton_admission(
                decision,
                decision.pair.right_source_record_id,
                &components[left],
                &all_decisions,
                &records,
            ))
            .then_some("SINGLETON_NEEDS_INDEPENDENT_GROUP")
        } else {
            established_admission(
                &components[left],
                &components[right],
                &all_decisions,
                &records,
            )
        };
        if let Some(reason) = reason {
            rejected.push(fact(
                edge_key,
                left_key,
                right_key,
                UnionOutcome::Rejected,
                reason,
                decision.independent_agreeing_groups.clone(),
            ));
            continue;
        }
        let size = components[left].members.len() + components[right].members.len();
        if size > input.limits.max_records_per_component {
            return Err(MdmError::ResolverLimit {
                resource: "max_records_per_component",
                observed: size,
                limit: input.limits.max_records_per_component,
            });
        }
        let right_component = std::mem::take(&mut components[right]);
        merge(&mut components[left], right_component);
        accepted.push(fact(
            edge_key,
            left_key,
            right_key,
            UnionOutcome::Accepted,
            decision.result.as_str(),
            decision.independent_agreeing_groups.clone(),
        ));
    }

    let mut memberships = records
        .values()
        .map(|record| {
            let index = component_index(&components, record.source_record_id);
            super::Membership {
                source_record_id: record.source_record_id,
                source_sort_key: record.source_sort_key.clone(),
                component_key: component_key(&components[index], &records),
            }
        })
        .collect::<Vec<_>>();
    memberships.sort_by(|left, right| left.source_sort_key.cmp(&right.source_sort_key));
    Ok(Resolution {
        memberships,
        accepted,
        rejected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_is_resolved() {
        let result = resolve(ResolverInput {
            records: Vec::new(),
            manual_matches: Vec::new(),
            cannot_links: Vec::new(),
            pair_decisions: Vec::new(),
            limits: super::super::ResolverLimits::default(),
        })
        .unwrap();
        assert!(result.memberships.is_empty());
    }
}
