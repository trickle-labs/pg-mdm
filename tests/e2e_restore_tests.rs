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
    assert!(content.contains("mdm_admin.rebind"));
}

#[test]
fn test_source_records_dump_configured() {
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("schema.rs");
    let content = fs::read_to_string(&schema_path).expect("schema.rs exists");

    assert!(content.contains("SELECT pg_catalog.pg_extension_config_dump('mdm_internal.source_records'::pg_catalog.regclass, '');"));
}
