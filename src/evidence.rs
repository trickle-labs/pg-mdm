use std::collections::BTreeMap;

use pgrx::Uuid;
use sha2::{Digest, Sha256};

use crate::candidate::CandidatePair;
use crate::comparators::{self, ComparisonClass};
use crate::definition::MatchRule;
use crate::error::MdmError;
use crate::normalization::NormalizedState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceClass {
    Agree,
    Disagree,
    NoEvidence,
}

impl EvidenceClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agree => "agree",
            Self::Disagree => "disagree",
            Self::NoEvidence => "no_evidence",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceItem {
    pub rule: String,
    pub evidence_group: String,
    pub class: EvidenceClass,
    pub score: Option<u16>,
    pub comparator: String,
    pub comparator_version: u16,
    pub left_value_digest: Option<[u8; 32]>,
    pub right_value_digest: Option<[u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedEvidenceValue {
    pub source_record_id: Uuid,
    pub field: String,
    pub state: NormalizedState,
    pub normalized: Option<String>,
    pub canonical_bytes: Option<Vec<u8>>,
}

pub type EvidenceRecord = NormalizedEvidenceValue;

fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

#[pgrx::pg_extern(
    name = "evidence_digest",
    requires = ["pg_mdm_foundation"],
    sql = "CREATE FUNCTION mdm_internal.evidence_digest(value text) RETURNS bytea IMMUTABLE PARALLEL SAFE SET search_path = pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'evidence_digest_wrapper';"
)]
#[pgrx::search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn evidence_digest(value: Option<String>) -> Option<Vec<u8>> {
    value.map(|value| digest(&value).to_vec())
}

fn comparison_class(class: ComparisonClass) -> EvidenceClass {
    match class {
        ComparisonClass::Agree => EvidenceClass::Agree,
        ComparisonClass::Disagree => EvidenceClass::Disagree,
        ComparisonClass::NoEvidence => EvidenceClass::NoEvidence,
    }
}

fn field_value<'a>(
    values: &'a [NormalizedEvidenceValue],
    record_id: Uuid,
    field: &str,
) -> Result<Option<&'a NormalizedEvidenceValue>, MdmError> {
    let mut found = None;
    for value in values
        .iter()
        .filter(|value| value.source_record_id == record_id && value.field == field)
    {
        if found.replace(value).is_some() {
            return Err(MdmError::EvidenceInvalid(format!(
                "source record has multiple normalized values for field {field}"
            )));
        }
    }
    Ok(found)
}

fn composite_bytes(values: &[&NormalizedEvidenceValue]) -> Vec<u8> {
    values.iter().fold(Vec::new(), |mut output, value| {
        let bytes = value.canonical_bytes.as_deref().unwrap_or_default();
        output.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        output.extend_from_slice(bytes);
        output
    })
}

fn composite_text(values: &[&NormalizedEvidenceValue]) -> String {
    values
        .iter()
        .map(|value| value.normalized.as_deref().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\0")
}

fn evaluate_rule(
    rule: &MatchRule,
    pair: &CandidatePair,
    values: &[NormalizedEvidenceValue],
    max_work: usize,
) -> Result<EvidenceItem, MdmError> {
    let comparator = comparators::comparator_name(&rule.comparison).ok_or_else(|| {
        MdmError::ComparatorInvalid(format!("unsupported comparator {}", rule.comparison))
    })?;
    let comparator_version =
        comparators::comparator_version(&rule.comparison).ok_or_else(|| {
            MdmError::ComparatorInvalid(format!("unsupported comparator {}", rule.comparison))
        })?;
    let mut left = Vec::with_capacity(rule.fields.len());
    let mut right = Vec::with_capacity(rule.fields.len());
    for field in &rule.fields {
        let left_value = field_value(values, pair.left_source_record_id, field)?;
        let right_value = field_value(values, pair.right_source_record_id, field)?;
        let (Some(left_value), Some(right_value)) = (left_value, right_value) else {
            return Ok(EvidenceItem {
                rule: rule.name.clone(),
                evidence_group: rule.evidence_group.clone(),
                class: EvidenceClass::NoEvidence,
                score: None,
                comparator: comparator.into(),
                comparator_version,
                left_value_digest: None,
                right_value_digest: None,
            });
        };
        if !left_value.state.can_supply_evidence()
            || !right_value.state.can_supply_evidence()
            || left_value.canonical_bytes.is_none()
            || right_value.canonical_bytes.is_none()
            || left_value.normalized.is_none()
            || right_value.normalized.is_none()
        {
            return Ok(EvidenceItem {
                rule: rule.name.clone(),
                evidence_group: rule.evidence_group.clone(),
                class: EvidenceClass::NoEvidence,
                score: None,
                comparator: comparator.into(),
                comparator_version,
                left_value_digest: None,
                right_value_digest: None,
            });
        }
        left.push(left_value);
        right.push(right_value);
    }

    let left_digest_value = composite_text(&left);
    let right_digest_value = composite_text(&right);
    let left_digest = Some(digest(&left_digest_value));
    let right_digest = Some(digest(&right_digest_value));
    let result = if rule.comparison == "exact" {
        let result = crate::comparators::exact::compare(
            Some(&composite_bytes(&left)),
            Some(&composite_bytes(&right)),
        );
        crate::comparators::ComparisonResult {
            class: result.class,
            score: result.score,
        }
    } else {
        let threshold = rule
            .threshold
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(comparators::SCORE_MAX);
        crate::comparators::levenshtein::compare(
            Some(&left_digest_value),
            Some(&right_digest_value),
            threshold,
            max_work,
        )?
    };
    Ok(EvidenceItem {
        rule: rule.name.clone(),
        evidence_group: rule.evidence_group.clone(),
        class: comparison_class(result.class),
        score: result.score,
        comparator: comparator.into(),
        comparator_version,
        left_value_digest: left_digest,
        right_value_digest: right_digest,
    })
}

pub fn evaluate_pair(
    pair: &CandidatePair,
    rules: &[MatchRule],
    values: &[NormalizedEvidenceValue],
    max_work: usize,
) -> Result<Vec<EvidenceItem>, MdmError> {
    let mut evidence = rules
        .iter()
        .map(|rule| evaluate_rule(rule, pair, values, max_work))
        .collect::<Result<Vec<_>, _>>()?;
    evidence.sort_by(|left, right| left.rule.as_bytes().cmp(right.rule.as_bytes()));
    Ok(evidence)
}

pub fn independent_agreeing_groups(evidence: &[EvidenceItem]) -> Vec<String> {
    let mut groups = evidence
        .iter()
        .filter(|item| item.class == EvidenceClass::Agree)
        .map(|item| item.evidence_group.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    groups
}

pub fn evidence_summary(evidence: &[EvidenceItem]) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::new();
    for item in evidence {
        if item.class == EvidenceClass::Agree {
            *result.entry(item.evidence_group.clone()).or_insert(0) += 1;
        }
    }
    result
}
