use std::collections::{BTreeMap, BTreeSet};

use pgrx::Uuid;
use serde_json::{Value, json};

use crate::candidate::exact::exact_key;
use crate::candidate::prefix::prefix_key;
use crate::candidate::token::token_keys;
use crate::definition::{Entity, MatchRule};
use crate::error::MdmError;

pub mod exact;
pub mod prefix;
pub mod token;

#[cfg(test)]
pub mod oracle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Exact,
    CompositeExact,
    Prefix,
    Token,
}

impl ChannelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::CompositeExact => "composite_exact",
            Self::Prefix => "prefix",
            Self::Token => "token",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateChannel {
    pub channel_id: String,
    pub kind: ChannelKind,
    pub fields: Vec<String>,
    pub prefix_length: Option<usize>,
    pub token_min_length: Option<usize>,
    pub owners: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidatePlan {
    pub channels: Vec<CandidateChannel>,
}

impl CandidatePlan {
    pub fn from_entity(entity: &Entity) -> Result<Self, MdmError> {
        let mut channels = entity
            .matches
            .iter()
            .filter(|rule| rule.candidate.is_some())
            .map(channel_from_rule)
            .collect::<Result<Vec<_>, _>>()?;
        channels.sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
        let plan = Self { channels };
        plan.validate()?;
        Ok(plan)
    }

    pub fn from_json(value: &Value) -> Result<Self, MdmError> {
        if value.get("format_version").and_then(Value::as_u64) != Some(1) {
            return Err(MdmError::CandidateInvalid(
                "candidate plan format_version 1 is required".into(),
            ));
        }
        let channels = value
            .get("channels")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                MdmError::CandidateInvalid("candidate plan channels are required".into())
            })?
            .iter()
            .map(channel_from_json)
            .collect::<Result<Vec<_>, _>>()?;
        let mut plan = Self { channels };
        plan.channels
            .sort_by(|left, right| left.channel_id.cmp(&right.channel_id));
        plan.validate()?;
        Ok(plan)
    }

    fn validate(&self) -> Result<(), MdmError> {
        let mut channel_ids = BTreeSet::new();
        for channel in &self.channels {
            if channel.channel_id.is_empty()
                || channel.fields.is_empty()
                || channel.fields.iter().any(String::is_empty)
                || !channel_ids.insert(channel.channel_id.clone())
            {
                return Err(MdmError::CandidateInvalid(
                    "candidate channels must have unique IDs and at least one field".into(),
                ));
            }
            match channel.kind {
                ChannelKind::Exact | ChannelKind::Prefix | ChannelKind::Token
                    if channel.fields.len() != 1 =>
                {
                    return Err(MdmError::CandidateInvalid(format!(
                        "channel {} requires one field",
                        channel.channel_id
                    )));
                }
                ChannelKind::CompositeExact => {
                    let mut fields = BTreeSet::new();
                    if channel.fields.iter().any(|field| !fields.insert(field)) {
                        return Err(MdmError::CandidateInvalid(format!(
                            "channel {} has duplicate fields",
                            channel.channel_id
                        )));
                    }
                }
                _ => {}
            }
            if channel.kind == ChannelKind::Prefix
                && channel.prefix_length.is_none_or(|length| length == 0)
            {
                return Err(MdmError::CandidateInvalid(format!(
                    "channel {} requires a positive prefix length",
                    channel.channel_id
                )));
            }
            if channel.kind == ChannelKind::Token
                && channel.token_min_length.is_none_or(|length| length == 0)
            {
                return Err(MdmError::CandidateInvalid(format!(
                    "channel {} requires a positive token length",
                    channel.channel_id
                )));
            }
        }
        Ok(())
    }

    pub fn to_json(&self, limits: &CandidateLimits) -> Value {
        self.to_json_with_warning(
            limits,
            crate::semantics::DEFAULT_WARNING_BLOCK_RECORDS.min(limits.max_block_records),
        )
    }

    pub fn to_json_with_warning(
        &self,
        limits: &CandidateLimits,
        warning_block_records: usize,
    ) -> Value {
        json!({
            "format_version": 1,
            "channels": self.channels.iter().map(|channel| json!({
                "channel_id": channel.channel_id,
                "kind": channel.kind.as_str(),
                "fields": channel.fields,
                "parameters": channel.parameters(),
                "owners": channel.owners
            })).collect::<Vec<_>>(),
            "max_block_records": limits.max_block_records,
            "max_candidate_pairs": limits.max_candidate_pairs,
            "warning_block_records": warning_block_records
        })
    }
}

impl CandidateChannel {
    fn parameters(&self) -> Value {
        match self.kind {
            ChannelKind::Exact | ChannelKind::CompositeExact => json!({}),
            ChannelKind::Prefix => json!({"length": self.prefix_length}),
            ChannelKind::Token => json!({"min_length": self.token_min_length}),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedRecord {
    pub source_record_id: Uuid,
    pub source_sort_key: Vec<u8>,
    pub field: String,
    pub normalized: String,
    pub canonical_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidatePair {
    pub left_source_record_id: Uuid,
    pub right_source_record_id: Uuid,
    pub left_sort_key: Vec<u8>,
    pub right_sort_key: Vec<u8>,
    pub discovery_channels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateLimits {
    pub max_block_records: usize,
    pub max_candidate_pairs: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PairKey {
    left_sort_key: Vec<u8>,
    right_sort_key: Vec<u8>,
    left_source_record_id: Uuid,
    right_source_record_id: Uuid,
}

#[derive(Clone, Debug)]
struct Member<'a> {
    record: &'a NormalizedRecord,
}

fn channel_from_rule(rule: &MatchRule) -> Result<CandidateChannel, MdmError> {
    let candidate = rule.candidate.as_ref().ok_or_else(|| {
        MdmError::CandidateInvalid(format!("channel {} has no candidate", rule.name))
    })?;
    let object = candidate.as_object().ok_or_else(|| {
        MdmError::CandidateInvalid(format!("channel {} candidate must be an object", rule.name))
    })?;
    let kind = object.get("kind").and_then(Value::as_str).ok_or_else(|| {
        MdmError::CandidateInvalid(format!("channel {} kind is required", rule.name))
    })?;
    let fields = match kind {
        "exact" | "prefix" | "token" => {
            let field = match object.get("field") {
                Some(value) => value.as_str().ok_or_else(|| {
                    MdmError::CandidateInvalid(format!("channel {} field must be text", rule.name))
                })?,
                None => rule
                    .fields
                    .first()
                    .filter(|_| rule.fields.len() == 1)
                    .ok_or_else(|| {
                        MdmError::CandidateInvalid(format!(
                            "channel {} requires one field",
                            rule.name
                        ))
                    })?,
            };
            if rule.fields != [field.to_owned()] {
                return Err(MdmError::CandidateInvalid(format!(
                    "channel {} candidate field must match its match rule",
                    rule.name
                )));
            }
            vec![field.to_owned()]
        }
        "composite_exact" => {
            let fields = match object.get("fields") {
                Some(value) => value
                    .as_array()
                    .ok_or_else(|| {
                        MdmError::CandidateInvalid(format!(
                            "channel {} fields must be an array",
                            rule.name
                        ))
                    })?
                    .iter()
                    .map(|field| {
                        field.as_str().map(str::to_owned).ok_or_else(|| {
                            MdmError::CandidateInvalid(format!(
                                "channel {} field must be text",
                                rule.name
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                None => rule.fields.clone(),
            };
            if fields != rule.fields {
                return Err(MdmError::CandidateInvalid(format!(
                    "channel {} candidate fields must match its match rule",
                    rule.name
                )));
            }
            fields
        }
        other => {
            return Err(MdmError::CandidateInvalid(format!(
                "channel {} has unsupported kind {other}",
                rule.name
            )));
        }
    };
    let prefix_length = match kind {
        "prefix" => Some(parameter(object, "length", &rule.name)?),
        _ => None,
    };
    let token_min_length = match kind {
        "token" => Some(
            object
                .get("min_length")
                .or_else(|| object.get("min_token_length"))
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    MdmError::CandidateInvalid(format!(
                        "channel {} requires a positive min_length",
                        rule.name
                    ))
                })?,
        ),
        _ => None,
    };
    Ok(CandidateChannel {
        channel_id: rule.name.clone(),
        kind: match kind {
            "exact" => ChannelKind::Exact,
            "composite_exact" => ChannelKind::CompositeExact,
            "prefix" => ChannelKind::Prefix,
            "token" => ChannelKind::Token,
            _ => unreachable!("validated above"),
        },
        fields,
        prefix_length,
        token_min_length,
        owners: vec![rule.name.clone()],
    })
}

fn channel_from_json(value: &Value) -> Result<CandidateChannel, MdmError> {
    let object = value
        .as_object()
        .ok_or_else(|| MdmError::CandidateInvalid("candidate channel must be an object".into()))?;
    let channel_id = object
        .get("channel_id")
        .or_else(|| object.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| MdmError::CandidateInvalid("candidate channel_id is required".into()))?;
    let kind = object.get("kind").and_then(Value::as_str).ok_or_else(|| {
        MdmError::CandidateInvalid(format!("channel {channel_id} kind is required"))
    })?;
    let fields = object
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            MdmError::CandidateInvalid(format!("channel {channel_id} fields are required"))
        })?
        .iter()
        .map(|field| {
            field.as_str().map(str::to_owned).ok_or_else(|| {
                MdmError::CandidateInvalid(format!("channel {channel_id} field must be text"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let parameters = object
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let parameters = parameters.as_object().ok_or_else(|| {
        MdmError::CandidateInvalid(format!("channel {channel_id} parameters must be an object"))
    })?;
    let kind = match kind {
        "exact" => ChannelKind::Exact,
        "composite_exact" => ChannelKind::CompositeExact,
        "prefix" => ChannelKind::Prefix,
        "token" => ChannelKind::Token,
        other => {
            return Err(MdmError::CandidateInvalid(format!(
                "channel {channel_id} has unsupported kind {other}"
            )));
        }
    };
    let prefix_length = (kind == ChannelKind::Prefix)
        .then(|| parameter(parameters, "length", channel_id))
        .transpose()?;
    let token_min_length = (kind == ChannelKind::Token)
        .then(|| parameter(parameters, "min_length", channel_id))
        .transpose()?;
    Ok(CandidateChannel {
        channel_id: channel_id.to_owned(),
        kind,
        fields,
        prefix_length,
        token_min_length,
        owners: match object.get("owners") {
            Some(value) => value
                .as_array()
                .ok_or_else(|| {
                    MdmError::CandidateInvalid(format!(
                        "channel {channel_id} owners must be an array"
                    ))
                })?
                .iter()
                .map(|owner| {
                    owner.as_str().map(str::to_owned).ok_or_else(|| {
                        MdmError::CandidateInvalid(format!(
                            "channel {channel_id} owner must be text"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        },
    })
}

fn parameter(
    object: &serde_json::Map<String, Value>,
    name: &str,
    channel: &str,
) -> Result<usize, MdmError> {
    object
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            MdmError::CandidateInvalid(format!("channel {channel} requires a positive {name}"))
        })
}

fn validate_records(values: &[NormalizedRecord]) -> Result<BTreeMap<Uuid, Vec<u8>>, MdmError> {
    let mut sort_keys = BTreeMap::new();
    let mut seen_sort_keys = BTreeMap::<Vec<u8>, Uuid>::new();
    let mut seen_fields = BTreeSet::new();
    for value in values {
        if !seen_fields.insert((value.source_record_id, value.field.clone())) {
            return Err(MdmError::CandidateInvalid(
                "a source record has more than one normalized value for a field".into(),
            ));
        }
        if let Some(previous) =
            sort_keys.insert(value.source_record_id, value.source_sort_key.clone())
            && previous != value.source_sort_key
        {
            return Err(MdmError::CandidateInvalid(
                "a source record has multiple sort keys".into(),
            ));
        }
        if let Some(previous) =
            seen_sort_keys.insert(value.source_sort_key.clone(), value.source_record_id)
            && previous != value.source_record_id
        {
            return Err(MdmError::CandidateInvalid(
                "source-record sort keys must be unique".into(),
            ));
        }
    }
    Ok(sort_keys)
}

fn add_block<'a>(
    blocks: &mut BTreeMap<Vec<u8>, Vec<Member<'a>>>,
    key: Vec<u8>,
    record: &'a NormalizedRecord,
) -> Result<(), MdmError> {
    let members = blocks.entry(key).or_default();
    if members
        .iter()
        .any(|member| member.record.source_record_id == record.source_record_id)
    {
        return Err(MdmError::CandidateInvalid(
            "a source record occurs more than once in a candidate block".into(),
        ));
    }
    members.push(Member { record });
    Ok(())
}

fn add_pairs(
    pairs: &mut BTreeMap<PairKey, BTreeSet<String>>,
    members: &mut [Member<'_>],
    channel: &CandidateChannel,
    limits: &CandidateLimits,
) -> Result<(), MdmError> {
    members.sort_by(|left, right| {
        left.record
            .source_sort_key
            .cmp(&right.record.source_sort_key)
            .then_with(|| {
                left.record
                    .source_record_id
                    .cmp(&right.record.source_record_id)
            })
    });
    let count = members.len();
    let pair_count = count
        .checked_mul(count.saturating_sub(1))
        .and_then(|value| value.checked_div(2))
        .ok_or_else(|| MdmError::CandidateCountOverflow {
            channel: channel.channel_id.clone(),
        })?;
    if pair_count > limits.max_candidate_pairs {
        return Err(MdmError::CandidateTotalLimit {
            candidate_pairs: pair_count,
            limit: limits.max_candidate_pairs,
        });
    }
    for left_index in 0..count {
        for right_index in left_index + 1..count {
            let left = members[left_index].record;
            let right = members[right_index].record;
            let key = PairKey {
                left_sort_key: left.source_sort_key.clone(),
                right_sort_key: right.source_sort_key.clone(),
                left_source_record_id: left.source_record_id,
                right_source_record_id: right.source_record_id,
            };
            if !pairs.contains_key(&key) {
                if pairs.len() >= limits.max_candidate_pairs {
                    return Err(MdmError::CandidateTotalLimit {
                        candidate_pairs: pairs.len().saturating_add(1),
                        limit: limits.max_candidate_pairs,
                    });
                }
                pairs.insert(key, BTreeSet::from([channel.channel_id.clone()]));
            } else if let Some(channels) = pairs.get_mut(&key) {
                channels.insert(channel.channel_id.clone());
            }
        }
    }
    Ok(())
}

pub fn generate_candidates(
    plan: &CandidatePlan,
    values: &[NormalizedRecord],
    limits: CandidateLimits,
) -> Result<Vec<CandidatePair>, MdmError> {
    plan.validate()?;
    if limits.max_block_records == 0 || limits.max_candidate_pairs == 0 {
        return Err(MdmError::CandidateInvalid(
            "candidate limits must be positive".into(),
        ));
    }
    validate_records(values)?;
    let mut pairs = BTreeMap::<PairKey, BTreeSet<String>>::new();
    for channel in &plan.channels {
        let mut blocks = BTreeMap::<Vec<u8>, Vec<Member<'_>>>::new();
        match channel.kind {
            ChannelKind::Exact => {
                let field = &channel.fields[0];
                for record in values.iter().filter(|record| &record.field == field) {
                    add_block(&mut blocks, exact_key(&record.canonical_bytes), record)?;
                }
            }
            ChannelKind::CompositeExact => {
                let mut records = BTreeMap::<Uuid, BTreeMap<&str, &NormalizedRecord>>::new();
                for record in values
                    .iter()
                    .filter(|record| channel.fields.contains(&record.field))
                {
                    let fields = records.entry(record.source_record_id).or_default();
                    if fields.insert(record.field.as_str(), record).is_some() {
                        return Err(MdmError::CandidateInvalid(
                            "a composite channel has duplicate field values".into(),
                        ));
                    }
                }
                for (_record_id, fields) in records {
                    if fields.len() != channel.fields.len() {
                        continue;
                    }
                    let record = fields
                        .get(channel.fields[0].as_str())
                        .expect("composite field count was checked");
                    let key = channel
                        .fields
                        .iter()
                        .flat_map(|field| {
                            fields
                                .get(field.as_str())
                                .map(|value| &value.canonical_bytes)
                        })
                        .fold(Vec::new(), |mut key, bytes| {
                            key.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                            key.extend_from_slice(bytes);
                            key
                        });
                    add_block(&mut blocks, key, record)?;
                }
            }
            ChannelKind::Prefix => {
                let field = &channel.fields[0];
                let length = channel.prefix_length.expect("validated prefix length");
                for record in values.iter().filter(|record| &record.field == field) {
                    add_block(&mut blocks, prefix_key(&record.normalized, length), record)?;
                }
            }
            ChannelKind::Token => {
                let field = &channel.fields[0];
                let min_length = channel.token_min_length.expect("validated token length");
                for record in values.iter().filter(|record| &record.field == field) {
                    for token in token_keys(&record.normalized, min_length) {
                        add_block(&mut blocks, token, record)?;
                    }
                }
            }
        }
        for members in blocks.values_mut() {
            if members.len() > limits.max_block_records {
                return Err(MdmError::CandidateBlockLimit {
                    channel: channel.channel_id.clone(),
                    cardinality: members.len(),
                    limit: limits.max_block_records,
                });
            }
            add_pairs(&mut pairs, members, channel, &limits)?;
        }
    }

    Ok(pairs
        .into_iter()
        .map(|(key, channels)| CandidatePair {
            left_source_record_id: key.left_source_record_id,
            right_source_record_id: key.right_source_record_id,
            left_sort_key: key.left_sort_key,
            right_sort_key: key.right_sort_key,
            discovery_channels: channels.into_iter().collect(),
        })
        .collect())
}

pub fn plan_json(entity: &Entity, limits: &CandidateLimits) -> Result<Value, MdmError> {
    Ok(CandidatePlan::from_entity(entity)?.to_json(limits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::oracle::all_pairs_oracle;
    use proptest::prelude::*;

    fn id(value: u8) -> Uuid {
        Uuid::from_bytes([value; 16])
    }

    fn record(id_value: u8, field: &str, normalized: &str) -> NormalizedRecord {
        NormalizedRecord {
            source_record_id: id(id_value),
            source_sort_key: vec![id_value],
            field: field.into(),
            normalized: normalized.into(),
            canonical_bytes: normalized.as_bytes().to_vec(),
        }
    }

    fn limits() -> CandidateLimits {
        CandidateLimits {
            max_block_records: 10,
            max_candidate_pairs: 100,
        }
    }

    #[test]
    fn all_channels_union_deterministically() {
        let plan = CandidatePlan {
            channels: vec![
                CandidateChannel {
                    channel_id: "token_name".into(),
                    kind: ChannelKind::Token,
                    fields: vec!["name".into()],
                    prefix_length: None,
                    token_min_length: Some(4),
                    owners: vec!["token_name".into()],
                },
                CandidateChannel {
                    channel_id: "exact_email".into(),
                    kind: ChannelKind::Exact,
                    fields: vec!["email".into()],
                    prefix_length: None,
                    token_min_length: None,
                    owners: vec!["exact_email".into()],
                },
            ],
        };
        let values = vec![
            record(1, "name", "alice smith"),
            record(1, "email", "alice@example.com"),
            record(2, "name", "alice jones"),
            record(2, "email", "alice@example.com"),
            record(3, "name", "bob jones"),
            record(3, "email", "bob@example.com"),
        ];
        let generated = generate_candidates(&plan, &values, limits()).unwrap();
        let expected = all_pairs_oracle(&plan, &values);
        assert_eq!(generated, expected);
        assert_eq!(generated.len(), 2);
        assert_eq!(
            generated[0].discovery_channels,
            vec!["exact_email", "token_name"]
        );
    }

    #[test]
    fn composite_requires_every_value_and_prefix_counts_scalars() {
        let plan = CandidatePlan {
            channels: vec![
                CandidateChannel {
                    channel_id: "composite".into(),
                    kind: ChannelKind::CompositeExact,
                    fields: vec!["first".into(), "last".into()],
                    prefix_length: None,
                    token_min_length: None,
                    owners: vec!["composite".into()],
                },
                CandidateChannel {
                    channel_id: "prefix".into(),
                    kind: ChannelKind::Prefix,
                    fields: vec!["name".into()],
                    prefix_length: Some(2),
                    token_min_length: None,
                    owners: vec!["prefix".into()],
                },
            ],
        };
        let values = vec![
            record(1, "first", "zoe"),
            record(1, "last", "lee"),
            record(1, "name", "Åsa"),
            record(2, "first", "zoe"),
            record(2, "name", "Åke"),
            record(3, "first", "zoe"),
            record(3, "last", "lee"),
            record(3, "name", "Bob"),
        ];
        let generated = generate_candidates(&plan, &values, limits()).unwrap();
        assert_eq!(generated.len(), 1);
        assert_eq!(generated[0].discovery_channels, vec!["composite"]);
    }

    #[test]
    fn limits_fail_before_returning_partial_pairs() {
        let plan = CandidatePlan {
            channels: vec![CandidateChannel {
                channel_id: "exact".into(),
                kind: ChannelKind::Exact,
                fields: vec!["email".into()],
                prefix_length: None,
                token_min_length: None,
                owners: vec!["exact".into()],
            }],
        };
        let values = (1..=3)
            .map(|value| record(value, "email", "same@example.com"))
            .collect::<Vec<_>>();
        let error = generate_candidates(
            &plan,
            &values,
            CandidateLimits {
                max_block_records: 2,
                max_candidate_pairs: 100,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "MDM_CANDIDATE_BLOCK_LIMIT");
        assert!(
            generate_candidates(
                &plan,
                &values[..2],
                CandidateLimits {
                    max_block_records: 2,
                    max_candidate_pairs: 1,
                }
            )
            .is_ok()
        );
    }

    proptest::proptest! {
        #[test]
        fn generated_exact_candidates_match_independent_oracle(values in proptest::collection::vec(any::<u8>(), 1..8)) {
            let plan = CandidatePlan {
                channels: vec![CandidateChannel {
                    channel_id: "exact".into(),
                    kind: ChannelKind::Exact,
                    fields: vec!["email".into()],
                    prefix_length: None,
                    token_min_length: None,
                    owners: vec!["exact".into()],
                }],
            };
            let values = values
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    record(index as u8 + 1, "email", &format!("v{}", value % 3))
                })
                .collect::<Vec<_>>();
            prop_assert_eq!(
                generate_candidates(&plan, &values, limits()).unwrap(),
                all_pairs_oracle(&plan, &values),
            );
        }
    }
}
