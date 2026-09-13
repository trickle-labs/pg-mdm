use pg_mdm::candidate::{CandidateLimits, CandidatePlan};
use serde_json::{Value, json};

#[test]
fn candidate_fixture_freezes_each_v1_channel_contract() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/candidate_cases.json")).unwrap();
    assert_eq!(fixture["format_version"], 1);

    let plan = CandidatePlan::from_json(&fixture).unwrap();
    let limits = CandidateLimits {
        max_block_records: fixture["limits"]["max_block_records"].as_u64().unwrap() as usize,
        max_candidate_pairs: fixture["limits"]["max_candidate_pairs"].as_u64().unwrap() as usize,
    };
    let contract = plan.to_json_with_warning(
        &limits,
        fixture["limits"]["warning_block_records"].as_u64().unwrap() as usize,
    );

    assert_eq!(contract["channels"], fixture["channels"]);
    assert_eq!(
        contract,
        json!({
            "format_version": 1,
            "channels": fixture["channels"],
            "max_block_records": 100,
            "max_candidate_pairs": 1000,
            "warning_block_records": 50
        })
    );
    assert_eq!(plan.channels.len(), 4);
}
