use std::cmp::Ordering;

use crate::candidate::CandidatePair;
use crate::definition::MatchRule;
use crate::error::MdmError;
use crate::evidence::{EvidenceClass, EvidenceItem, independent_agreeing_groups};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairResult {
    Prohibited,
    ManualMatch,
    AuthoritativeConflict,
    AutomaticIdentity,
    AutomaticStrong,
    Review,
    NoEdge,
}

impl PairResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prohibited => "PROHIBITED",
            Self::ManualMatch => "MANUAL_MATCH",
            Self::AuthoritativeConflict => "AUTHORITATIVE_CONFLICT",
            Self::AutomaticIdentity => "AUTOMATIC_IDENTITY",
            Self::AutomaticStrong => "AUTOMATIC_STRONG",
            Self::Review => "REVIEW",
            Self::NoEdge => "NO_EDGE",
        }
    }

    pub const fn is_automatic_edge(self) -> bool {
        matches!(self, Self::AutomaticIdentity | Self::AutomaticStrong)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairDecision {
    pub pair: CandidatePair,
    pub result: PairResult,
    pub evidence: Vec<EvidenceItem>,
    pub independent_agreeing_groups: Vec<String>,
    pub sort_key: Vec<u8>,
    pub reason_codes: Vec<String>,
}

fn result_for_manual(manual_decision: Option<&str>) -> Option<PairResult> {
    match manual_decision {
        Some("MATCH") => Some(PairResult::ManualMatch),
        Some("NOT_MATCH") => Some(PairResult::Prohibited),
        Some(other) => {
            let _ = other;
            None
        }
        None => None,
    }
}

fn automatic_sort_key(
    pair: &CandidatePair,
    result: PairResult,
    groups: &[String],
    evidence: &[EvidenceItem],
) -> Vec<u8> {
    let mut key = vec![
        1,
        match result {
            PairResult::AutomaticIdentity => 0,
            PairResult::AutomaticStrong => 1,
            _ => 255,
        },
    ];
    key.extend_from_slice(&(u16::MAX - groups.len().min(u16::MAX as usize) as u16).to_be_bytes());
    let mut scores = evidence
        .iter()
        .filter(|item| item.class == EvidenceClass::Agree)
        .map(|item| item.score.unwrap_or(10_000))
        .collect::<Vec<_>>();
    scores.sort_unstable_by(|left, right| right.cmp(left));
    key.push(scores.len().min(u8::MAX as usize) as u8);
    for score in scores {
        key.extend_from_slice(&(u16::MAX - score).to_be_bytes());
    }
    for sort_key in [&pair.left_sort_key, &pair.right_sort_key] {
        key.extend_from_slice(&(sort_key.len() as u32).to_be_bytes());
        key.extend_from_slice(sort_key);
    }
    key
}

pub fn decide_pair(
    pair: CandidatePair,
    mut evidence: Vec<EvidenceItem>,
    authority_conflict: bool,
    manual_decision: Option<&str>,
) -> PairDecision {
    evidence.sort_by(|left, right| left.rule.as_bytes().cmp(right.rule.as_bytes()));
    let groups = independent_agreeing_groups(&evidence);
    let result = result_for_manual(manual_decision).unwrap_or_else(|| {
        if authority_conflict {
            PairResult::AuthoritativeConflict
        } else if evidence.iter().any(|item| {
            item.class == EvidenceClass::Agree
                && item.evidence_group.as_str() != ""
                && item.rule.as_str() != ""
        }) && evidence.iter().any(|item| {
            item.class == EvidenceClass::Agree
                && item.rule.as_str() != ""
                && item.evidence_group.as_str() != ""
        }) {
            // The strength is supplied by the rule name in the evidence metadata in
            // SQL; pure callers use the explicit helper below when strength matters.
            PairResult::AutomaticStrong
        } else {
            PairResult::NoEdge
        }
    });
    let reason_codes = match result {
        PairResult::Prohibited => vec!["NOT_MATCH".into()],
        PairResult::ManualMatch => vec!["MATCH".into()],
        PairResult::AuthoritativeConflict => vec!["AUTHORITATIVE_CONFLICT".into()],
        PairResult::Review => vec!["SUPPORTING_ONLY".into()],
        PairResult::AutomaticIdentity => vec!["IDENTITY_AGREE".into()],
        PairResult::AutomaticStrong => vec!["STRONG_AGREE".into()],
        PairResult::NoEdge => vec!["NO_AGREEMENT".into()],
    };
    let sort_key = automatic_sort_key(&pair, result, &groups, &evidence);
    PairDecision {
        pair,
        result,
        evidence,
        independent_agreeing_groups: groups,
        sort_key,
        reason_codes,
    }
}

pub fn decide_pair_with_strength(
    pair: CandidatePair,
    evidence: Vec<(EvidenceItem, String)>,
    authority_conflict: bool,
    manual_decision: Option<&str>,
) -> PairDecision {
    let supporting_only = !evidence.is_empty()
        && evidence
            .iter()
            .all(|(item, strength)| item.class != EvidenceClass::Agree || strength == "supporting")
        && evidence
            .iter()
            .any(|(item, strength)| item.class == EvidenceClass::Agree && strength == "supporting");
    let has_identity = evidence
        .iter()
        .any(|(item, strength)| item.class == EvidenceClass::Agree && strength == "identity");
    let has_strong = evidence
        .iter()
        .any(|(item, strength)| item.class == EvidenceClass::Agree && strength == "strong");
    let mut items = evidence
        .iter()
        .map(|(item, _)| item.clone())
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.rule.as_bytes().cmp(right.rule.as_bytes()));
    let mut decision = PairDecision {
        pair,
        result: PairResult::NoEdge,
        evidence: items,
        independent_agreeing_groups: Vec::new(),
        sort_key: Vec::new(),
        reason_codes: Vec::new(),
    };
    let groups = independent_agreeing_groups(&decision.evidence);
    decision.independent_agreeing_groups = groups.clone();
    decision.result = if let Some(manual) = result_for_manual(manual_decision) {
        manual
    } else if authority_conflict {
        PairResult::AuthoritativeConflict
    } else if has_identity {
        PairResult::AutomaticIdentity
    } else if has_strong {
        PairResult::AutomaticStrong
    } else if supporting_only {
        PairResult::Review
    } else {
        PairResult::NoEdge
    };
    decision.reason_codes = match decision.result {
        PairResult::Prohibited => vec!["NOT_MATCH".into()],
        PairResult::ManualMatch => vec!["MATCH".into()],
        PairResult::AuthoritativeConflict => vec!["AUTHORITATIVE_CONFLICT".into()],
        PairResult::AutomaticIdentity => vec!["IDENTITY_AGREE".into()],
        PairResult::AutomaticStrong => vec!["STRONG_AGREE".into()],
        PairResult::Review => vec!["SUPPORTING_ONLY".into()],
        PairResult::NoEdge => vec!["NO_AGREEMENT".into()],
    };
    decision.sort_key =
        automatic_sort_key(&decision.pair, decision.result, &groups, &decision.evidence);
    decision
}

pub fn decide_pair_for_rules(
    pair: CandidatePair,
    evidence: Vec<EvidenceItem>,
    rules: &[MatchRule],
    authority_conflict: bool,
    manual_decision: Option<&str>,
) -> PairDecision {
    let strength_by_rule = rules
        .iter()
        .map(|rule| (rule.name.as_str(), rule.strength.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let evidence = evidence
        .into_iter()
        .map(|item| {
            let strength = strength_by_rule
                .get(item.rule.as_str())
                .copied()
                .unwrap_or("supporting")
                .to_owned();
            (item, strength)
        })
        .collect();
    decide_pair_with_strength(pair, evidence, authority_conflict, manual_decision)
}

pub fn sort_automatic_edges(decisions: &mut [PairDecision]) -> Result<(), MdmError> {
    if decisions
        .iter()
        .any(|decision| !decision.result.is_automatic_edge())
    {
        return Err(MdmError::EvidenceInvalid(
            "only automatic edges can be sorted".into(),
        ));
    }
    decisions.sort_by(|left, right| {
        left.sort_key.cmp(&right.sort_key).then_with(|| {
            left.pair
                .left_source_record_id
                .cmp(&right.pair.left_source_record_id)
                .then_with(|| {
                    left.pair
                        .right_source_record_id
                        .cmp(&right.pair.right_source_record_id)
                })
        })
    });
    Ok(())
}

pub fn sort_key_version() -> u8 {
    1
}

pub fn compare_sort_keys(left: &[u8], right: &[u8]) -> Ordering {
    left.cmp(right)
}
