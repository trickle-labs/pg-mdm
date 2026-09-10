use std::collections::{BTreeMap, BTreeSet};

use pgrx::Uuid;

use crate::error::MdmError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionKind {
    Match,
    NotMatch,
}

impl DecisionKind {
    pub fn parse(value: &str) -> Result<Self, MdmError> {
        match value {
            "MATCH" => Ok(Self::Match),
            "NOT_MATCH" => Ok(Self::NotMatch),
            _ => Err(MdmError::DecisionInvalid(
                "decision must be MATCH or NOT_MATCH".into(),
            )),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Match => "MATCH",
            Self::NotMatch => "NOT_MATCH",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionEdge {
    pub decision_id: Uuid,
    pub left_source_record_id: Uuid,
    pub right_source_record_id: Uuid,
    pub decision: DecisionKind,
}

impl DecisionEdge {
    pub fn canonical(mut self) -> Self {
        if self.left_source_record_id > self.right_source_record_id {
            std::mem::swap(
                &mut self.left_source_record_id,
                &mut self.right_source_record_id,
            );
        }
        self
    }
}

fn find(parent: &mut BTreeMap<Uuid, Uuid>, node: Uuid) -> Uuid {
    let root = parent.get(&node).copied().unwrap_or(node);
    if root == node {
        return node;
    }
    let root = find(parent, root);
    parent.insert(node, root);
    root
}

fn union(parent: &mut BTreeMap<Uuid, Uuid>, left: Uuid, right: Uuid) {
    let left_root = find(parent, left);
    let right_root = find(parent, right);
    if left_root != right_root {
        let (first, second) = if left_root < right_root {
            (left_root, right_root)
        } else {
            (right_root, left_root)
        };
        parent.insert(second, first);
    }
}

fn contradiction_path(edge: &DecisionEdge, other: &DecisionEdge) -> String {
    format!(
        "{}:{}-{} crosses {}:{}-{}",
        edge.decision_id,
        edge.left_source_record_id,
        edge.right_source_record_id,
        other.decision_id,
        other.left_source_record_id,
        other.right_source_record_id
    )
}

pub fn validate_proposed_decision(
    existing: &[DecisionEdge],
    proposed: &DecisionEdge,
    closure_limit: usize,
) -> Result<(), MdmError> {
    if proposed.left_source_record_id == proposed.right_source_record_id {
        return Err(MdmError::DecisionInvalid(
            "a decision cannot reference the same source record twice".into(),
        ));
    }
    if closure_limit == 0 {
        return Err(MdmError::DecisionCheckLimit {
            checked: 1,
            limit: closure_limit,
        });
    }
    let current = existing
        .iter()
        .filter(|edge| edge.decision_id != proposed.decision_id)
        .map(|edge| edge.clone().canonical())
        .chain(std::iter::once(proposed.clone().canonical()))
        .collect::<Vec<_>>();
    let checked = current
        .len()
        .checked_mul(4)
        .and_then(|value| value.checked_add(2))
        .unwrap_or(usize::MAX);
    if checked > closure_limit {
        return Err(MdmError::DecisionCheckLimit {
            checked,
            limit: closure_limit,
        });
    }

    let mut nodes = BTreeSet::new();
    for edge in &current {
        nodes.insert(edge.left_source_record_id);
        nodes.insert(edge.right_source_record_id);
    }
    let mut parent = nodes.iter().copied().map(|id| (id, id)).collect();
    for edge in current
        .iter()
        .filter(|edge| edge.decision == DecisionKind::Match)
    {
        union(
            &mut parent,
            edge.left_source_record_id,
            edge.right_source_record_id,
        );
    }

    let cannot_links = current
        .iter()
        .filter(|edge| edge.decision == DecisionKind::NotMatch)
        .collect::<Vec<_>>();
    for edge in cannot_links {
        if find(&mut parent, edge.left_source_record_id)
            == find(&mut parent, edge.right_source_record_id)
        {
            let match_edge = current.iter().find(|candidate| {
                candidate.decision == DecisionKind::Match
                    && find(&mut parent, candidate.left_source_record_id)
                        == find(&mut parent, candidate.right_source_record_id)
            });
            let path = match_edge
                .map(|match_edge| contradiction_path(match_edge, edge))
                .unwrap_or_else(|| {
                    format!(
                        "{}:{}-{}",
                        edge.decision_id, edge.left_source_record_id, edge.right_source_record_id
                    )
                });
            return Err(MdmError::DecisionContradiction(path));
        }
    }
    Ok(())
}

pub fn manual_components(
    decisions: &[DecisionEdge],
    closure_limit: usize,
) -> Result<BTreeMap<Uuid, BTreeSet<Uuid>>, MdmError> {
    if decisions.len() > closure_limit {
        return Err(MdmError::DecisionCheckLimit {
            checked: decisions.len(),
            limit: closure_limit,
        });
    }
    let mut parent = BTreeMap::new();
    for edge in decisions
        .iter()
        .filter(|edge| edge.decision == DecisionKind::Match)
    {
        parent
            .entry(edge.left_source_record_id)
            .or_insert(edge.left_source_record_id);
        parent
            .entry(edge.right_source_record_id)
            .or_insert(edge.right_source_record_id);
        union(
            &mut parent,
            edge.left_source_record_id,
            edge.right_source_record_id,
        );
    }
    let mut components = BTreeMap::new();
    for node in parent.keys().copied().collect::<Vec<_>>() {
        let root = find(&mut parent, node);
        components
            .entry(root)
            .or_insert_with(BTreeSet::new)
            .insert(node);
    }
    Ok(components)
}
