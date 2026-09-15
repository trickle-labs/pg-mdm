use std::collections::BTreeSet;

use pgrx::Uuid;

use crate::constraint::{DecisionEdge, DecisionKind};
use crate::error::MdmError;
use crate::identity::IdentityMembership;
use crate::pair::{PairDecision, PairResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffectedSet {
    records: BTreeSet<Uuid>,
}

impl AffectedSet {
    pub fn records(&self) -> &BTreeSet<Uuid> {
        &self.records
    }

    pub fn into_records(self) -> BTreeSet<Uuid> {
        self.records
    }
}

pub fn build(
    seeds: impl IntoIterator<Item = Uuid>,
    old_memberships: &[IdentityMembership],
    manual_decisions: &[DecisionEdge],
    pair_decisions: &[PairDecision],
) -> Result<AffectedSet, MdmError> {
    let mut records = seeds.into_iter().collect::<BTreeSet<_>>();
    loop {
        let before = records.len();
        let mdm_ids = old_memberships
            .iter()
            .filter(|membership| records.contains(&membership.source_record_id))
            .map(|membership| membership.mdm_id)
            .collect::<BTreeSet<_>>();
        records.extend(
            old_memberships
                .iter()
                .filter(|membership| mdm_ids.contains(&membership.mdm_id))
                .map(|membership| membership.source_record_id),
        );
        for decision in manual_decisions
            .iter()
            .filter(|decision| decision.decision == DecisionKind::Match)
        {
            add_edge(
                &mut records,
                decision.left_source_record_id,
                decision.right_source_record_id,
            );
        }
        for decision in pair_decisions.iter().filter(|decision| {
            matches!(
                decision.result,
                PairResult::AutomaticIdentity | PairResult::AutomaticStrong | PairResult::Review
            )
        }) {
            add_edge(
                &mut records,
                decision.pair.left_source_record_id,
                decision.pair.right_source_record_id,
            );
        }
        if records.len() == before {
            break;
        }
    }

    for decision in manual_decisions
        .iter()
        .filter(|decision| decision.decision == DecisionKind::Match)
    {
        ensure_closed(
            &records,
            decision.left_source_record_id,
            decision.right_source_record_id,
        )?;
    }
    for decision in pair_decisions.iter().filter(|decision| {
        matches!(
            decision.result,
            PairResult::AutomaticIdentity | PairResult::AutomaticStrong | PairResult::Review
        )
    }) {
        ensure_closed(
            &records,
            decision.pair.left_source_record_id,
            decision.pair.right_source_record_id,
        )?;
    }
    Ok(AffectedSet { records })
}

fn add_edge(records: &mut BTreeSet<Uuid>, left: Uuid, right: Uuid) {
    if records.contains(&left) || records.contains(&right) {
        records.insert(left);
        records.insert(right);
    }
}

fn ensure_closed(records: &BTreeSet<Uuid>, left: Uuid, right: Uuid) -> Result<(), MdmError> {
    if records.contains(&left) != records.contains(&right) {
        return Err(MdmError::AffectedClosure(format!(
            "connecting edge {left}:{right} crosses the affected boundary"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::CandidatePair;

    fn id(value: u8) -> Uuid {
        Uuid::from_bytes([value; 16])
    }

    fn membership(record: u8, mdm_id: u8) -> IdentityMembership {
        IdentityMembership {
            source_record_id: id(record),
            source_sort_key: vec![record],
            mdm_id: id(mdm_id),
            active: true,
            first_membership_revision: 1,
            last_membership_revision: 1,
            membership_reason: "test".into(),
            last_change_revision: 1,
        }
    }

    fn pair(left: u8, right: u8, result: PairResult) -> PairDecision {
        PairDecision {
            pair: CandidatePair {
                left_source_record_id: id(left),
                right_source_record_id: id(right),
                left_sort_key: vec![left],
                right_sort_key: vec![right],
                discovery_channels: Vec::new(),
            },
            result,
            evidence: Vec::new(),
            independent_agreeing_groups: Vec::new(),
            sort_key: Vec::new(),
            reason_codes: Vec::new(),
        }
    }

    #[test]
    fn closure_includes_old_component_and_connecting_edges() {
        let manual = DecisionEdge {
            decision_id: id(9),
            left_source_record_id: id(3),
            right_source_record_id: id(4),
            decision: DecisionKind::Match,
        };
        let affected = build(
            [id(1)],
            &[membership(1, 8), membership(2, 8)],
            &[manual],
            &[pair(2, 3, PairResult::Review)],
        )
        .unwrap()
        .into_records();
        assert_eq!(affected, BTreeSet::from([id(1), id(2), id(3), id(4)]));
    }

    #[test]
    fn rejected_edges_do_not_connect_the_closure() {
        let affected = build([id(1)], &[], &[], &[pair(1, 2, PairResult::Prohibited)])
            .unwrap()
            .into_records();
        assert_eq!(affected, BTreeSet::from([id(1)]));
    }

    #[test]
    fn closure_is_order_independent() {
        let memberships = [membership(1, 8), membership(2, 8), membership(3, 9)];
        let pairs = [pair(2, 3, PairResult::AutomaticStrong)];
        let first = build([id(1)], &memberships, &[], &pairs).unwrap();
        let second = build([id(1)], &memberships, &[], &pairs).unwrap();
        assert_eq!(first, second);
    }
}
