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
    let candidate_raw = nodes
        .iter()
        .find(|node| node["logical_id"] == "candidate-raw/organization")
        .unwrap();
    assert_eq!(candidate_raw["refresh_mode"], "DIFFERENTIAL");
    assert_eq!(candidate_raw["output_schema"]["candidate_channel"], "text");
    assert_eq!(
        candidate_raw["output_schema"]["candidate_block_key"],
        "text"
    );
    assert_eq!(
        candidate_raw["dependencies"],
        json!([
            "blocks/same_email",
            "block-stats/same_email",
            "block-overflow/same_email",
            "blocks/same_name",
            "block-stats/same_name",
            "block-overflow/same_name"
        ])
    );
    let raw_sql = candidate_raw["defining_sql"].as_str().unwrap();
    assert!(raw_sql.contains("source_sort_key < r.source_sort_key"));
    assert!(raw_sql.contains("@{blocks/same_email}"));
    assert!(raw_sql.contains("@{block-stats/same_email}"));
    assert!(raw_sql.contains("@{block-overflow/same_email}"));
    assert!(!raw_sql.contains("GROUP BY"));
    let channel_pairs = raw_sql.split("\nUNION ALL\n").collect::<Vec<_>>();
    assert_eq!(channel_pairs.len(), 2);
    assert!(
        channel_pairs
            .iter()
            .any(|sql| sql.contains("@{blocks/same_email}"))
    );
    assert!(
        channel_pairs
            .iter()
            .any(|sql| sql.contains("@{blocks/same_name}"))
    );

    let candidate_set = nodes
        .iter()
        .find(|node| node["logical_id"] == "candidate-set/organization")
        .unwrap();
    assert_eq!(candidate_set["refresh_mode"], "DIFFERENTIAL");
    assert_eq!(
        candidate_set["dependencies"],
        json!(["candidate-raw/organization"])
    );
    let candidate_set_sql = candidate_set["defining_sql"].as_str().unwrap();
    assert!(candidate_set_sql.contains("FROM @{candidate-raw/organization} AS candidate"));
    assert!(
        candidate_set_sql
            .contains("GROUP BY candidate.left_source_record_id, candidate.right_source_record_id")
    );
    assert!(candidate_set_sql.contains("pg_catalog.min(candidate.left_sort_key)"));
    assert!(candidate_set_sql.contains("pg_catalog.min(candidate.right_sort_key)"));
    assert_eq!(candidate_set["output_schema"]["left_sort_key"], "bytea");
    assert_eq!(candidate_set["output_schema"]["right_sort_key"], "bytea");
    let pair = nodes
        .iter()
        .find(|node| node["logical_id"] == "pairs/organization")
        .unwrap();
    assert_eq!(
        pair["dependencies"],
        json!([
            "candidate-set/organization",
            "normalized/email",
            "normalized/name",
            "pair-overflow/organization"
        ])
    );
    let pair_sql = pair["defining_sql"].as_str().unwrap();
    assert!(pair_sql.contains("FROM @{candidate-set/organization} AS candidate"));
    assert!(pair_sql.contains("LEFT JOIN @{normalized/email} AS evidence_left_0"));
    assert!(pair_sql.contains("LEFT JOIN @{normalized/name} AS evidence_left_1"));
    assert!(pair_sql.contains("LEFT JOIN @{pair-overflow/organization} AS overflow ON true"));
    for index in 0..2 {
        assert!(pair_sql.contains(&format!("AS \"evidence_rule_{index}_class\"")));
        assert!(pair_sql.contains(&format!("AS \"evidence_rule_{index}_score\"")));
    }
    assert_eq!(pair["output_schema"]["left_source_record_id"], "uuid");
    assert_eq!(pair["output_schema"]["left_sort_key"], "bytea");
    assert_eq!(pair["output_schema"]["left_evidence_email_state"], "text");
    assert_eq!(pair["output_schema"]["evidence_rule_0_class"], "text");
    assert_eq!(
        pair["output_schema"]["evidence_rule_1_left_value_digest"],
        "bytea"
    );
    let exact_block = nodes
        .iter()
        .find(|node| node["logical_id"] == "blocks/same_email")
        .unwrap();
    assert_eq!(exact_block["refresh_mode"], "DIFFERENTIAL");
    assert_eq!(exact_block["output_schema"]["block_key"], "bytea");
    assert_eq!(exact_block["output_schema"]["field_name"], "text");
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
    assert_eq!(token_block["output_schema"]["block_key"], "text");
    assert!(
        token_block["defining_sql"]
            .as_str()
            .unwrap()
            .contains("token AS block_key")
    );
    let pair_stats = nodes
        .iter()
        .find(|node| node["logical_id"] == "pair-stats/organization")
        .unwrap();
    assert_eq!(pair_stats["refresh_mode"], "DIFFERENTIAL");
    assert_eq!(
        pair_stats["dependencies"],
        json!(["candidate-set/organization"])
    );
    assert_eq!(
        pair_stats["defining_sql"],
        "SELECT pg_catalog.count(*)::bigint AS candidate_pairs FROM @{candidate-set/organization}"
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
    assert!(pair_sql.contains("WHERE NOT EXISTS"));
    assert!(pair_sql.contains("prior.candidate_channel < candidate.candidate_channel"));
    assert!(pair_sql.contains("prior.candidate_block_key < candidate.candidate_block_key"));
    assert!(pair_sql.contains("\nUNION ALL\n"));
    assert!(!pair_sql.contains("\nUNION\n"));
}

#[test]
fn candidate_pair_stage_projects_pair_and_evidence_columns() {
    let entity = parse_entity(json!({
        "name": "projection_fixture",
        "sources": [],
        "fields": [
            {"name": "name", "type": "text", "cleaner": "text", "cleaner_options": {}, "display": "full"}
        ],
        "matches": [
            {"name": "same_name", "fields": ["name"], "comparison": "exact", "strength": "identity", "evidence_group": "name", "threshold": null, "candidate": {"kind": "exact", "field": "name"}}
        ],
        "golden_values": [], "preset": null, "limits": {}, "execution_role": null
    })).unwrap();
    let graph = compile(&entity);
    let pairs = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["logical_id"] == "pairs/projection_fixture")
        .unwrap();

    assert_eq!(
        pairs["defining_sql"],
        r#"SELECT candidate.left_source_record_id, candidate.right_source_record_id, candidate.left_sort_key, candidate.right_sort_key, evidence_left_0.state AS "left_evidence_name_state", evidence_left_0.normalized AS "left_evidence_name_normalized", evidence_left_0.canonical_bytes AS "left_evidence_name_canonical_bytes", evidence_right_0.state AS "right_evidence_name_state", evidence_right_0.normalized AS "right_evidence_name_normalized", evidence_right_0.canonical_bytes AS "right_evidence_name_canonical_bytes", CASE WHEN evidence_left_0.state = 'value' AND evidence_right_0.state = 'value' AND evidence_left_0.canonical_bytes IS NOT NULL AND evidence_right_0.canonical_bytes IS NOT NULL THEN CASE WHEN evidence_left_0.canonical_bytes = evidence_right_0.canonical_bytes THEN 'agree'::text ELSE 'disagree'::text END ELSE 'no_evidence'::text END AS "evidence_rule_0_class", CASE WHEN evidence_left_0.state = 'value' AND evidence_right_0.state = 'value' AND evidence_left_0.canonical_bytes IS NOT NULL AND evidence_right_0.canonical_bytes IS NOT NULL THEN NULL::smallint ELSE NULL::smallint END AS "evidence_rule_0_score", CASE WHEN evidence_left_0.state = 'value' AND evidence_right_0.state = 'value' AND evidence_left_0.canonical_bytes IS NOT NULL AND evidence_right_0.canonical_bytes IS NOT NULL THEN mdm_graph.evidence_digest(COALESCE(evidence_left_0.normalized, ''::text)) ELSE NULL::bytea END AS "evidence_rule_0_left_value_digest", CASE WHEN evidence_left_0.state = 'value' AND evidence_right_0.state = 'value' AND evidence_left_0.canonical_bytes IS NOT NULL AND evidence_right_0.canonical_bytes IS NOT NULL THEN mdm_graph.evidence_digest(COALESCE(evidence_right_0.normalized, ''::text)) ELSE NULL::bytea END AS "evidence_rule_0_right_value_digest"
FROM @{candidate-set/projection_fixture} AS candidate
LEFT JOIN @{normalized/name} AS evidence_left_0 ON evidence_left_0.source_record_id = candidate.left_source_record_id AND evidence_left_0.field_name = 'name'
LEFT JOIN @{normalized/name} AS evidence_right_0 ON evidence_right_0.source_record_id = candidate.right_source_record_id AND evidence_right_0.field_name = 'name'
LEFT JOIN @{pair-overflow/projection_fixture} AS overflow ON true
WHERE overflow.candidate_pairs IS NULL"#
    );
    assert!(
        !pairs["defining_sql"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .any(|column| column == "*")
    );
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
