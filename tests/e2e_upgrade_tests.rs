use std::fs;
use std::path::Path;

#[test]
fn test_upgrade_scripts_exist() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let upgrade_01_02 = root.join("sql").join("pg_mdm--0.1.0--0.2.0.sql");
    let upgrade_02_03 = root.join("sql").join("pg_mdm--0.2.0--0.3.0.sql");

    assert!(
        upgrade_01_02.is_file(),
        "0.1.0 -> 0.2.0 upgrade script must exist"
    );
    assert!(
        upgrade_02_03.is_file(),
        "0.2.0 -> 0.3.0 upgrade script must exist"
    );

    let sql_02_03 = fs::read_to_string(&upgrade_02_03).expect("read 0.2.0 to 0.3.0");
    assert!(sql_02_03.contains("mdm_internal.normalized_value"));
    assert!(sql_02_03.contains("mdm_internal.source_records"));
    assert!(sql_02_03.contains("mdm_internal.normalize_text"));
    assert!(sql_02_03.contains("mdm_internal.normalize_date"));
}
