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
    assert!(
        candidate_block_overflow_sql(
            &pg_mdm::candidate::CandidatePlan::from_entity(&entity)
                .unwrap()
                .channels[0],
            &candidate_limits(),
        )
        .contains("block_records > 10000")
    );
    assert!(
        candidate_pairs_sql(
            &pg_mdm::candidate::CandidatePlan::from_entity(&entity)
                .unwrap()
                .channels,
            &candidate_limits(),
        )
        .contains("array_agg(DISTINCT channel_id")
    );
}
