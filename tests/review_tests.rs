#[path = "../src/review.rs"]
mod review;

use pg_mdm::identity::{self, IdAllocator};
use pgrx::Uuid;
use review::{
    ReviewCandidate, ReviewStatus, SplitNotice, Subject, issue_key, reconcile,
    reconcile_with_splits,
};
use serde_json::json;

fn id(value: u8) -> Uuid {
    Uuid::from_bytes([value; 16])
}

#[derive(Default)]
struct Allocator(u8);

impl IdAllocator for Allocator {
    fn next_id(&mut self) -> Uuid {
        self.0 += 1;
        id(self.0)
    }
}

fn candidate(summary: &str) -> ReviewCandidate {
    ReviewCandidate::new(
        7,
        "warning",
        "SUPPORTING_ONLY",
        &[Subject::uuid("pair", id(2)), Subject::uuid("pair", id(1))],
        json!({"kind":"pair","ids":[1,2]}),
        json!({"summary":summary}),
    )
}

#[test]
fn issue_key_is_canonical_and_excludes_display_payload() {
    let subjects = [
        Subject::uuid("source_record", id(1)),
        Subject::uuid("mdm_id", id(2)),
    ];
    let reversed = [subjects[1].clone(), subjects[0].clone()];
    assert_eq!(
        issue_key(7, "GOLDEN_TIE", &subjects),
        issue_key(7, "GOLDEN_TIE", &reversed)
    );
    assert_ne!(
        issue_key(7, "GOLDEN_TIE", &subjects),
        issue_key(8, "GOLDEN_TIE", &subjects)
    );
    assert_ne!(
        issue_key(7, "GOLDEN_TIE", &subjects),
        issue_key(7, "INVALID_AUTHORITATIVE_VALUE", &subjects)
    );
    assert_eq!(candidate("one").issue_key, candidate("two").issue_key);
}

#[test]
fn lifecycle_opens_keeps_unchanged_updates_and_resolves() {
    let mut allocator = Allocator::default();
    let opened = reconcile(&[], &[candidate("one")], 1, &mut allocator);
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].occurrence, 1);
    assert_eq!(opened[0].status, ReviewStatus::Open);
    assert_eq!(opened[0].concurrency_version, 1);

    let unchanged = reconcile(&opened, &[candidate("one")], 2, &mut allocator);
    assert_eq!(unchanged[0], opened[0]);

    let updated = reconcile(&unchanged, &[candidate("two")], 3, &mut allocator);
    assert_eq!(updated[0].review_id, opened[0].review_id);
    assert_eq!(updated[0].last_change_revision, 3);
    assert_eq!(updated[0].concurrency_version, 2);

    let resolved = reconcile(&updated, &[], 4, &mut allocator);
    assert_eq!(resolved[0].status, ReviewStatus::Resolved);
    assert_eq!(resolved[0].resolved_revision, Some(4));
    assert_eq!(resolved[0].concurrency_version, 3);
}

#[test]
fn resolved_issue_reopens_as_next_occurrence() {
    let mut allocator = Allocator::default();
    let opened = reconcile(&[], &[candidate("one")], 1, &mut allocator);
    let resolved = reconcile(&opened, &[], 2, &mut allocator);
    let recurred = reconcile(&resolved, &[candidate("one")], 3, &mut allocator);

    assert_eq!(recurred.len(), 2);
    assert_eq!(recurred[0].occurrence, 1);
    assert_eq!(recurred[0].status, ReviewStatus::Resolved);
    assert_eq!(recurred[1].occurrence, 2);
    assert_eq!(recurred[1].status, ReviewStatus::Open);
    assert_ne!(recurred[0].review_id, recurred[1].review_id);
}

#[test]
fn split_notice_opens_once_then_resolves_without_history_churn() {
    let split = SplitNotice {
        parent_id: id(9),
        successor_ids: vec![id(3), id(2)],
        publication_revision: 10,
        severity: "warning".into(),
        subjects: json!({"parent":"masked","successors":2}),
        masked_summary: json!({"kind":"split"}),
    };
    let mut allocator = Allocator::default();
    let opened = reconcile_with_splits(
        &[],
        &[],
        std::slice::from_ref(&split),
        7,
        10,
        &mut allocator,
    );
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].status, ReviewStatus::Open);

    let unchanged = reconcile_with_splits(
        &opened,
        &[],
        std::slice::from_ref(&split),
        7,
        10,
        &mut allocator,
    );
    assert_eq!(unchanged, opened);

    let resolved = reconcile_with_splits(&opened, &[], &[split], 7, 11, &mut allocator);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].status, ReviewStatus::Resolved);
    assert_eq!(resolved[0].resolved_revision, Some(11));

    let retained_history = reconcile_with_splits(&resolved, &[], &[], 7, 12, &mut allocator);
    assert_eq!(retained_history, resolved);
}
