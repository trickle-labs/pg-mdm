use pg_mdm::source_record::quote_identifier;

#[test]
fn test_quote_identifier() {
    assert_eq!(quote_identifier("id"), "\"id\"");
    assert_eq!(quote_identifier("user_id"), "\"user_id\"");
    assert_eq!(quote_identifier("col\"name"), "\"col\"\"name\"");
}
