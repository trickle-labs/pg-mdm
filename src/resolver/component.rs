use std::collections::{BTreeMap, BTreeSet};

use pgrx::Uuid;

use super::ResolverRecord;

#[derive(Clone, Debug, Default)]
pub struct Component {
    pub members: BTreeSet<Uuid>,
    pub authority: BTreeMap<String, BTreeSet<String>>,
    key: Vec<u8>,
}

impl Component {
    pub fn singleton(record: &ResolverRecord) -> Self {
        let mut authority = BTreeMap::new();
        for (field, value) in &record.authority {
            if !value.is_empty() {
                authority
                    .entry(field.clone())
                    .or_insert_with(BTreeSet::new)
                    .insert(value.clone());
            }
        }
        Self {
            members: BTreeSet::from([record.source_record_id]),
            authority,
            key: record.source_sort_key.clone(),
        }
    }

    pub fn is_singleton(&self) -> bool {
        self.members.len() == 1
    }

    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub fn merge(&mut self, other: Self) {
        if other.members.is_empty() {
            return;
        }
        self.members.extend(other.members);
        for (field, values) in other.authority {
            self.authority.entry(field).or_default().extend(values);
        }
        if other.key < self.key {
            self.key = other.key;
        }
    }
}
