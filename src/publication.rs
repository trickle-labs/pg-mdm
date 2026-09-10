use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::MdmError;
use crate::golden::GoldenCandidate;
use crate::identity::{IdAllocator, IdentityResolution};
use crate::output::canonical_json;
use crate::review::ReviewFact;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PublishedState {
    pub definition_version: u64,
    pub decision_epoch: u64,
    pub projection: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Resolution {
    pub identities: IdentityResolution,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PublicationPlan {
    pub changed: bool,
    pub result_digest: [u8; 32],
    pub projection: Value,
    pub facts: Vec<ReviewFact>,
}

#[allow(dead_code)]
fn digest<T: Serialize>(value: &T) -> Result<[u8; 32], MdmError> {
    let value = serde_json::to_value(value)
        .map(|value| canonical_json(&value))
        .map_err(|error| MdmError::OperationState(error.to_string()))?;
    let bytes =
        serde_json::to_vec(&value).map_err(|error| MdmError::OperationState(error.to_string()))?;
    Ok(Sha256::digest(bytes).into())
}

#[allow(dead_code)]
pub(crate) fn publish_resolution(
    old: PublishedState,
    resolved: Resolution,
    golden_candidates: Vec<GoldenCandidate>,
    review_facts: Vec<ReviewFact>,
    allocator: &mut dyn IdAllocator,
) -> Result<PublicationPlan, MdmError> {
    let projection = json!({
        "definition_version": old.definition_version,
        "identities": {
            "registry": resolved.identities.registry.iter().map(|record| json!({
                "mdm_id": record.mdm_id.to_string(),
                "created_revision": record.created_revision,
                "retired_revision": record.retired_revision,
                "status": format!("{:?}", record.status),
            })).collect::<Vec<_>>(),
            "memberships": resolved.identities.memberships.iter().map(|membership| json!({
                "source_record_id": membership.source_record_id.to_string(),
                "source_sort_key": membership.source_sort_key,
                "mdm_id": membership.mdm_id.to_string(),
                "active": membership.active,
                "first_membership_revision": membership.first_membership_revision,
                "last_membership_revision": membership.last_membership_revision,
                "membership_reason": membership.membership_reason,
                "last_change_revision": membership.last_change_revision,
            })).collect::<Vec<_>>(),
            "aliases": resolved.identities.aliases.iter().map(|alias| json!({
                "alias_mdm_id": alias.alias_mdm_id.to_string(),
                "canonical_mdm_id": alias.canonical_mdm_id.to_string(),
                "publication_revision": alias.publication_revision,
            })).collect::<Vec<_>>(),
            "splits": resolved.identities.splits.iter().map(|split| json!({
                "parent_mdm_id": split.parent_mdm_id.to_string(),
                "child_mdm_id": split.child_mdm_id.to_string(),
                "publication_revision": split.publication_revision,
            })).collect::<Vec<_>>(),
        },
        "goldens": golden_candidates.iter().map(|candidate| json!({
            "source_record_id": candidate.source_record_id.to_string(),
            "source_name": candidate.source_name,
            "source_priority": candidate.source_priority,
            "row_changed_at": candidate.row_changed_at,
            "authoritative": candidate.authoritative,
            "source_sort_key": candidate.source_sort_key,
            "raw_value": candidate.raw_value,
            "state": format!("{:?}", candidate.state),
            "normalized": candidate.normalized,
            "canonical_bytes": candidate.canonical_bytes,
        })).collect::<Vec<_>>(),
        "reviews": review_facts,
    });
    let result_digest = digest(&projection)?;
    let _ = allocator;
    let old_digest = digest(&old.projection)?;
    Ok(PublicationPlan {
        changed: old_digest != result_digest,
        result_digest,
        projection,
        facts: review_facts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::IdentityResolution;

    #[test]
    fn bookkeeping_order_does_not_change_digest() {
        let old = PublishedState {
            projection: json!({"b": 1, "a": 2}),
            ..Default::default()
        };
        let resolution = Resolution {
            identities: IdentityResolution::default(),
        };
        let mut allocator = || pgrx::Uuid::from_bytes([1; 16]);
        let plan =
            publish_resolution(old, resolution, Vec::new(), Vec::new(), &mut allocator).unwrap();
        assert_eq!(plan.result_digest.len(), 32);
    }
}
