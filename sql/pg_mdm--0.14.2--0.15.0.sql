ALTER TABLE mdm_steward.policy_cases_v1
    ADD COLUMN pending_since timestamptz;

ALTER TABLE mdm_steward.policy_cases_v1
    ADD CONSTRAINT policy_cases_v1_pending_since_check
    CHECK (pending_stewardship OR pending_since IS NULL);

ALTER TABLE mdm_steward.policy_receipts_v1
    ADD COLUMN publication_tracking_started_at timestamptz NOT NULL
    DEFAULT pg_catalog.statement_timestamp();

CREATE INDEX policy_receipts_publication_pending_idx
    ON mdm_steward.policy_receipts_v1 (case_key, action_revision)
    WHERE outcome = 'APPLIED_CONTROL';

CREATE TABLE mdm_steward.policy_receipt_publications_v1 (
    receipt_id uuid PRIMARY KEY REFERENCES mdm_steward.policy_receipts_v1(receipt_id),
    case_key bigint NOT NULL CHECK (case_key > 0),
    action_revision bigint NOT NULL CHECK (action_revision > 0),
    publication_revision bigint NOT NULL CHECK (publication_revision >= 0),
    publication_state text NOT NULL CHECK (publication_state IN ('PUBLISHED', 'SUPERSEDED_BEFORE_PUBLICATION')),
    observed_at timestamptz NOT NULL DEFAULT pg_catalog.statement_timestamp()
);

SELECT pg_catalog.pg_extension_config_dump('mdm_steward.policy_receipt_publications_v1'::pg_catalog.regclass, '');
REVOKE ALL ON TABLE mdm_steward.policy_receipt_publications_v1 FROM PUBLIC;

DO $$
DECLARE helper_owner name;
BEGIN
    SELECT pg_catalog.pg_get_userbyid(relowner) INTO STRICT helper_owner
    FROM pg_catalog.pg_class
    WHERE oid = 'mdm_steward.policy_receipts_v1'::pg_catalog.regclass;
    EXECUTE pg_catalog.format(
        'GRANT INSERT ON TABLE mdm_steward.policy_receipt_publications_v1 TO %I',
        helper_owner
    );
END
$$;

DO $$
DECLARE principal_role name;
BEGIN
    FOR principal_role IN
        SELECT DISTINCT automation_role_name
        FROM mdm_steward.policy_bindings_v1
    LOOP
        EXECUTE pg_catalog.format(
            'GRANT SELECT ON TABLE mdm_steward.policy_receipt_publications_v1 TO %I',
            principal_role
        );
    END LOOP;
END
$$;
