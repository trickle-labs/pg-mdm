use serde_json::json;

use pg_mdm::definition::source::ValidatedSource;
use pg_mdm::source_record::{quote_identifier, source_key_sql};

#[test]
fn test_quote_identifier() {
    assert_eq!(quote_identifier("id"), "\"id\"");
    assert_eq!(quote_identifier("user_id"), "\"user_id\"");
    assert_eq!(quote_identifier("col\"name"), "\"col\"\"name\"");
}

#[test]
fn test_scalar_source_key_sql() {
    let source = ValidatedSource {
        entity_name: "customer".into(),
        name: "crm".into(),
        relation_name: "public.crm_customer".into(),
        relation_oid: 12345.into(),
        key_contract: json!({
            "key_columns": ["id"],
            "key_types": ["integer"],
            "source_key_encoding": 2
        }),
        identity_digest: vec![0; 32],
        binding_fingerprint: json!({}),
    };

    let sql = source_key_sql(&source).expect("generates sql");
    assert!(sql.starts_with("pgtrickle.encode_row_id_v2('MDM_SOURCE_KEY_V1', ROW("));
    assert!(sql.contains("WHERE entity_name = 'customer'"));
    assert!(sql.contains("AND source_name = 'crm'"));
    assert!(sql.ends_with(", \"id\"))"));
}

#[test]
fn test_composite_source_key_sql() {
    let source = ValidatedSource {
        entity_name: "organization".into(),
        name: "erp".into(),
        relation_name: "public.erp_org".into(),
        relation_oid: 54321.into(),
        key_contract: json!({
            "key_columns": ["tenant_id", "org_id"],
            "key_types": ["integer", "text"],
            "source_key_encoding": 2
        }),
        identity_digest: vec![0; 32],
        binding_fingerprint: json!({}),
    };

    let sql = source_key_sql(&source).expect("generates sql");
    assert!(sql.contains(", \"tenant_id\", \"org_id\"))"));
}

#[test]
fn test_escaped_names_source_key_sql() {
    let source = ValidatedSource {
        entity_name: "cust'omer".into(),
        name: "cr'm".into(),
        relation_name: "public.crm".into(),
        relation_oid: 11111.into(),
        key_contract: json!({
            "key_columns": ["id"],
            "key_types": ["integer"],
            "source_key_encoding": 2
        }),
        identity_digest: vec![0; 32],
        binding_fingerprint: json!({}),
    };

    let sql = source_key_sql(&source).expect("generates sql");
    assert!(sql.contains("WHERE entity_name = 'cust''omer'"));
    assert!(sql.contains("AND source_name = 'cr''m'"));
}

#[test]
fn test_empty_key_columns_rejected() {
    let source = ValidatedSource {
        entity_name: "customer".into(),
        name: "crm".into(),
        relation_name: "public.crm".into(),
        relation_oid: 12345.into(),
        key_contract: json!({
            "key_columns": [],
            "source_key_encoding": 2
        }),
        identity_digest: vec![0; 32],
        binding_fingerprint: json!({}),
    };

    let err = source_key_sql(&source).unwrap_err();
    assert_eq!(err.code(), "MDM_SOURCE_RECORD_INVALID");
}
