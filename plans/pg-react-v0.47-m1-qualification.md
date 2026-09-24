# pg-react v0.47 M1 qualification requirements

## Purpose

pg-react v0.47 reads `mdm_steward.policy_cases_v1` to choose a route and preview a due date. Its T47.11 and T47.13 gates need evidence that the rows keep the right case identity and opening time across publication, resolution, recurrence, backfill, and restore. The initial live publication is already covered by the pg-react joint test. This document specifies the remaining pg-mdm evidence.

The required behavior comes from [the v0.13 policy projection plan](v0.13.md). The public projection and `mdm_admin.backfill_policy_case_opened_at(bigint, timestamptz, text)` already exist. First extend the tests around those APIs. Change implementation only if a test exposes a defect.

## Test boundary

Run the lifecycle checks against an installed pg-mdm package with the pinned PostgreSQL and pg-trickle versions. Create reviews and publications through supported MDM APIs. Read case rows through `mdm_steward.policy_cases_v1` as the granted policy reader. Do not create a recurrence by updating `mdm_internal.reviews` or `mdm_steward.policy_cases_v1` directly.

Use a real pre-v0.13 upgrade fixture to exercise an unknown legacy opening time. Any fixture construction before the upgrade must be identified in the test. Exercise backfill through `mdm_admin.backfill_policy_case_opened_at`, under its required execution role. Exercise restore through the supported logical dump, restore, helper setup, grant, and rebind procedure.

Compare exact, ordered rows and timestamps at each step. A row count alone does not establish identity or deadline behavior. Record the API calls, role, package versions, source commit, and image digest with the test result.

## T47.11: publication and recurrence

Use one issue that passes through this sequence:

1. Publish an open review. Capture its `case_key`, `review_id`, `issue_key`, `occurrence`, `opened_at`, `opened_at_source`, `review_version`, `evidence_basis_digest`, and `action_revision`. `opened_at` must equal the retained opening publication's `published_at`.
2. Publish an update that does not change the review's action meaning. Prove that a new publication was retained. The same case row must retain the captured values. Observation fields may advance. The pg-react route and due-date preview for that case must stay the same.
3. Resolve the issue through the MDM lifecycle and publish. The existing row must remain with the same identity and `opened_at`, have `status = 'resolved'`, have no permitted actions, and have `resolved_at` from the matching publication.
4. Reintroduce the same issue and publish. The resolved row must remain. The new open row must keep the same `issue_key`, have `occurrence = 2`, receive a different `review_id` and `case_key`, set `action_revision = 1`, and take its own `opened_at` from the new opening publication. The pg-react route and due-date preview must use the new row and its opening time.

The existing `resolved_issue_reopens_as_next_occurrence` unit test in `tests/review_tests.rs` proves the review allocator's behavior. It does not prove the installed public projection or the pg-react read path.

## T47.13: unknown opening time and backfill

After upgrading the legacy fixture, retain a case whose opening publication time is unavailable. Its public row must have `opened_at IS NULL` and `opened_at_source = 'unknown'`. It must not permit `SET_DUE_AT`, and pg-react must produce no due-date proposal for it.

Backfill that case with a known timestamp no later than its first retained observation and a nonempty reason. The call must return an operation ID and the new action revision. The same case must then have that exact `opened_at`, `opened_at_source = 'administrator'`, and an action revision one greater than before. An open case must now permit `SET_DUE_AT`; pg-react must calculate the due date from the backfilled time. The matching operation must record the authenticated session, selected role, case key, reason, and success. Publication history must remain unchanged.

Also prove rejection without a row or audit change for an unauthorized caller, an empty reason, a time after the first retained observation, and a second backfill. Roll back one otherwise valid backfill and prove that neither its case change nor its operation survives.

## T47.13: logical restore

Extend `tests/restore.sql` and its setup to include open, resolved, recurrent, and backfilled policy cases. Before dump and after restore into a database with different relation and role OIDs, compare the exact ordered public rows, including case keys, review IDs, issue keys, occurrences, status, opening and resolution times, opening sources, publication revisions, action revisions, and evidence digests. Verify that each case still refers to its retained review and, where one exists, its opening publication. Reapply the documented helper setup and reader grants, then rebind the entity.

After restore, publish a new recurrence through the MDM API. Its new `case_key` must exceed every restored key, and its `opened_at` must come from its own opening publication. The restored cases must remain unchanged.

## Handoff and acceptance

Place reproducible SQL assertions in the pg-mdm E2E and restore tests. Report the exact commands, passing result, tested commit, package and image digests, and transcript locations to the pg-react v0.47 qualification record. The pg-react owner then runs the route and due-date assertions against those real rows in the final joint image.

T47.11 passes only when the publication-only update, resolution, recurrence, and pg-react assertions pass. T47.13 passes only when the unknown-time, backfill, restore, new-key, and pg-react assertions pass. A fixture-only result or a test that changes projection rows directly does not close either gate. Signed workload limits remain a separate release-owner gate. No pg-trickle change is requested by this qualification work.
