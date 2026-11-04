ALTER TABLE mdm_steward.policy_cases_v1
    ADD COLUMN pending_since timestamptz;

ALTER TABLE mdm_steward.policy_cases_v1
    ADD CONSTRAINT policy_cases_v1_pending_since_check
    CHECK (pending_stewardship OR pending_since IS NULL);
