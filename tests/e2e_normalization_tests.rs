use pg_mdm::definition::parse_entity;
use pg_mdm::graph_spec::{compile, normalized_field_sql, source_record_sql};
use serde_json::json;

#[test]
fn test_e2e_generated_normalization_sql_shape() {
    let entity = parse_entity(json!({
        "name": "customer",
        "sources": [{
            "name": "crm",
            "relation": "public.crm_customer",
            "source_id": ["id"],
            "mode": "tracked",
            "fields": {
                "email": "email_addr",
                "name": {"value": "full_name", "state": "name_status"}
            },
            "row_changed_at": "updated_at",
            "soft_delete_when": null,
            "authority": {}
        }],
        "fields": [
            {
                "name": "email",
                "type": "text",
                "cleaner": "email",
                "cleaner_options": {},
                "display": "masked"
            },
            {
                "name": "name",
                "type": "text",
                "cleaner": "person_name",
                "cleaner_options": {},
                "display": "full"
            },
            {
                "name": "dob",
                "type": "date",
                "cleaner": "date",
                "cleaner_options": {},
                "display": "full"
            }
        ],
        "matches": [{
            "name": "exact_email",
            "fields": ["email"],
            "comparison": "exact",
            "strength": "identity",
            "evidence_group": "email_group",
            "threshold": null,
            "candidate": {"kind": "exact", "field": "email"}
        }],
        "golden_values": [],
        "preset": null,
        "limits": {},
        "execution_role": null
    }))
    .expect("entity parses");

    let source = &entity.sources[0];
    let rec_sql = source_record_sql(&entity.name, source);
    assert!(rec_sql.contains("SELECT 'crm'::text AS source_name"));
    assert!(rec_sql.contains("pgtrickle.encode_row_id_v2('SCAN_KEY'"));
    assert!(rec_sql.contains("\"id\""));
    assert!(rec_sql.contains("\"email_addr\" AS \"email\""));
    assert!(rec_sql.contains("'present'::text AS \"email_state\""));
    assert!(rec_sql.contains("\"full_name\" AS \"name\""));
    assert!(rec_sql.contains("\"name_status\" AS \"name_state\""));
    assert!(rec_sql.contains("\"updated_at\" AS row_changed_at"));
    assert!(rec_sql.contains("FROM public.crm_customer"));

    let email_field = &entity.fields[0];
    let norm_email_sql = normalized_field_sql(email_field, &entity.sources);
    assert!(norm_email_sql.contains("mdm_graph.normalize_text"));
    assert!(norm_email_sql.contains("CROSS JOIN LATERAL mdm_graph.normalize_text"));
    assert!(norm_email_sql.contains(
        "n.state AS state, n.normalized AS normalized, n.canonical_bytes AS canonical_bytes"
    ));
    assert!(norm_email_sql.contains("'email'"));
    assert!(norm_email_sql.contains("FROM @{records/crm}"));

    let dob_field = &entity.fields[2];
    let norm_dob_sql = normalized_field_sql(dob_field, &entity.sources);
    // Dob is not mapped in crm source, so it produces empty result query
    assert!(norm_dob_sql.contains("WHERE false"));

    let graph = compile(&entity);
    assert_eq!(graph["compiler_version"], 5);
    let nodes = graph["nodes"].as_array().expect("nodes array");
    assert!(nodes.iter().any(|n| n["logical_id"] == "records/crm"));
    assert!(nodes.iter().any(|n| n["logical_id"] == "normalized/email"));
    assert!(nodes.iter().any(|n| n["logical_id"] == "normalized/name"));
    assert!(nodes.iter().any(|n| n["logical_id"] == "normalized/dob"));
}
