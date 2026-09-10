use pg_mdm::constraint::{
    DecisionEdge, DecisionKind, manual_components, validate_proposed_decision,
};
use pg_mdm::decision::{edge, next_version, validate_reason};
use pgrx::Uuid;

fn id(value: u8) -> Uuid {
    Uuid::from_bytes([value; 16])
}
fn match_edge(a: u8, b: u8) -> DecisionEdge {
    edge(Uuid::from_bytes([9; 16]), id(a), id(b), DecisionKind::Match)
}
fn not_match_edge(a: u8, b: u8) -> DecisionEdge {
    edge(
        Uuid::from_bytes([8; 16]),
        id(a),
        id(b),
        DecisionKind::NotMatch,
    )
}

#[test]
fn closure_rejects_cannot_link_inside_manual_component() {
    let existing = vec![match_edge(1, 2)];
    let error = validate_proposed_decision(&existing, &not_match_edge(2, 1), 100).unwrap_err();
    assert_eq!(error.code(), "MDM_DECISION_CONTRADICTION");
    assert!(error.to_string().contains(&id(1).to_string()));
}

#[test]
fn coherent_replacement_and_version_checks_work() {
    let existing = vec![not_match_edge(1, 2)];
    assert!(validate_proposed_decision(&existing, &match_edge(1, 3), 100).is_ok());
    assert_eq!(next_version(None, 0).unwrap(), 1);
    assert_eq!(next_version(Some(1), 1).unwrap(), 2);
    assert!(next_version(Some(1), 0).is_err());
    assert!(validate_reason("steward review").is_ok());
    assert!(validate_reason("  ").is_err());
    let components = manual_components(&[match_edge(1, 2), match_edge(2, 3)], 100).unwrap();
    assert_eq!(components.values().next().unwrap().len(), 3);
}
