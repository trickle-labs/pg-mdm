use std::cmp::Ordering;
use std::collections::BTreeMap;

use pgrx::Uuid;
use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::error::MdmError;
use crate::normalization::NormalizedState;

pub const GOLDEN_POLICY_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GoldenCandidate {
    #[serde(serialize_with = "serialize_uuid")]
    pub source_record_id: Uuid,
    pub source_name: String,
    /// Lower values have higher declared source priority.
    pub source_priority: u32,
    pub row_changed_at: Option<i64>,
    pub authoritative: bool,
    pub source_sort_key: Vec<u8>,
    pub raw_value: Option<Value>,
    pub state: NormalizedState,
    pub normalized: Option<String>,
    pub canonical_bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoldenOverride {
    pub anchor_source_record_id: Uuid,
    pub directive_id: Uuid,
    pub created_at: i64,
    pub raw_value: Value,
    pub normalized: Option<String>,
    pub canonical_bytes: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldenStatus {
    Selected,
    Override,
    OverrideConflict,
    NoValue,
}

impl GoldenStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Selected => "selected",
            Self::Override => "override",
            Self::OverrideConflict => "override_conflict",
            Self::NoValue => "no_value",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldenIssue {
    Tie,
    InvalidAuthoritativeValue,
    OverrideConflict,
    OverrideAnchorInactive,
}

impl GoldenIssue {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tie => "GOLDEN_TIE",
            Self::InvalidAuthoritativeValue => "INVALID_AUTHORITATIVE_VALUE",
            Self::OverrideConflict => "GOLDEN_OVERRIDE_CONFLICT",
            Self::OverrideAnchorInactive => "GOLDEN_OVERRIDE_ANCHOR_INACTIVE",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoldenSelection {
    pub value: Option<Value>,
    pub normalized: Option<String>,
    pub canonical_bytes: Option<Vec<u8>>,
    pub status: GoldenStatus,
    pub winning_source_record_id: Option<Uuid>,
    pub policy: String,
    pub policy_version: u16,
    pub tie_break: String,
    pub contributors: Vec<Uuid>,
    pub issues: Vec<GoldenIssue>,
}

fn serialize_uuid<S>(value: &Uuid, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

fn usable(candidate: &GoldenCandidate) -> bool {
    candidate.state == NormalizedState::Value
        && candidate
            .raw_value
            .as_ref()
            .is_some_and(|value| !value.is_null())
        && candidate.normalized.is_some()
        && candidate.canonical_bytes.is_some()
}

fn source_order(left: &GoldenCandidate, right: &GoldenCandidate) -> Ordering {
    left.source_priority
        .cmp(&right.source_priority)
        .then_with(|| left.source_sort_key.cmp(&right.source_sort_key))
        .then_with(|| left.source_record_id.cmp(&right.source_record_id))
}

fn provenance_order(left: &GoldenCandidate, right: &GoldenCandidate) -> Ordering {
    left.source_sort_key
        .cmp(&right.source_sort_key)
        .then_with(|| left.source_record_id.cmp(&right.source_record_id))
}

fn base_selection(policy: &str, tie_break: &str) -> GoldenSelection {
    GoldenSelection {
        value: None,
        normalized: None,
        canonical_bytes: None,
        status: GoldenStatus::NoValue,
        winning_source_record_id: None,
        policy: policy.into(),
        policy_version: GOLDEN_POLICY_VERSION,
        tie_break: tie_break.into(),
        contributors: Vec::new(),
        issues: Vec::new(),
    }
}

fn invalid_authoritative_issue(records: &[GoldenCandidate], selection: &mut GoldenSelection) {
    if records
        .iter()
        .any(|record| record.authoritative && record.state == NormalizedState::Invalid)
    {
        selection
            .issues
            .push(GoldenIssue::InvalidAuthoritativeValue);
    }
}

fn selected_from_group(
    policy: &str,
    tie_break: &str,
    winner: &GoldenCandidate,
    group: &[&GoldenCandidate],
) -> GoldenSelection {
    let mut contributors = group.to_vec();
    contributors.sort_by(|left, right| provenance_order(left, right));
    GoldenSelection {
        value: winner.raw_value.clone(),
        normalized: winner.normalized.clone(),
        canonical_bytes: winner.canonical_bytes.clone(),
        status: GoldenStatus::Selected,
        winning_source_record_id: Some(winner.source_record_id),
        policy: policy.into(),
        policy_version: GOLDEN_POLICY_VERSION,
        tie_break: tie_break.into(),
        contributors: contributors
            .into_iter()
            .map(|record| record.source_record_id)
            .collect(),
        issues: Vec::new(),
    }
}

fn valid_records(records: &[GoldenCandidate]) -> Vec<&GoldenCandidate> {
    records.iter().filter(|record| usable(record)).collect()
}

pub fn prefer_source(records: &[GoldenCandidate]) -> GoldenSelection {
    let mut selection = base_selection(
        "prefer_source",
        "source_priority,row_changed_at_desc,source_sort_key",
    );
    invalid_authoritative_issue(records, &mut selection);
    let mut valid = valid_records(records);
    valid.sort_by(|left, right| {
        left.source_priority
            .cmp(&right.source_priority)
            .then_with(|| right.row_changed_at.cmp(&left.row_changed_at))
            .then_with(|| left.source_sort_key.cmp(&right.source_sort_key))
            .then_with(|| left.source_record_id.cmp(&right.source_record_id))
    });
    let Some(winner) = valid.first().copied() else {
        return selection;
    };
    let group: Vec<_> = valid
        .iter()
        .copied()
        .filter(|record| record.canonical_bytes == winner.canonical_bytes)
        .collect();
    let mut result = selected_from_group(
        "prefer_source",
        "source_priority,row_changed_at_desc,source_sort_key",
        winner,
        &group,
    );
    result.issues = selection.issues;
    result
}

pub fn latest(records: &[GoldenCandidate]) -> GoldenSelection {
    let mut selection = base_selection(
        "latest",
        "row_changed_at_desc,source_priority,source_sort_key",
    );
    invalid_authoritative_issue(records, &mut selection);
    let mut valid: Vec<_> = valid_records(records)
        .into_iter()
        .filter(|record| record.row_changed_at.is_some())
        .collect();
    valid.sort_by(|left, right| {
        right
            .row_changed_at
            .cmp(&left.row_changed_at)
            .then_with(|| left.source_priority.cmp(&right.source_priority))
            .then_with(|| left.source_sort_key.cmp(&right.source_sort_key))
            .then_with(|| left.source_record_id.cmp(&right.source_record_id))
    });
    let Some(winner) = valid.first().copied() else {
        return selection;
    };
    let group: Vec<_> = valid
        .iter()
        .copied()
        .filter(|record| record.canonical_bytes == winner.canonical_bytes)
        .collect();
    let mut result = selected_from_group(
        "latest",
        "row_changed_at_desc,source_priority,source_sort_key",
        winner,
        &group,
    );
    result.issues = selection.issues;
    result
}

pub fn first_non_null(records: &[GoldenCandidate]) -> GoldenSelection {
    let mut selection = base_selection("first_non_null", "source_priority,source_sort_key");
    invalid_authoritative_issue(records, &mut selection);
    let mut valid = valid_records(records);
    valid.sort_by(|left, right| source_order(left, right));
    let Some(winner) = valid.first().copied() else {
        return selection;
    };
    let group: Vec<_> = valid
        .iter()
        .copied()
        .filter(|record| record.canonical_bytes == winner.canonical_bytes)
        .collect();
    let mut result = selected_from_group(
        "first_non_null",
        "source_priority,source_sort_key",
        winner,
        &group,
    );
    result.issues = selection.issues;
    result
}

pub fn most_common(records: &[GoldenCandidate]) -> GoldenSelection {
    let mut selection = base_selection(
        "most_common",
        "count_desc,authoritative_count_desc,source_priority,canonical_bytes",
    );
    invalid_authoritative_issue(records, &mut selection);
    let mut groups: BTreeMap<Vec<u8>, Vec<&GoldenCandidate>> = BTreeMap::new();
    for record in valid_records(records) {
        groups
            .entry(
                record
                    .canonical_bytes
                    .clone()
                    .expect("usable canonical bytes"),
            )
            .or_default()
            .push(record);
    }
    let Some(max_count) = groups.values().map(Vec::len).max() else {
        return selection;
    };
    if groups
        .values()
        .filter(|group| group.len() == max_count)
        .count()
        > 1
    {
        selection.issues.push(GoldenIssue::Tie);
    }
    let (_, group) = groups
        .into_iter()
        .max_by(|(left_bytes, left_group), (right_bytes, right_group)| {
            left_group
                .len()
                .cmp(&right_group.len())
                .then_with(|| {
                    left_group
                        .iter()
                        .filter(|record| record.authoritative)
                        .count()
                        .cmp(
                            &right_group
                                .iter()
                                .filter(|record| record.authoritative)
                                .count(),
                        )
                })
                .then_with(|| {
                    right_group
                        .iter()
                        .min_by(|left, right| source_order(left, right))
                        .expect("non-empty group")
                        .source_priority
                        .cmp(
                            &left_group
                                .iter()
                                .min_by(|left, right| source_order(left, right))
                                .expect("non-empty group")
                                .source_priority,
                        )
                })
                .then_with(|| right_bytes.cmp(left_bytes))
        })
        .expect("groups are non-empty");
    let winner = group
        .iter()
        .min_by(|left, right| source_order(left, right))
        .copied()
        .expect("selected group is non-empty");
    let mut result = selected_from_group(
        "most_common",
        "count_desc,authoritative_count_desc,source_priority,canonical_bytes",
        winner,
        &group,
    );
    result.issues = selection.issues;
    result
}

fn built_in(policy: &str, records: &[GoldenCandidate]) -> Result<GoldenSelection, MdmError> {
    match policy {
        "prefer_source" => Ok(prefer_source(records)),
        "latest" => Ok(latest(records)),
        "first_non_null" => Ok(first_non_null(records)),
        "most_common" => Ok(most_common(records)),
        other => Err(MdmError::GoldenInvalid(format!(
            "unsupported golden policy {other}"
        ))),
    }
}

fn override_order(left: &GoldenOverride, right: &GoldenOverride) -> Ordering {
    left.created_at
        .cmp(&right.created_at)
        .then_with(|| left.directive_id.cmp(&right.directive_id))
}

pub fn select_golden(
    policy: &str,
    records: &[GoldenCandidate],
    overrides: &[GoldenOverride],
) -> Result<GoldenSelection, MdmError> {
    let mut selection = built_in(policy, records)?;
    let mut active = Vec::new();
    let mut has_inactive_anchor = false;
    for directive in overrides {
        if records
            .iter()
            .any(|record| record.source_record_id == directive.anchor_source_record_id)
        {
            active.push(directive);
        } else {
            has_inactive_anchor = true;
        }
    }
    if has_inactive_anchor {
        selection.issues.push(GoldenIssue::OverrideAnchorInactive);
    }
    if active.is_empty() {
        return Ok(selection);
    }

    let first_value = &active[0].raw_value;
    if active
        .iter()
        .any(|directive| &directive.raw_value != first_value)
    {
        selection.value = None;
        selection.normalized = None;
        selection.canonical_bytes = None;
        selection.status = GoldenStatus::OverrideConflict;
        selection.winning_source_record_id = None;
        selection.tie_break = "override_value".into();
        selection.contributors = active
            .iter()
            .map(|directive| directive.anchor_source_record_id)
            .collect();
        selection.contributors.sort();
        selection.issues.push(GoldenIssue::OverrideConflict);
        return Ok(selection);
    }

    let chosen = active
        .iter()
        .min_by(|left, right| override_order(left, right))
        .expect("active overrides are non-empty");
    selection.value = Some(chosen.raw_value.clone());
    selection.normalized = chosen.normalized.clone();
    selection.canonical_bytes = chosen.canonical_bytes.clone();
    selection.status = GoldenStatus::Override;
    selection.winning_source_record_id = Some(chosen.anchor_source_record_id);
    selection.tie_break = "directive_created_at,directive_id".into();
    selection.contributors = active
        .iter()
        .map(|directive| directive.anchor_source_record_id)
        .collect();
    selection.contributors.sort();
    Ok(selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(value: u8) -> Uuid {
        Uuid::from_bytes([value; 16])
    }

    fn record(
        id_value: u8,
        priority: u32,
        sort_key: u8,
        raw: &str,
        changed_at: Option<i64>,
    ) -> GoldenCandidate {
        GoldenCandidate {
            source_record_id: id(id_value),
            source_name: format!("source-{priority}"),
            source_priority: priority,
            row_changed_at: changed_at,
            authoritative: false,
            source_sort_key: vec![sort_key],
            raw_value: Some(json!(raw)),
            state: NormalizedState::Value,
            normalized: Some(raw.into()),
            canonical_bytes: Some(raw.as_bytes().to_vec()),
        }
    }

    fn directive(id_value: u8, anchor: u8, created_at: i64, raw: &str) -> GoldenOverride {
        GoldenOverride {
            anchor_source_record_id: id(anchor),
            directive_id: id(id_value),
            created_at,
            raw_value: json!(raw),
            normalized: Some(raw.into()),
            canonical_bytes: Some(raw.as_bytes().to_vec()),
        }
    }

    #[test]
    fn policies_follow_their_declared_order() {
        let records = vec![
            record(1, 2, 1, "late", Some(30)),
            record(2, 1, 2, "priority", Some(40)),
            record(3, 1, 1, "newer", Some(40)),
        ];
        assert_eq!(
            prefer_source(&records).winning_source_record_id,
            Some(id(3))
        );
        assert_eq!(latest(&records).winning_source_record_id, Some(id(3)));
        assert_eq!(
            first_non_null(&records).winning_source_record_id,
            Some(id(3))
        );
    }

    #[test]
    fn most_common_uses_authority_then_priority_and_reports_ties() {
        let mut records = vec![
            record(1, 2, 1, "a", Some(1)),
            record(2, 1, 2, "b", Some(2)),
            record(3, 1, 3, "b", Some(3)),
            record(4, 2, 4, "a", Some(4)),
        ];
        records[0].authoritative = true;
        let selected = most_common(&records);
        assert_eq!(selected.normalized.as_deref(), Some("a"));
        assert_eq!(selected.winning_source_record_id, Some(id(1)));
        assert_eq!(selected.issues, vec![GoldenIssue::Tie]);
        assert_eq!(selected.issues[0].as_str(), "GOLDEN_TIE");

        records[2].normalized = Some("a".into());
        records[2].canonical_bytes = Some(b"a".to_vec());
        let selected = most_common(&records);
        assert!(!selected.issues.contains(&GoldenIssue::Tie));
    }

    #[test]
    fn invalid_authoritative_values_are_reported_but_not_selected() {
        let mut invalid = record(1, 1, 1, "bad", Some(1));
        invalid.state = NormalizedState::Invalid;
        invalid.authoritative = true;
        let selected = first_non_null(&[invalid, record(2, 2, 2, "good", Some(2))]);
        assert_eq!(selected.normalized.as_deref(), Some("good"));
        assert_eq!(
            selected.issues,
            vec![GoldenIssue::InvalidAuthoritativeValue]
        );
        assert_eq!(selected.status.as_str(), "selected");
    }

    #[test]
    fn overrides_cover_conflict_and_inactive_anchor_paths() {
        let records = vec![record(1, 1, 1, "built-in", Some(1))];
        let conflict = select_golden(
            "first_non_null",
            &records,
            &[directive(10, 1, 1, "x"), directive(11, 1, 2, "y")],
        )
        .unwrap();
        assert_eq!(conflict.status, GoldenStatus::OverrideConflict);
        assert_eq!(conflict.value, None);
        assert!(conflict.issues.contains(&GoldenIssue::OverrideConflict));

        let inactive =
            select_golden("first_non_null", &records, &[directive(10, 9, 1, "x")]).unwrap();
        assert_eq!(inactive.status, GoldenStatus::Selected);
        assert_eq!(inactive.normalized.as_deref(), Some("built-in"));
        assert!(
            inactive
                .issues
                .contains(&GoldenIssue::OverrideAnchorInactive)
        );
    }

    #[test]
    fn equal_overrides_use_oldest_then_directive_id_and_provenance_is_stable() {
        let records = vec![
            record(2, 2, 2, "built-in", Some(1)),
            record(1, 1, 1, "built-in", Some(1)),
        ];
        let overrides = vec![directive(8, 2, 5, "forced"), directive(7, 1, 5, "forced")];
        let forward = select_golden("first_non_null", &records, &overrides).unwrap();
        let reverse = select_golden(
            "first_non_null",
            &records,
            &[overrides[1].clone(), overrides[0].clone()],
        )
        .unwrap();
        assert_eq!(forward, reverse);
        assert_eq!(forward.winning_source_record_id, Some(id(1)));
        assert_eq!(forward.contributors, vec![id(1), id(2)]);
    }
}
