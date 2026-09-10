use std::collections::BTreeSet;

use super::{CandidateChannel, CandidatePair, CandidatePlan, ChannelKind, NormalizedRecord};

pub(crate) fn all_pairs_oracle(
    plan: &CandidatePlan,
    values: &[NormalizedRecord],
) -> Vec<CandidatePair> {
    let mut records = values
        .iter()
        .map(|value| (value.source_record_id, value.source_sort_key.clone()))
        .collect::<Vec<_>>();
    records.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    records.dedup();

    let mut pairs = Vec::new();
    for left_index in 0..records.len() {
        for right_index in left_index + 1..records.len() {
            let (left_id, left_sort_key) = &records[left_index];
            let (right_id, right_sort_key) = &records[right_index];
            let channels = plan
                .channels
                .iter()
                .filter(|channel| channel_matches(channel, values, *left_id, *right_id))
                .map(|channel| channel.channel_id.clone())
                .collect::<BTreeSet<_>>();
            if !channels.is_empty() {
                pairs.push(CandidatePair {
                    left_source_record_id: *left_id,
                    right_source_record_id: *right_id,
                    left_sort_key: left_sort_key.clone(),
                    right_sort_key: right_sort_key.clone(),
                    discovery_channels: channels.into_iter().collect(),
                });
            }
        }
    }
    pairs
}

fn channel_matches(
    channel: &CandidateChannel,
    values: &[NormalizedRecord],
    left_id: pgrx::Uuid,
    right_id: pgrx::Uuid,
) -> bool {
    match channel.kind {
        ChannelKind::Exact => matching_field(values, &channel.fields[0], left_id, right_id)
            .is_some_and(|(left, right)| left.canonical_bytes == right.canonical_bytes),
        ChannelKind::CompositeExact => channel.fields.iter().all(|field| {
            matching_field(values, field, left_id, right_id)
                .is_some_and(|(left, right)| left.canonical_bytes == right.canonical_bytes)
        }),
        ChannelKind::Prefix => matching_field(values, &channel.fields[0], left_id, right_id)
            .is_some_and(|(left, right)| {
                left.normalized
                    .chars()
                    .take(channel.prefix_length.unwrap())
                    .eq(right
                        .normalized
                        .chars()
                        .take(channel.prefix_length.unwrap()))
            }),
        ChannelKind::Token => matching_field(values, &channel.fields[0], left_id, right_id)
            .is_some_and(|(left, right)| {
                let min_length = channel.token_min_length.unwrap();
                let left_tokens = left
                    .normalized
                    .split_whitespace()
                    .filter(|token| token.chars().count() >= min_length)
                    .collect::<BTreeSet<_>>();
                right
                    .normalized
                    .split_whitespace()
                    .filter(|token| token.chars().count() >= min_length)
                    .any(|token| left_tokens.contains(token))
            }),
    }
}

fn matching_field<'a>(
    values: &'a [NormalizedRecord],
    field: &str,
    left_id: pgrx::Uuid,
    right_id: pgrx::Uuid,
) -> Option<(&'a NormalizedRecord, &'a NormalizedRecord)> {
    let left = values
        .iter()
        .find(|value| value.source_record_id == left_id && value.field == field)?;
    let right = values
        .iter()
        .find(|value| value.source_record_id == right_id && value.field == field)?;
    Some((left, right))
}
