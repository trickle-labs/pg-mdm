use std::collections::{BTreeMap, BTreeSet};

use pgrx::Uuid;
use serde::{Serialize, Serializer};

use crate::error::MdmError;
use crate::resolver::Resolution;

/// Supplies IDs for components which have no continuity anchor.
pub trait IdAllocator {
    fn next_id(&mut self) -> Uuid;
}

impl<F> IdAllocator for F
where
    F: FnMut() -> Uuid,
{
    fn next_id(&mut self) -> Uuid {
        self()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityStatus {
    Active,
    Merged,
    Split,
    Retired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IdentityRecord {
    #[serde(serialize_with = "serialize_uuid")]
    pub mdm_id: Uuid,
    pub created_revision: i64,
    pub retired_revision: Option<i64>,
    pub status: IdentityStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IdentityMembership {
    #[serde(serialize_with = "serialize_uuid")]
    pub source_record_id: Uuid,
    pub source_sort_key: Vec<u8>,
    #[serde(serialize_with = "serialize_uuid")]
    pub mdm_id: Uuid,
    pub active: bool,
    pub first_membership_revision: i64,
    pub last_membership_revision: i64,
    pub membership_reason: String,
    pub last_change_revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IdentityAlias {
    #[serde(serialize_with = "serialize_uuid")]
    pub alias_mdm_id: Uuid,
    #[serde(serialize_with = "serialize_uuid")]
    pub canonical_mdm_id: Uuid,
    pub publication_revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IdentitySplit {
    #[serde(serialize_with = "serialize_uuid")]
    pub parent_mdm_id: Uuid,
    #[serde(serialize_with = "serialize_uuid")]
    pub child_mdm_id: Uuid,
    pub publication_revision: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct IdentityState {
    pub registry: Vec<IdentityRecord>,
    pub memberships: Vec<IdentityMembership>,
    pub aliases: Vec<IdentityAlias>,
    pub splits: Vec<IdentitySplit>,
}

pub type PublishedIdentityState = IdentityState;
pub type IdentityResolution = IdentityState;

fn serialize_uuid<S>(value: &Uuid, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_bytes(value.as_bytes())
}

#[derive(Default)]
pub struct TestAllocator {
    pub ids: Vec<Uuid>,
    pub next: usize,
}

impl IdAllocator for TestAllocator {
    fn next_id(&mut self) -> Uuid {
        let id = self
            .ids
            .get(self.next)
            .copied()
            .unwrap_or_else(|| Uuid::from_bytes([self.next as u8; 16]));
        self.next += 1;
        id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Component {
    key: Vec<u8>,
}

pub fn reconcile(
    old: &IdentityState,
    resolved: &Resolution,
    publication_revision: i64,
    allocator: &mut dyn IdAllocator,
) -> Result<IdentityState, MdmError> {
    if publication_revision <= 0 {
        return Err(MdmError::IdentityInvalid(
            "publication revision must be greater than zero".into(),
        ));
    }

    let registry = registry_map(old)?;
    let old_memberships = membership_map(old, &registry)?;
    validate_aliases(&old.aliases, &registry)?;

    let (components, resolved_by_record) = components(resolved)?;
    let resolved_sort_keys = resolved
        .memberships
        .iter()
        .map(|membership| {
            (
                membership.source_record_id,
                membership.source_sort_key.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut active_old_by_record = BTreeMap::new();
    for membership in old_memberships
        .values()
        .filter(|membership| membership.active)
    {
        if active_old_by_record
            .insert(membership.source_record_id, membership.mdm_id)
            .is_some()
        {
            return Err(MdmError::IdentityInvalid(
                "active source records must have one identity membership".into(),
            ));
        }
    }

    let mut anchors = BTreeMap::<Uuid, Uuid>::new();
    for membership in old_memberships
        .values()
        .filter(|membership| membership.active)
    {
        if !resolved_by_record.contains_key(&membership.source_record_id) {
            continue;
        }
        let replace = anchors
            .get(&membership.mdm_id)
            .map(|anchor| {
                let current = anchor_membership_key(membership, &resolved_by_record);
                let previous = anchor_membership_key(
                    old_memberships
                        .get(anchor)
                        .expect("anchor came from old memberships"),
                    &resolved_by_record,
                );
                current < previous
            })
            .unwrap_or(true);
        if replace {
            anchors.insert(membership.mdm_id, membership.source_record_id);
        }
    }

    let mut anchor_claims = BTreeMap::<Vec<u8>, Vec<Uuid>>::new();
    for (old_id, source_record_id) in &anchors {
        let component_key = resolved_by_record
            .get(source_record_id)
            .expect("anchor was checked against resolved records");
        anchor_claims
            .entry(component_key.clone())
            .or_default()
            .push(*old_id);
    }
    for claims in anchor_claims.values_mut() {
        claims.sort_by_key(|id| identity_order(*id, &registry));
    }

    let mut assigned = BTreeMap::<Vec<u8>, Uuid>::new();
    let mut used_ids = BTreeSet::new();
    for component in &components {
        let (mdm_id, is_allocated) = if let Some(id) = anchor_claims
            .get(&component.key)
            .and_then(|claims| claims.first().copied())
        {
            (id, false)
        } else {
            let id = allocator.next_id();
            if registry.contains_key(&id) || !used_ids.insert(id) {
                return Err(MdmError::IdentityInvalid(
                    "identity allocator returned a duplicate ID".into(),
                ));
            }
            (id, true)
        };
        if registry.contains_key(&mdm_id) && !is_active_id(mdm_id, &registry) {
            return Err(MdmError::IdentityInvalid(
                "identity continuity selected a non-active ID".into(),
            ));
        }
        if !is_allocated && !used_ids.insert(mdm_id) {
            return Err(MdmError::IdentityInvalid(
                "identity continuity selected an ID twice".into(),
            ));
        }
        if assigned.insert(component.key.clone(), mdm_id).is_some() {
            return Err(MdmError::IdentityInvalid(
                "component keys must be unique".into(),
            ));
        }
    }

    let mut next_registry = registry.clone();
    for component in &components {
        let mdm_id = assigned[&component.key];
        next_registry.entry(mdm_id).or_insert(IdentityRecord {
            mdm_id,
            created_revision: publication_revision,
            retired_revision: None,
            status: IdentityStatus::Active,
        });
    }

    let mut successors = BTreeMap::<Uuid, BTreeSet<Uuid>>::new();
    for (source_record_id, old_id) in &active_old_by_record {
        if let Some(component_key) = resolved_by_record.get(source_record_id) {
            successors
                .entry(*old_id)
                .or_default()
                .insert(assigned[component_key]);
        }
    }

    let active_old_ids = registry
        .values()
        .filter(|identity| identity.status == IdentityStatus::Active)
        .map(|identity| identity.mdm_id)
        .collect::<BTreeSet<_>>();
    for old_id in active_old_ids {
        let next = successors.get(&old_id).cloned().unwrap_or_default();
        let entry = next_registry.get_mut(&old_id).ok_or_else(|| {
            MdmError::IdentityInvalid("membership references unknown identity".into())
        })?;
        match next.len() {
            0 => {
                entry.status = IdentityStatus::Retired;
                entry.retired_revision = Some(publication_revision);
            }
            1 if next.contains(&old_id) => {
                entry.status = IdentityStatus::Active;
                entry.retired_revision = None;
            }
            1 => {
                entry.status = IdentityStatus::Merged;
                entry.retired_revision = Some(publication_revision);
            }
            _ if next.contains(&old_id) => {
                entry.status = IdentityStatus::Active;
                entry.retired_revision = None;
            }
            _ => {
                entry.status = IdentityStatus::Split;
                entry.retired_revision = Some(publication_revision);
            }
        }
    }

    let mut aliases = old.aliases.clone();
    let mut splits = old.splits.clone();
    for (old_id, next) in &successors {
        if next.len() == 1 {
            let canonical = *next.first().expect("one successor");
            if canonical != *old_id
                && !aliases.iter().any(|alias| {
                    alias.alias_mdm_id == *old_id && alias.canonical_mdm_id == canonical
                })
            {
                aliases.push(IdentityAlias {
                    alias_mdm_id: *old_id,
                    canonical_mdm_id: canonical,
                    publication_revision,
                });
            }
        } else if next.len() > 1 {
            for child in next {
                if !splits.iter().any(|split| {
                    split.parent_mdm_id == *old_id
                        && split.child_mdm_id == *child
                        && split.publication_revision == publication_revision
                }) {
                    splits.push(IdentitySplit {
                        parent_mdm_id: *old_id,
                        child_mdm_id: *child,
                        publication_revision,
                    });
                }
            }
        }
    }
    validate_aliases(&aliases, &next_registry)?;

    let mut memberships = old_memberships.into_values().collect::<Vec<_>>();
    for membership in &mut memberships {
        if let Some(component_key) = resolved_by_record.get(&membership.source_record_id) {
            let next_id = assigned[component_key];
            let was_active = membership.active;
            let changed = !was_active || membership.mdm_id != next_id;
            membership.mdm_id = next_id;
            membership.active = true;
            membership.source_sort_key = resolved_sort_keys
                .get(&membership.source_record_id)
                .expect("resolved membership index is complete")
                .clone();
            if changed {
                membership.last_membership_revision = publication_revision;
                membership.last_change_revision = publication_revision;
                membership.membership_reason = if was_active {
                    "reconciled".into()
                } else {
                    "reactivated".into()
                };
            }
        } else if membership.active {
            membership.active = false;
            membership.last_membership_revision = publication_revision;
            membership.last_change_revision = publication_revision;
            membership.membership_reason = "removed".into();
        }
    }
    let known_records = memberships
        .iter()
        .map(|membership| membership.source_record_id)
        .collect::<BTreeSet<_>>();
    for resolved_membership in &resolved.memberships {
        if known_records.contains(&resolved_membership.source_record_id) {
            continue;
        }
        memberships.push(IdentityMembership {
            source_record_id: resolved_membership.source_record_id,
            source_sort_key: resolved_membership.source_sort_key.clone(),
            mdm_id: assigned[&resolved_membership.component_key],
            active: true,
            first_membership_revision: publication_revision,
            last_membership_revision: publication_revision,
            membership_reason: "new".into(),
            last_change_revision: publication_revision,
        });
    }

    let mut registry = next_registry.into_values().collect::<Vec<_>>();
    registry.sort_by_key(|identity| identity.mdm_id);
    memberships.sort_by(|left, right| {
        left.source_sort_key
            .cmp(&right.source_sort_key)
            .then_with(|| left.source_record_id.cmp(&right.source_record_id))
    });
    aliases.sort_by_key(|alias| {
        (
            alias.alias_mdm_id,
            alias.publication_revision,
            alias.canonical_mdm_id,
        )
    });
    splits.sort_by_key(|split| {
        (
            split.parent_mdm_id,
            split.publication_revision,
            split.child_mdm_id,
        )
    });
    Ok(IdentityState {
        registry,
        memberships,
        aliases,
        splits,
    })
}

pub fn resolve_alias(aliases: &[IdentityAlias], start: Uuid) -> Result<Uuid, MdmError> {
    let mut by_alias = BTreeMap::new();
    for alias in aliases {
        if alias.alias_mdm_id == alias.canonical_mdm_id {
            return Err(MdmError::IdentityInvalid(
                "identity alias cannot point to itself".into(),
            ));
        }
        if by_alias
            .insert(alias.alias_mdm_id, alias.canonical_mdm_id)
            .is_some()
        {
            return Err(MdmError::IdentityInvalid(
                "identity alias has more than one canonical target".into(),
            ));
        }
    }
    let mut current = start;
    let mut visited = BTreeSet::new();
    while let Some(next) = by_alias.get(&current).copied() {
        if !visited.insert(current) {
            return Err(MdmError::IdentityInvalid(
                "identity alias chain contains a cycle".into(),
            ));
        }
        current = next;
    }
    Ok(current)
}

pub fn current_successors(
    state: &IdentityState,
    start: Uuid,
    max_work: usize,
) -> Result<Vec<Uuid>, MdmError> {
    if max_work == 0 {
        return Err(MdmError::IdentityInvalid(
            "identity history work limit must be greater than zero".into(),
        ));
    }
    validate_aliases(&state.aliases, &registry_map(state)?)?;
    let mut transitions = BTreeMap::<(Uuid, i64), Vec<Uuid>>::new();
    for alias in &state.aliases {
        transitions
            .entry((alias.alias_mdm_id, alias.publication_revision))
            .or_default()
            .push(alias.canonical_mdm_id);
    }
    for split in &state.splits {
        transitions
            .entry((split.parent_mdm_id, split.publication_revision))
            .or_default()
            .push(split.child_mdm_id);
    }
    for children in transitions.values_mut() {
        children.sort();
        children.dedup();
    }

    let mut frontier = vec![(start, i64::MIN)];
    let mut terminals = BTreeSet::new();
    let mut work = 0;
    while let Some((id, revision)) = frontier.pop() {
        work += 1;
        if work > max_work {
            return Err(MdmError::IdentityLimit {
                resource: "identity_history_work",
                observed: work,
                limit: max_work,
            });
        }
        let next = transitions
            .iter()
            .filter(|((parent, next_revision), _)| *parent == id && *next_revision > revision)
            .min_by_key(|((_, next_revision), _)| *next_revision)
            .map(|((_, next_revision), children)| (*next_revision, children.clone()));
        match next {
            Some((next_revision, children)) => {
                for child in children {
                    frontier.push((child, next_revision));
                }
            }
            None => {
                terminals.insert(id);
            }
        }
    }
    Ok(terminals.into_iter().collect())
}

fn registry_map(state: &IdentityState) -> Result<BTreeMap<Uuid, IdentityRecord>, MdmError> {
    let mut registry = BTreeMap::new();
    for identity in &state.registry {
        if registry.insert(identity.mdm_id, identity.clone()).is_some() {
            return Err(MdmError::IdentityInvalid(
                "identity registry IDs must be unique".into(),
            ));
        }
    }
    Ok(registry)
}

fn membership_map(
    state: &IdentityState,
    registry: &BTreeMap<Uuid, IdentityRecord>,
) -> Result<BTreeMap<Uuid, IdentityMembership>, MdmError> {
    let mut memberships = BTreeMap::new();
    for membership in &state.memberships {
        let Some(identity) = registry.get(&membership.mdm_id) else {
            return Err(MdmError::IdentityInvalid(
                "membership references an unknown identity".into(),
            ));
        };
        if membership.active && identity.status != IdentityStatus::Active {
            return Err(MdmError::IdentityInvalid(
                "active membership references a non-active identity".into(),
            ));
        }
        if memberships
            .insert(membership.source_record_id, membership.clone())
            .is_some()
        {
            return Err(MdmError::IdentityInvalid(
                "source record membership IDs must be unique".into(),
            ));
        }
    }
    Ok(memberships)
}

#[allow(clippy::type_complexity)]
fn components(
    resolved: &Resolution,
) -> Result<(Vec<Component>, BTreeMap<Uuid, Vec<u8>>), MdmError> {
    let mut by_key = BTreeMap::<Vec<u8>, Vec<Uuid>>::new();
    let mut by_record = BTreeMap::new();
    for membership in &resolved.memberships {
        if by_record
            .insert(
                membership.source_record_id,
                membership.component_key.clone(),
            )
            .is_some()
        {
            return Err(MdmError::IdentityInvalid(
                "resolved source records must have one membership".into(),
            ));
        }
        by_key
            .entry(membership.component_key.clone())
            .or_default()
            .push(membership.source_record_id);
    }
    let components = by_key.into_keys().map(|key| Component { key }).collect();
    Ok((components, by_record))
}

fn anchor_membership_key(
    membership: &IdentityMembership,
    _resolved_by_record: &BTreeMap<Uuid, Vec<u8>>,
) -> (i64, Vec<u8>, Uuid) {
    (
        membership.first_membership_revision,
        membership.source_sort_key.clone(),
        membership.source_record_id,
    )
}

fn identity_order(mdm_id: Uuid, registry: &BTreeMap<Uuid, IdentityRecord>) -> (i64, Uuid) {
    registry
        .get(&mdm_id)
        .map(|identity| (identity.created_revision, mdm_id))
        .unwrap_or((i64::MAX, mdm_id))
}

fn is_active_id(mdm_id: Uuid, registry: &BTreeMap<Uuid, IdentityRecord>) -> bool {
    registry
        .get(&mdm_id)
        .is_some_and(|identity| identity.status == IdentityStatus::Active)
}

fn validate_aliases(
    aliases: &[IdentityAlias],
    registry: &BTreeMap<Uuid, IdentityRecord>,
) -> Result<(), MdmError> {
    for alias in aliases {
        if !registry.contains_key(&alias.alias_mdm_id)
            || !registry.contains_key(&alias.canonical_mdm_id)
        {
            return Err(MdmError::IdentityInvalid(
                "identity alias references an unknown identity".into(),
            ));
        }
    }
    for alias in aliases {
        resolve_alias(aliases, alias.alias_mdm_id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::{Membership, Resolution};

    struct Allocator {
        ids: Vec<Uuid>,
        next: usize,
    }

    impl IdAllocator for Allocator {
        fn next_id(&mut self) -> Uuid {
            let id = self.ids[self.next];
            self.next += 1;
            id
        }
    }

    fn id(value: u8) -> Uuid {
        Uuid::from_bytes([value; 16])
    }

    fn resolution(rows: &[(u8, u8)]) -> Resolution {
        Resolution {
            memberships: rows
                .iter()
                .map(|(record, component)| Membership {
                    source_record_id: id(*record),
                    source_sort_key: vec![*record],
                    component_key: vec![*component],
                })
                .collect(),
            accepted: Vec::new(),
            rejected: Vec::new(),
        }
    }

    fn identity(mdm_id: u8, created_revision: i64) -> IdentityRecord {
        IdentityRecord {
            mdm_id: id(mdm_id),
            created_revision,
            retired_revision: None,
            status: IdentityStatus::Active,
        }
    }

    fn membership(record: u8, mdm_id: u8, first_revision: i64) -> IdentityMembership {
        IdentityMembership {
            source_record_id: id(record),
            source_sort_key: vec![record],
            mdm_id: id(mdm_id),
            active: true,
            first_membership_revision: first_revision,
            last_membership_revision: first_revision,
            membership_reason: "new".into(),
            last_change_revision: first_revision,
        }
    }

    #[test]
    fn new_and_unchanged_are_deterministic_and_allocator_is_injected() {
        let old = IdentityState {
            registry: vec![identity(10, 1)],
            memberships: vec![membership(1, 10, 1)],
            ..IdentityState::default()
        };
        let mut allocator = Allocator {
            ids: vec![id(20)],
            next: 0,
        };
        let result = reconcile(&old, &resolution(&[(2, 2), (1, 1)]), 2, &mut allocator).unwrap();
        assert_eq!(result.memberships[0].source_record_id, id(1));
        assert_eq!(result.memberships[1].mdm_id, id(20));
        assert_eq!(
            result
                .registry
                .iter()
                .map(|id| id.mdm_id)
                .collect::<Vec<_>>(),
            vec![id(10), id(20)]
        );

        let mut no_allocations = Allocator {
            ids: vec![],
            next: 0,
        };
        let unchanged = reconcile(&old, &resolution(&[(1, 1)]), 2, &mut no_allocations).unwrap();
        assert_eq!(unchanged.memberships[0].mdm_id, id(10));
        assert_eq!(unchanged.registry[0].status, IdentityStatus::Active);
    }

    #[test]
    fn merge_keeps_oldest_identity_and_adds_alias() {
        let old = IdentityState {
            registry: vec![identity(10, 1), identity(11, 2)],
            memberships: vec![membership(1, 10, 1), membership(2, 11, 2)],
            ..IdentityState::default()
        };
        let mut allocator = Allocator {
            ids: vec![],
            next: 0,
        };
        let result = reconcile(&old, &resolution(&[(1, 9), (2, 9)]), 3, &mut allocator).unwrap();
        assert_eq!(
            result
                .memberships
                .iter()
                .map(|row| row.mdm_id)
                .collect::<Vec<_>>(),
            vec![id(10), id(10)]
        );
        assert_eq!(result.registry[1].status, IdentityStatus::Merged);
        assert_eq!(result.aliases[0].alias_mdm_id, id(11));
        assert_eq!(resolve_alias(&result.aliases, id(11)).unwrap(), id(10));
    }

    #[test]
    fn split_keeps_anchor_and_records_every_successor_without_alias() {
        let old = IdentityState {
            registry: vec![identity(10, 1)],
            memberships: vec![membership(1, 10, 1), membership(2, 10, 2)],
            ..IdentityState::default()
        };
        let mut allocator = Allocator {
            ids: vec![id(20)],
            next: 0,
        };
        let result = reconcile(&old, &resolution(&[(2, 8), (1, 7)]), 3, &mut allocator).unwrap();
        assert_eq!(result.memberships[0].mdm_id, id(10));
        assert_eq!(result.memberships[1].mdm_id, id(20));
        assert_eq!(result.registry[0].status, IdentityStatus::Active);
        assert!(result.aliases.is_empty());
        assert_eq!(result.splits.len(), 2);
        assert_eq!(
            current_successors(&result, id(10), 10).unwrap(),
            vec![id(10), id(20)]
        );
    }

    #[test]
    fn retirement_keeps_tombstone_and_reactivation_keeps_first_revision() {
        let old = IdentityState {
            registry: vec![identity(10, 1)],
            memberships: vec![membership(1, 10, 1)],
            ..IdentityState::default()
        };
        let mut no_allocations = Allocator {
            ids: vec![],
            next: 0,
        };
        let retired = reconcile(&old, &resolution(&[]), 2, &mut no_allocations).unwrap();
        assert_eq!(retired.registry[0].status, IdentityStatus::Retired);
        assert!(!retired.memberships[0].active);
        assert_eq!(retired.memberships[0].first_membership_revision, 1);

        let mut allocator = Allocator {
            ids: vec![id(20)],
            next: 0,
        };
        let reactivated = reconcile(&retired, &resolution(&[(1, 7)]), 3, &mut allocator).unwrap();
        assert!(reactivated.memberships[0].active);
        assert_eq!(reactivated.memberships[0].mdm_id, id(20));
        assert_eq!(reactivated.memberships[0].first_membership_revision, 1);
    }

    #[test]
    fn mixed_merge_and_split_uses_anchor_claims() {
        let old = IdentityState {
            registry: vec![identity(10, 1), identity(11, 2)],
            memberships: vec![
                membership(1, 10, 1),
                membership(2, 10, 2),
                membership(3, 11, 1),
            ],
            ..IdentityState::default()
        };
        let mut allocator = Allocator {
            ids: vec![id(20)],
            next: 0,
        };
        let result = reconcile(
            &old,
            &resolution(&[(3, 9), (2, 8), (1, 9)]),
            3,
            &mut allocator,
        )
        .unwrap();
        assert_eq!(
            result
                .memberships
                .iter()
                .map(|row| row.mdm_id)
                .collect::<Vec<_>>(),
            vec![id(10), id(20), id(10)]
        );
        assert_eq!(
            result
                .registry
                .iter()
                .find(|row| row.mdm_id == id(11))
                .unwrap()
                .status,
            IdentityStatus::Merged
        );
        assert_eq!(result.aliases[0].canonical_mdm_id, id(10));
        assert_eq!(result.splits.len(), 2);
    }

    #[test]
    fn alias_cycles_are_rejected() {
        let aliases = vec![
            IdentityAlias {
                alias_mdm_id: id(1),
                canonical_mdm_id: id(2),
                publication_revision: 1,
            },
            IdentityAlias {
                alias_mdm_id: id(2),
                canonical_mdm_id: id(1),
                publication_revision: 2,
            },
        ];
        assert!(resolve_alias(&aliases, id(1)).is_err());
    }
}
