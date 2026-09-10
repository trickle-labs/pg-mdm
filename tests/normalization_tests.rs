use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use pg_mdm::normalization::{
    LogicalType, NormalizedState, make_canonical_bytes, normalize_date_pure, normalize_text_pure,
};

#[derive(Debug, Deserialize)]
struct Fixture {
    format_version: i32,
    canonical_encoding_version: u8,
    cases: Vec<TestCase>,
}

#[derive(Debug, Deserialize)]
struct TestCase {
    id: String,
    cleaner: String,
    cleaner_version: i32,
    logical_type: String,
    input: Option<String>,
    source_state: String,
    options: Value,
    expected_state: String,
    expected_normalized: Option<String>,
    expected_canonical_bytes_hex: Option<String>,
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn test_all_fixture_cases() {
    let fixture_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("normalization_v1.json");
    let content = fs::read_to_string(&fixture_path).expect("fixture file exists");
    let fixture: Fixture = serde_json::from_str(&content).expect("valid fixture json");

    assert_eq!(fixture.format_version, 1);
    assert_eq!(fixture.canonical_encoding_version, 1);

    for tc in &fixture.cases {
        let result = if tc.logical_type == "date" {
            let parts = tc.input.as_deref().and_then(|s| {
                let p: Vec<&str> = s.trim().split('-').collect();
                if p.len() == 3 {
                    let y = p[0].parse::<i32>().ok()?;
                    let m = p[1].parse::<u8>().ok()?;
                    let d = p[2].parse::<u8>().ok()?;
                    Some((y, m, d))
                } else {
                    None
                }
            });
            if tc.input.is_some() && parts.is_none() && tc.source_state == "present" {
                // If it's a date string that couldn't even parse into 3 integer parts, pure date returns Invalid
                Ok(pg_mdm::normalization::NormalizedValue {
                    state: NormalizedState::Invalid,
                    normalized: None,
                    canonical_bytes: None,
                })
            } else {
                normalize_date_pure(
                    parts,
                    &tc.cleaner,
                    tc.cleaner_version,
                    &tc.source_state,
                    &tc.options,
                )
            }
        } else {
            normalize_text_pure(
                tc.input.as_deref(),
                &tc.cleaner,
                tc.cleaner_version,
                &tc.source_state,
                &tc.options,
            )
        };

        let norm_val = match result {
            Ok(v) => v,
            Err(e) => panic!("Case {} failed with error: {e}", tc.id),
        };

        assert_eq!(
            norm_val.state.as_str(),
            tc.expected_state,
            "State mismatch in case {}: expected {}, got {}",
            tc.id,
            tc.expected_state,
            norm_val.state.as_str()
        );

        assert_eq!(
            norm_val.normalized, tc.expected_normalized,
            "Normalized mismatch in case {}",
            tc.id
        );

        let hex_bytes = norm_val.canonical_bytes.as_ref().map(|b| to_hex(b));
        assert_eq!(
            hex_bytes, tc.expected_canonical_bytes_hex,
            "Canonical bytes mismatch in case {}",
            tc.id
        );

        // Only the 'value' state can produce candidate or matching evidence
        if norm_val.state == NormalizedState::Value {
            assert!(norm_val.state.can_supply_evidence());
            assert!(norm_val.normalized.is_some());
            assert!(norm_val.canonical_bytes.is_some());
        } else {
            assert!(!norm_val.state.can_supply_evidence());
            assert!(norm_val.normalized.is_none());
            assert!(norm_val.canonical_bytes.is_none());
        }
    }
}

#[test]
fn test_unknown_cleaner_and_version_fail_closed() {
    let err_cleaner = normalize_text_pure(
        Some("test"),
        "unsupported_cleaner",
        1,
        "present",
        &serde_json::json!({}),
    );
    assert!(err_cleaner.is_err());
    let err_cleaner = err_cleaner.unwrap_err();
    assert_eq!(err_cleaner.code(), "MDM_DEFINITION_INVALID");

    let err_version =
        normalize_text_pure(Some("test"), "text", 2, "present", &serde_json::json!({}));
    assert!(err_version.is_err());
    let err_version = err_version.unwrap_err();
    assert_eq!(err_version.code(), "MDM_CLEANER_VERSION");
}

#[test]
fn test_unknown_cleaner_option_fails_closed() {
    let err_option = normalize_text_pure(
        Some("test"),
        "text",
        1,
        "present",
        &serde_json::json!({"unknown_opt": 123}),
    );
    assert!(err_option.is_err());
    let err_option = err_option.unwrap_err();
    assert_eq!(err_option.code(), "MDM_DEFINITION_INVALID");
}

#[test]
fn test_canonically_equivalent_unicode_forms() {
    let val_composed =
        normalize_text_pure(Some("café"), "text", 1, "present", &serde_json::json!({})).unwrap();
    let val_decomposed = normalize_text_pure(
        Some("cafe\u{0301}"),
        "text",
        1,
        "present",
        &serde_json::json!({}),
    )
    .unwrap();

    assert_eq!(val_composed.normalized, val_decomposed.normalized);
    assert_eq!(val_composed.canonical_bytes, val_decomposed.canonical_bytes);
}

#[test]
fn test_canonical_bytes_ordering_stability() {
    let b_alpha = make_canonical_bytes(LogicalType::Text, "alpha");
    let b_beta = make_canonical_bytes(LogicalType::Text, "beta");
    let b_gamma = make_canonical_bytes(LogicalType::Text, "gamma");

    assert!(b_alpha < b_beta);
    assert!(b_beta < b_gamma);

    let d_jan = make_canonical_bytes(LogicalType::Date, "2026-01-15");
    let d_sep = make_canonical_bytes(LogicalType::Date, "2026-09-10");

    assert!(d_jan < d_sep);
}
