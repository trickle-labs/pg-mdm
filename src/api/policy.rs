use std::collections::BTreeMap;

use pgrx::datum::TimestampWithTimeZone;
use pgrx::prelude::*;
use pgrx::spi::SpiClient;
use pgrx::{Internal, JsonB, Uuid};
use serde_json::{Value, json};

use crate::catalog;
use crate::error::MdmError;
use crate::policy::{
    PolicyActionTuple, PolicyCaseBasis, PolicySubject, basis_digest, next_action_revision,
};
use crate::review::{Review, ReviewStatus};

struct ExistingCase {
    status: String,
    reason_code: String,
    approved_metadata: Value,
    permitted_actions: Vec<String>,
    assigned_queue: Option<String>,
    due_at: Option<String>,
    escalation_level: i32,
    manual_assignment_protected: bool,
    opened_at: Option<String>,
    opened_at_source: String,
    resolved_at: Option<String>,
    review_version: i64,
    definition_version: i64,
    publication_revision: i64,
    stewardship_epoch: i64,
    evidence_basis_digest: [u8; 32],
    action_revision: i64,
    pending_stewardship: bool,
}

fn publication_time(
    client: &mut SpiClient<'_>,
    entity_id: &str,
    revision: i64,
) -> Result<Option<String>, MdmError> {
    client
        .select(
            "SELECT published_at::text FROM mdm_internal.publications WHERE entity_id = $1::pg_catalog.uuid AND publication_revision = $2",
            Some(1),
            &[entity_id.into(), revision.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .first()
        .get::<String>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))
}

fn policy_subjects(value: &Value) -> Vec<PolicySubject> {
    if let Some(subjects) = value.as_array() {
        let mut result = subjects
            .iter()
            .filter_map(|subject| {
                Some(PolicySubject {
                    id: subject.get("id")?.as_str()?.to_owned(),
                    kind: subject.get("kind")?.as_str()?.to_owned(),
                })
            })
            .collect::<Vec<_>>();
        result.sort_by(|left, right| left.kind.cmp(&right.kind).then(left.id.cmp(&right.id)));
        if !result.is_empty() {
            return result;
        }
    }
    if let Some(subjects) = value.as_object() {
        if let (Some(left), Some(right)) = (
            subjects
                .get("left_source_record_id")
                .and_then(Value::as_str),
            subjects
                .get("right_source_record_id")
                .and_then(Value::as_str),
        ) {
            let mut ids = [left, right];
            ids.sort_unstable();
            return ids
                .into_iter()
                .map(|id| PolicySubject {
                    id: id.to_owned(),
                    kind: "source_record".into(),
                })
                .collect();
        }
        if let Some(id) = subjects.get("mdm_id").and_then(Value::as_str) {
            return vec![PolicySubject {
                id: format!(
                    "{}:{}",
                    id,
                    subjects
                        .get("field")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                ),
                kind: "golden".into(),
            }];
        }
    }
    vec![PolicySubject {
        id: serde_json::to_string(value).unwrap_or_default(),
        kind: "review".into(),
    }]
}

fn load_existing_case(
    client: &mut SpiClient<'_>,
    review_id: Uuid,
) -> Result<Option<ExistingCase>, MdmError> {
    let rows = client
        .select(
            "SELECT status, reason_code, approved_metadata, pg_catalog.array_to_string(permitted_actions, ','), assigned_queue::text, due_at::text, escalation_level, manual_assignment_protected, opened_at::text, opened_at_source, resolved_at::text, review_version, definition_version, publication_revision, stewardship_epoch, evidence_basis_digest, action_revision, pending_stewardship FROM mdm_steward.policy_cases_v1 WHERE review_id = $1::pg_catalog.uuid FOR UPDATE",
            Some(1),
            &[review_id.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if rows.is_empty() {
        return Ok(None);
    }
    let row = rows.first();
    let digest = row
        .get::<Vec<u8>>(16)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::PolicyProjection("basis digest is NULL".into()))?;
    Ok(Some(ExistingCase {
        status: row
            .get::<String>(1)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("policy status is NULL".into()))?,
        reason_code: row
            .get::<String>(2)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("policy reason is NULL".into()))?,
        approved_metadata: row
            .get::<JsonB>(3)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("approved metadata is NULL".into()))?
            .0,
        permitted_actions: row
            .get::<String>(4)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .unwrap_or_default()
            .split(',')
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect(),
        assigned_queue: row
            .get::<String>(5)
            .map_err(|error| MdmError::Spi(error.to_string()))?,
        due_at: row
            .get::<String>(6)
            .map_err(|error| MdmError::Spi(error.to_string()))?,
        escalation_level: row
            .get::<i32>(7)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("escalation level is NULL".into()))?,
        manual_assignment_protected: row
            .get::<bool>(8)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("manual protection is NULL".into()))?,
        opened_at: row
            .get::<String>(9)
            .map_err(|error| MdmError::Spi(error.to_string()))?,
        opened_at_source: row
            .get::<String>(10)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("opening source is NULL".into()))?,
        resolved_at: row
            .get::<String>(11)
            .map_err(|error| MdmError::Spi(error.to_string()))?,
        review_version: row
            .get::<i64>(12)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("review version is NULL".into()))?,
        definition_version: row
            .get::<i64>(13)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("definition version is NULL".into()))?,
        publication_revision: row
            .get::<i64>(14)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("publication revision is NULL".into()))?,
        stewardship_epoch: row
            .get::<i64>(15)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::Spi("stewardship epoch is NULL".into()))?,
        evidence_basis_digest: digest
            .try_into()
            .map_err(|_| MdmError::PolicyProjection("basis digest is not 32 bytes".into()))?,
        action_revision: row
            .get::<i64>(17)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("action revision is NULL".into()))?,
        pending_stewardship: row
            .get::<bool>(18)
            .map_err(|error| MdmError::Spi(error.to_string()))?
            .ok_or_else(|| MdmError::PolicyProjection("pending state is NULL".into()))?,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn project_policy_cases(
    client: &mut SpiClient<'_>,
    entity_id: &str,
    entity_name: &str,
    artifact_digest: &[u8],
    definition_version: i64,
    publication_revision: i64,
    decision_epoch: i64,
    source_boundary_digest: &[u8],
    reviews: &[Review],
) -> Result<(), MdmError> {
    let semantic_versions = BTreeMap::from([
        ("candidate".into(), 1_u64),
        ("clustering".into(), 1_u64),
        ("normalization".into(), 1_u64),
    ]);
    for review in reviews {
        let existing = load_existing_case(client, review.review_id)?;
        let basis_changed = existing.as_ref().is_none_or(|case| {
            review.last_change_revision == publication_revision
                && case.publication_revision != publication_revision
        });
        let (opened_at, opened_at_source, resolved_at) = if let Some(case) = &existing {
            (
                case.opened_at.clone(),
                case.opened_at_source.clone(),
                if basis_changed {
                    if review.status == ReviewStatus::Resolved {
                        review
                            .resolved_revision
                            .map(|revision| publication_time(client, entity_id, revision))
                            .transpose()?
                            .flatten()
                    } else {
                        None
                    }
                } else {
                    case.resolved_at.clone()
                },
            )
        } else {
            (
                publication_time(client, entity_id, review.opened_revision)?,
                publication_time(client, entity_id, review.opened_revision)?
                    .map(|_| "publication".into())
                    .unwrap_or_else(|| "unknown".into()),
                review
                    .resolved_revision
                    .map(|revision| publication_time(client, entity_id, revision))
                    .transpose()?
                    .flatten(),
            )
        };
        let permitted_actions = if review.status == ReviewStatus::Resolved {
            Vec::new()
        } else if opened_at.is_some() {
            vec![
                "ASSIGN_QUEUE".into(),
                "ESCALATE".into(),
                "SET_DUE_AT".into(),
            ]
        } else {
            vec!["ASSIGN_QUEUE".into(), "ESCALATE".into()]
        };
        let approved_metadata = json!({});
        let evidence_basis_digest = if basis_changed {
            basis_digest(&PolicyCaseBasis {
                approved_metadata: approved_metadata.clone(),
                artifact_digest: artifact_digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                canonical_encoding_version: 1,
                definition_version: u64::try_from(definition_version).map_err(|_| {
                    MdmError::PolicyProjection("definition version is invalid".into())
                })?,
                entity_name: entity_name.into(),
                issue_key: review
                    .issue_key
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                observed_decision_epoch: u64::try_from(decision_epoch)
                    .map_err(|_| MdmError::PolicyProjection("decision epoch is invalid".into()))?,
                occurrence: u32::try_from(review.occurrence).map_err(|_| {
                    MdmError::PolicyProjection("review occurrence is invalid".into())
                })?,
                reason_code: review.reason_code.clone(),
                semantic_versions: semantic_versions.clone(),
                source_boundary_digest: source_boundary_digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                status: match review.status {
                    ReviewStatus::Open => "open".into(),
                    ReviewStatus::Resolved => "resolved".into(),
                },
                subjects: policy_subjects(&review.subjects),
            })
        } else {
            existing
                .as_ref()
                .map(|case| case.evidence_basis_digest)
                .ok_or_else(|| MdmError::PolicyProjection("missing policy basis".into()))?
        };
        let basis_review_version = if basis_changed {
            review.concurrency_version
        } else {
            existing
                .as_ref()
                .map(|case| case.review_version)
                .ok_or_else(|| MdmError::PolicyProjection("missing review version".into()))?
        };
        let basis_definition_version = if basis_changed {
            definition_version
        } else {
            existing
                .as_ref()
                .map(|case| case.definition_version)
                .ok_or_else(|| MdmError::PolicyProjection("missing definition version".into()))?
        };
        let basis_publication_revision = if basis_changed {
            publication_revision
        } else {
            existing
                .as_ref()
                .map(|case| case.publication_revision)
                .ok_or_else(|| MdmError::PolicyProjection("missing publication revision".into()))?
        };
        let assigned_queue = existing
            .as_ref()
            .and_then(|case| case.assigned_queue.clone());
        let due_at = existing.as_ref().and_then(|case| case.due_at.clone());
        let escalation_level = existing.as_ref().map_or(0, |case| case.escalation_level);
        let manual_assignment_protected = existing
            .as_ref()
            .is_some_and(|case| case.manual_assignment_protected);
        let action_after = PolicyActionTuple {
            status: match review.status {
                ReviewStatus::Open => "open".into(),
                ReviewStatus::Resolved => "resolved".into(),
            },
            reason_code: review.reason_code.clone(),
            approved_metadata: approved_metadata.clone(),
            permitted_actions: permitted_actions.clone(),
            assigned_queue: assigned_queue.clone(),
            due_at: due_at.clone(),
            escalation_level: u32::try_from(escalation_level)
                .map_err(|_| MdmError::PolicyProjection("escalation level is invalid".into()))?,
            manual_assignment_protected,
            opening_time_available: opened_at.is_some(),
            pending_stewardship: false,
            review_version: u64::try_from(basis_review_version)
                .map_err(|_| MdmError::PolicyProjection("review version is invalid".into()))?,
            definition_version: u64::try_from(basis_definition_version)
                .map_err(|_| MdmError::PolicyProjection("definition version is invalid".into()))?,
            stewardship_epoch: u64::try_from(decision_epoch)
                .map_err(|_| MdmError::PolicyProjection("decision epoch is invalid".into()))?,
            evidence_basis_digest,
        };
        let action_revision = if let Some(case) = &existing {
            let before = PolicyActionTuple {
                status: case.status.clone(),
                reason_code: case.reason_code.clone(),
                approved_metadata: case.approved_metadata.clone(),
                permitted_actions: case.permitted_actions.clone(),
                assigned_queue: case.assigned_queue.clone(),
                due_at: case.due_at.clone(),
                escalation_level: u32::try_from(case.escalation_level).map_err(|_| {
                    MdmError::PolicyProjection("escalation level is invalid".into())
                })?,
                manual_assignment_protected: case.manual_assignment_protected,
                opening_time_available: case.opened_at.is_some(),
                pending_stewardship: case.pending_stewardship,
                review_version: u64::try_from(case.review_version)
                    .map_err(|_| MdmError::PolicyProjection("review version is invalid".into()))?,
                definition_version: u64::try_from(case.definition_version).map_err(|_| {
                    MdmError::PolicyProjection("definition version is invalid".into())
                })?,
                stewardship_epoch: u64::try_from(case.stewardship_epoch).map_err(|_| {
                    MdmError::PolicyProjection("stewardship epoch is invalid".into())
                })?,
                evidence_basis_digest: case.evidence_basis_digest,
            };
            next_action_revision(
                u64::try_from(case.action_revision)
                    .map_err(|_| MdmError::PolicyProjection("action revision is invalid".into()))?,
                &before,
                &action_after,
            )
            .ok_or_else(|| MdmError::PolicyProjection("action revision exhausted".into()))?
        } else {
            1
        };
        let status = match review.status {
            ReviewStatus::Open => "open",
            ReviewStatus::Resolved => "resolved",
        };
        let args = &[
            review.issue_key.to_vec().into(),
            review.review_id.into(),
            entity_name.into(),
            review.occurrence.into(),
            status.into(),
            review.severity.clone().into(),
            review.reason_code.clone().into(),
            JsonB(approved_metadata).into(),
            JsonB(json!(permitted_actions)).into(),
            assigned_queue.into(),
            due_at.into(),
            escalation_level.into(),
            manual_assignment_protected.into(),
            opened_at.into(),
            opened_at_source.into(),
            resolved_at.into(),
            basis_review_version.into(),
            basis_definition_version.into(),
            basis_publication_revision.into(),
            decision_epoch.into(),
            evidence_basis_digest.to_vec().into(),
            i64::try_from(action_revision)
                .map_err(|_| MdmError::PolicyProjection("action revision is invalid".into()))?
                .into(),
            false.into(),
        ];
        if existing.is_some() {
            client
                .update(
                    "UPDATE mdm_steward.policy_cases_v1 SET entity_name = $3::pg_catalog.name, issue_key = $1, occurrence = $4, status = $5, severity = $6, reason_code = $7, approved_metadata = $8, permitted_actions = ARRAY(SELECT pg_catalog.jsonb_array_elements_text($9::pg_catalog.jsonb)), assigned_queue = $10::pg_catalog.name, due_at = $11::timestamptz, escalation_level = $12, manual_assignment_protected = $13, opened_at = $14::timestamptz, opened_at_source = $15, resolved_at = $16::timestamptz, review_version = $17, definition_version = $18, publication_revision = $19, stewardship_epoch = $20, evidence_basis_digest = $21, action_revision = $22, pending_stewardship = $23, last_observed_at = pg_catalog.statement_timestamp() WHERE review_id = $2::pg_catalog.uuid",
                    None,
                    args,
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
        } else {
            client
                .update(
                    "INSERT INTO mdm_steward.policy_cases_v1 (case_key, review_id, entity_name, issue_key, occurrence, status, severity, reason_code, approved_metadata, permitted_actions, assigned_queue, due_at, escalation_level, manual_assignment_protected, opened_at, opened_at_source, resolved_at, review_version, definition_version, publication_revision, stewardship_epoch, evidence_basis_digest, action_revision, pending_stewardship) VALUES (pg_catalog.nextval('mdm_internal.policy_case_key_seq'), $2::pg_catalog.uuid, $3::pg_catalog.name, $1, $4, $5, $6, $7, $8, ARRAY(SELECT pg_catalog.jsonb_array_elements_text($9::pg_catalog.jsonb)), $10::pg_catalog.name, $11::timestamptz, $12, $13, $14::timestamptz, $15, $16::timestamptz, $17, $18, $19, $20, $21, $22, $23)",
                    None,
                    args,
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
        }
    }
    Ok(())
}

struct BackfillRequest {
    case_key: i64,
    opened_at: TimestampWithTimeZone,
    reason: String,
}

fn parse_uuid(value: &str) -> Result<Uuid, MdmError> {
    let bytes = value
        .split('-')
        .flat_map(|part| {
            (0..part.len()).step_by(2).filter_map(|index| {
                part.get(index..index + 2)
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            })
        })
        .collect::<Vec<_>>();
    Uuid::from_slice(&bytes).map_err(|error| MdmError::OperationState(error.to_string()))
}

pub(crate) fn mark_pending_cases(
    client: &mut SpiClient<'_>,
    entity_name: &str,
) -> Result<(), MdmError> {
    client
        .update(
            "UPDATE mdm_steward.policy_cases_v1 SET pending_stewardship = true, action_revision = action_revision + 1 WHERE entity_name = $1::pg_catalog.name AND status = 'open' AND NOT pending_stewardship",
            None,
            &[entity_name.into()],
        )
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    Ok(())
}

#[pg_extern(
    name = "backfill_policy_case_opened_at",
    requires = [persist_backfill_policy_case_opened_at],
    sql = "CREATE FUNCTION mdm_admin.backfill_policy_case_opened_at(case_key bigint, opened_at timestamptz, reason text) RETURNS TABLE (operation_id uuid, action_revision bigint) LANGUAGE c AS 'MODULE_PATHNAME', 'backfill_policy_case_opened_at_wrapper';"
)]
pub(crate) fn backfill_policy_case_opened_at(
    case_key: i64,
    opened_at: TimestampWithTimeZone,
    reason: String,
) -> TableIterator<'static, (name!(operation_id, Uuid), name!(action_revision, i64))> {
    let result = (|| {
        let value = catalog::call_helper(
            "persist_backfill_policy_case_opened_at",
            BackfillRequest {
                case_key,
                opened_at,
                reason,
            },
        )?;
        let operation_id = parse_uuid(
            value.0["operation_id"]
                .as_str()
                .ok_or_else(|| MdmError::OperationState("operation ID is missing".into()))?,
        )?;
        let action_revision = value.0["action_revision"]
            .as_i64()
            .ok_or_else(|| MdmError::OperationState("action revision is missing".into()))?;
        Ok(vec![(operation_id, action_revision)])
    })();
    match result {
        Ok(rows) => TableIterator::new(rows),
        Err(error) => crate::raise(error),
    }
}

#[pg_extern(
    name = "persist_backfill_policy_case_opened_at",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_backfill_policy_case_opened_at(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_backfill_policy_case_opened_at_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn persist_backfill_policy_case_opened_at(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only the public backfill wrapper constructs this request.
        let request = unsafe { request.get::<BackfillRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("policy backfill request is required".into()))?;
        if request.case_key <= 0 {
            return Err(MdmError::PolicyProjection(
                "case_key must be positive".into(),
            ));
        }
        if request.reason.trim().is_empty() {
            return Err(MdmError::PolicyProjection(
                "reason must not be empty".into(),
            ));
        }
        let helper_owner = catalog::validate_helper_owner()?;
        let (session, selected) = catalog::validate_caller(&helper_owner)?;

        Spi::connect_mut(|client| {
            let row = client
                .select(
                    "SELECT c.entity_name::text, c.opened_at::text, c.opened_at_source, c.status, c.action_revision, e.execution_role_name, b.role_oid, r.opened_revision, (SELECT min(o.observed_at)::text FROM mdm_internal.publication_observations o WHERE o.entity_id = e.entity_id AND o.publication_revision >= r.opened_revision) FROM mdm_steward.policy_cases_v1 c JOIN mdm_internal.reviews r ON r.review_id = c.review_id JOIN mdm_internal.entities e ON e.entity_name = c.entity_name LEFT JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id WHERE c.case_key = $1 FOR UPDATE",
                    Some(1),
                    &[request.case_key.into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            if row.is_empty() {
                return Err(MdmError::PolicyProjection(format!(
                    "policy case {} does not exist",
                    request.case_key
                )));
            }
            let row = row.first();
            let entity_name = row
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::PolicyProjection("entity name is NULL".into()))?;
            let opened_at_current = row
                .get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let opened_at_source = row
                .get::<String>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::PolicyProjection("opening source is NULL".into()))?;
            if opened_at_current.is_some() || opened_at_source != "unknown" {
                return Err(MdmError::PolicyProjection(
                    "policy case opening time is already known".into(),
                ));
            }
            let execution_role = row
                .get::<String>(6)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::PolicyProjection("execution role is NULL".into()))?;
            let bound_oid = row
                .get::<pg_sys::Oid>(7)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            if execution_role != selected.name || bound_oid != Some(catalog::outer_user_id()) {
                return Err(MdmError::Unauthorized(
                    "entity is bound to another execution role".into(),
                ));
            }
            let first_observation = row
                .get::<String>(9)
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let valid_time = client
                .select(
                    "SELECT $1::timestamptz <= COALESCE($2::timestamptz, pg_catalog.statement_timestamp())",
                    Some(1),
                    &[request.opened_at.into(), first_observation.clone().into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .first()
                .get::<bool>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false);
            if !valid_time {
                return Err(MdmError::PolicyProjection(
                    "opening time must not be later than the first retained observation".into(),
                ));
            }
            let status = row
                .get::<String>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::PolicyProjection("policy status is NULL".into()))?;
            let operation = client
                .update(
                    "INSERT INTO mdm_internal.operations (operation_kind, entity_name, status, outcome, actor_name, actor_role_name) VALUES ('policy_case_opened_at_backfill', $1::pg_catalog.name, 'running', $2, $3, $4) RETURNING operation_id::text",
                    Some(1),
                    &[
                        entity_name.clone().into(),
                        JsonB(json!({"case_key": request.case_key, "reason": request.reason})).into(),
                        session.name.clone().into(),
                        selected.name.clone().into(),
                    ],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let operation_id = operation
                .first()
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::OperationState("operation ID is NULL".into()))?;
            let updated = client
                .update(
                    "UPDATE mdm_steward.policy_cases_v1 SET opened_at = $2, opened_at_source = 'administrator', permitted_actions = CASE WHEN status = 'open' THEN ARRAY['ASSIGN_QUEUE', 'ESCALATE', 'SET_DUE_AT']::text[] ELSE '{}'::text[] END, action_revision = action_revision + 1 WHERE case_key = $1 RETURNING action_revision",
                    Some(1),
                    &[request.case_key.into(), request.opened_at.into()],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            let action_revision = updated
                .first()
                .get::<i64>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::OperationState("action revision is NULL".into()))?;
            client
                .update(
                    "UPDATE mdm_internal.operations SET status = 'succeeded', result_code = 'MDM_OK', outcome = outcome || $2, completed_at = pg_catalog.statement_timestamp() WHERE operation_id = $1::pg_catalog.uuid AND status = 'running'",
                    None,
                    &[
                        operation_id.clone().into(),
                        JsonB(json!({"case_key": request.case_key, "action_revision": action_revision, "status": status})).into(),
                    ],
                )
                .map_err(|error| MdmError::Spi(error.to_string()))?;
            Ok(JsonB(json!({
                "operation_id": operation_id,
                "action_revision": action_revision
            })))
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
