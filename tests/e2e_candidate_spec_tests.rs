use pg_mdm::definition::parse_entity;
use pg_mdm::graph_spec::{candidate_block_overflow_sql, candidate_pairs_sql, compile};
use pg_mdm::semantics::candidate_limits;
use serde_json::json;

#[test]
fn candidate_graph_has_separate_limit_and_pair_stages() {
    let entity = parse_entity(json!({
        "name": "organization",
        "sources": [],
        "fields": [
            {"name": "email", "type": "text", "cleaner": "email", "cleaner_options": {}, "display": "masked"},
            {"name": "name", "type": "text", "cleaner": "text", "cleaner_options": {}, "display": "full"}
        ],
        "matches": [
            {"name": "same_email", "fields": ["email"], "comparison": "exact", "strength": "identity", "evidence_group": "email", "threshold": null, "candidate": {"kind": "exact", "field": "email"}},
            {"name": "same_name", "fields": ["name"], "comparison": "exact", "strength": "strong", "evidence_group": "name", "threshold": null, "candidate": {"kind": "token", "field": "name", "min_length": 4}}
        ],
        "golden_values": [], "preset": null, "limits": {}, "execution_role": null
    })).unwrap();
    let graph = compile(&entity);
    let nodes = graph["nodes"].as_array().unwrap();
    assert!(
        nodes
            .iter()
            .any(|node| node["logical_id"] == "block-stats/same_email")
    );
    assert!(
        nodes
            .iter()
            .any(|node| node["logical_id"] == "block-overflow/same_email")
    );
    let pair = nodes
        .iter()
        .find(|node| node["logical_id"] == "pairs/organization")
        .unwrap();
    let pair_sql = pair["defining_sql"].as_str().unwrap();
    assert!(pair_sql.contains("@{block-stats/same_email}"));
    assert!(pair_sql.contains("source_sort_key < r.source_sort_key"));
    let exact_block = nodes
        .iter()
        .find(|node| node["logical_id"] == "blocks/same_email")
        .unwrap();
    assert_eq!(exact_block["output_schema"]["block_key"], "bytea");
    assert!(
        exact_block["defining_sql"]
            .as_str()
            .unwrap()
            .contains("canonical_bytes AS block_key")
    );
    let token_block = nodes
        .iter()
        .find(|node| node["logical_id"] == "blocks/same_name")
        .unwrap();
    assert_eq!(token_block["output_schema"]["block_key"], "bytea");
    assert!(
        token_block["defining_sql"]
            .as_str()
            .unwrap()
            .contains("convert_to(token, 'UTF8') AS block_key")
    );
    assert!(
        candidate_block_overflow_sql(
            &pg_mdm::candidate::CandidatePlan::from_entity(&entity)
                .unwrap()
                .channels[0],
            &candidate_limits(),
        )
        .contains("block_records > 10000")
    );
    let pair_sql = candidate_pairs_sql(
        &pg_mdm::candidate::CandidatePlan::from_entity(&entity)
            .unwrap()
            .channels,
        &candidate_limits(),
    );
    assert!(pair_sql.contains("GROUP BY l.source_record_id, r.source_record_id"));
    assert!(pair_sql.contains("\nUNION\n"));
}

#[test]
fn composite_candidate_keys_are_length_delimited_bytea() {
    let entity = parse_entity(json!({
        "name": "person",
        "sources": [],
        "fields": [
            {"name": "first", "type": "text", "cleaner": "text", "cleaner_options": {}, "display": "full"},
            {"name": "last", "type": "text", "cleaner": "text", "cleaner_options": {}, "display": "full"}
        ],
        "matches": [
            {"name": "same_person", "fields": ["first", "last"], "comparison": "exact", "strength": "identity", "evidence_group": "person", "threshold": null, "candidate": {"kind": "composite_exact"}}
        ],
        "golden_values": [], "preset": null, "limits": {}, "execution_role": null
    })).unwrap();
    let graph = compile(&entity);
    let block = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["logical_id"] == "blocks/same_person")
        .unwrap();
    assert_eq!(block["output_schema"]["block_key"], "bytea");
    let sql = block["defining_sql"].as_str().unwrap();
    assert!(sql.contains("pg_catalog.int4send(pg_catalog.octet_length(n0.canonical_bytes))"));
    assert!(sql.contains("n0.canonical_bytes || pg_catalog.int4send"));
}
