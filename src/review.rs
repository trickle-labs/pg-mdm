use std::collections::BTreeSet;

use pgrx::Uuid;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::identity::IdAllocator;

pub const ISSUE_KEY_FORMAT_VERSION: u8 = 1;
pub const SPLIT_NOTICE_REASON: &str = "SPLIT_NOTICE";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Subject {
    pub kind: String,
    pub identity: Vec<u8>,
}

impl Subject {
    pub fn new(kind: impl Into<String>, identity: impl Into<Vec<u8>>) -> Self {
        Self {
            kind: kind.into(),
            identity: identity.into(),
        }
    }

    pub fn uuid(kind: impl Into<String>, identity: Uuid) -> Self {
        Self::new(kind, identity.as_bytes().to_vec())
    }

    pub fn split(
        parent: Uuid,
        successors: impl IntoIterator<Item = Uuid>,
        publication: i64,
    ) -> Self {
        let successors = successors
            .into_iter()
            .map(|successor| *successor.as_bytes())
            .collect::<BTreeSet<_>>();
        let mut identity = Vec::with_capacity(16 + 8 + successors.len() * 16);
        write_part(&mut identity, parent.as_bytes());
        identity.extend_from_slice(&publication.to_be_bytes());
        identity.extend_from_slice(&(successors.len() as u32).to_be_bytes());
        for successor in successors {
            write_part(&mut identity, &successor);
        }
        Self::new("split", identity)
    }
}

pub fn issue_key(definition_version: i64, reason_code: &str, subjects: &[Subject]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"pg_mdm.review.issue_key");
    hasher.update([ISSUE_KEY_FORMAT_VERSION]);
    hasher.update(definition_version.to_be_bytes());
    write_part_hash(&mut hasher, reason_code.as_bytes());

    let subjects = subjects
        .iter()
        .map(|subject| (subject.kind.as_bytes(), subject.identity.as_slice()))
        .collect::<BTreeSet<_>>();
    hasher.update((subjects.len() as u32).to_be_bytes());
    for (kind, identity) in subjects {
        write_part_hash(&mut hasher, kind);
        write_part_hash(&mut hasher, identity);
    }
    hasher.finalize().into()
}

fn write_part(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    output.extend_from_slice(bytes);
}

fn write_part_hash(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u32).to_be_bytes());
    hasher.update(bytes);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewStatus {
    Open,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReviewCandidate {
    pub issue_key: [u8; 32],
    pub severity: String,
    pub reason_code: String,
    pub subjects: Value,
    pub masked_summary: Value,
}

impl ReviewCandidate {
    pub fn new(
        definition_version: i64,
        severity: impl Into<String>,
        reason_code: impl Into<String>,
        subject_identities: &[Subject],
        subjects: Value,
        masked_summary: Value,
    ) -> Self {
        let reason_code = reason_code.into();
        Self {
            issue_key: issue_key(definition_version, &reason_code, subject_identities),
            severity: severity.into(),
            reason_code,
            subjects,
            masked_summary,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitNotice {
    pub parent_id: Uuid,
    pub successor_ids: Vec<Uuid>,
    pub publication_revision: i64,
    pub severity: String,
    pub subjects: Value,
    pub masked_summary: Value,
}

impl SplitNotice {
    pub fn candidate(&self, definition_version: i64) -> ReviewCandidate {
        ReviewCandidate::new(
            definition_version,
            self.severity.clone(),
            SPLIT_NOTICE_REASON,
            &[Subject::split(
                self.parent_id,
                self.successor_ids.iter().copied(),
                self.publication_revision,
            )],
            self.subjects.clone(),
            self.masked_summary.clone(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub review_id: Uuid,
    pub issue_key: [u8; 32],
    pub occurrence: i32,
    pub status: ReviewStatus,
    pub severity: String,
    pub reason_code: String,
    pub subjects: Value,
    pub masked_summary: Value,
    pub opened_revision: i64,
    pub resolved_revision: Option<i64>,
    pub last_change_revision: i64,
    pub concurrency_version: i64,
}

impl Review {
    fn open(candidate: &ReviewCandidate, occurrence: i32, revision: i64, review_id: Uuid) -> Self {
        Self {
            review_id,
            issue_key: candidate.issue_key,
            occurrence,
            status: ReviewStatus::Open,
            severity: candidate.severity.clone(),
            reason_code: candidate.reason_code.clone(),
            subjects: candidate.subjects.clone(),
            masked_summary: candidate.masked_summary.clone(),
            opened_revision: revision,
            resolved_revision: None,
            last_change_revision: revision,
            concurrency_version: 1,
        }
    }

    fn matches(&self, candidate: &ReviewCandidate) -> bool {
        self.severity == candidate.severity
            && self.reason_code == candidate.reason_code
            && self.subjects == candidate.subjects
            && self.masked_summary == candidate.masked_summary
    }

    fn update(&mut self, candidate: &ReviewCandidate, revision: i64) {
        self.severity = candidate.severity.clone();
        self.reason_code = candidate.reason_code.clone();
        self.subjects = candidate.subjects.clone();
        self.masked_summary = candidate.masked_summary.clone();
        self.last_change_revision = revision;
        self.concurrency_version += 1;
    }

    fn resolve(&mut self, revision: i64) {
        self.status = ReviewStatus::Resolved;
        self.resolved_revision = Some(revision);
        self.last_change_revision = revision;
        self.concurrency_version += 1;
    }
}

pub fn reconcile(
    previous: &[Review],
    current: &[ReviewCandidate],
    revision: i64,
    allocator: &mut dyn IdAllocator,
) -> Vec<Review> {
    let mut reviews = previous.to_vec();
    let mut current = current.to_vec();
    current.sort_by_key(|candidate| candidate.issue_key);

    let mut seen = BTreeSet::new();
    for candidate in &current {
        if !seen.insert(candidate.issue_key) {
            continue;
        }
        match latest(&reviews, candidate.issue_key) {
            Some(index) if reviews[index].status == ReviewStatus::Open => {
                if !reviews[index].matches(candidate) {
                    reviews[index].update(candidate, revision);
                }
            }
            Some(index) => {
                let occurrence = reviews[index].occurrence + 1;
                reviews.push(Review::open(
                    candidate,
                    occurrence,
                    revision,
                    allocator.next_id(),
                ));
            }
            None => reviews.push(Review::open(candidate, 1, revision, allocator.next_id())),
        }
    }

    let active = current
        .iter()
        .map(|candidate| candidate.issue_key)
        .collect::<BTreeSet<_>>();
    for review in &mut reviews {
        if review.status == ReviewStatus::Open && !active.contains(&review.issue_key) {
            review.resolve(revision);
        }
    }

    reviews.sort_by_key(|review| (review.issue_key, review.occurrence));
    reviews
}

pub fn reconcile_with_splits(
    previous: &[Review],
    current: &[ReviewCandidate],
    new_splits: &[SplitNotice],
    definition_version: i64,
    revision: i64,
    allocator: &mut dyn IdAllocator,
) -> Vec<Review> {
    let mut candidates = current.to_vec();
    candidates.extend(
        new_splits
            .iter()
            .filter(|split| split.publication_revision == revision)
            .map(|split| split.candidate(definition_version)),
    );
    reconcile(previous, &candidates, revision, allocator)
}

fn latest(reviews: &[Review], issue_key: [u8; 32]) -> Option<usize> {
    reviews
        .iter()
        .enumerate()
        .filter(|(_, review)| review.issue_key == issue_key)
        .max_by_key(|(_, review)| review.occurrence)
        .map(|(index, _)| index)
}
