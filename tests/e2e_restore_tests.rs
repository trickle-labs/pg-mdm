use std::fs;
use std::path::Path;

#[test]
fn test_restore_script_integrity() {
    let restore_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("restore.sql");
    let content = fs::read_to_string(&restore_path).expect("restore.sql exists");

    // The restore test must assert durable tables are restored
    assert!(content.contains("mdm_internal.entities"));
    assert!(content.contains("mdm_internal.definitions"));
    assert!(content.contains("mdm_internal.source_identities"));
    assert!(content.contains("mdm_internal.steward_decisions"));
    assert!(content.contains("mdm_admin.rebind"));
    assert!(content.contains("mdm_steward.policy_bindings_v1"));
    assert!(content.contains("mdm_steward.policy_receipts_v1"));
    assert!(content.contains("request_body->>'binding_id'"));
    assert!(content.contains("mdm_internal.policy_binding_runtime"));
    assert!(content.contains("policy binding runtime was restored before reconciliation"));
    assert!(content.contains("restored binding was not rejected before reconciliation"));
    assert!(content.contains("failed policy entity drop did not preserve complete M2 audit state"));
    assert!(content.contains(
        "successful policy entity drop left entity, cases, binding, receipt, or runtime rows"
    ));
}

#[test]
fn test_source_records_dump_configured() {
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("schema.rs");
    let content = fs::read_to_string(&schema_path).expect("schema.rs exists");

    assert!(content.contains("SELECT pg_catalog.pg_extension_config_dump('mdm_internal.source_records'::pg_catalog.regclass, '');"));
    assert!(content.contains("steward_decisions"));
}
