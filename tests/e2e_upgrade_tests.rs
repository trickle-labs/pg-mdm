use std::fs;
use std::path::Path;

#[test]
fn test_upgrade_scripts_exist() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let upgrade_01_02 = root.join("sql").join("pg_mdm--0.1.0--0.2.0.sql");
    let upgrade_02_03 = root.join("sql").join("pg_mdm--0.2.0--0.3.0.sql");
    let upgrade_03_04 = root.join("sql").join("pg_mdm--0.3.0--0.4.0.sql");
    let upgrade_04_05 = root.join("sql").join("pg_mdm--0.4.0--0.5.0.sql");
    let upgrade_05_06 = root.join("sql").join("pg_mdm--0.5.0--0.6.0.sql");
    let upgrade_06_07 = root.join("sql").join("pg_mdm--0.6.0--0.7.0.sql");
    let upgrade_07_08 = root.join("sql").join("pg_mdm--0.7.0--0.8.0.sql");

    assert!(
        upgrade_01_02.is_file(),
        "0.1.0 -> 0.2.0 upgrade script must exist"
    );
    assert!(
        upgrade_02_03.is_file(),
        "0.2.0 -> 0.3.0 upgrade script must exist"
    );
    assert!(
        upgrade_03_04.is_file(),
        "0.3.0 -> 0.4.0 upgrade script must exist"
    );
    assert!(
        upgrade_04_05.is_file(),
        "0.4.0 -> 0.5.0 upgrade script must exist"
    );
    assert!(
        upgrade_05_06.is_file(),
        "0.5.0 -> 0.6.0 upgrade script must exist"
    );
    assert!(
        upgrade_06_07.is_file(),
        "0.6.0 -> 0.7.0 upgrade script must exist"
    );
    assert!(
        upgrade_07_08.is_file(),
        "0.7.0 -> 0.8.0 upgrade script must exist"
    );

    let sql_02_03 = fs::read_to_string(&upgrade_02_03).expect("read 0.2.0 to 0.3.0");
    assert!(sql_02_03.contains("mdm_internal.normalized_value"));
    assert!(sql_02_03.contains("mdm_internal.source_records"));
    assert!(sql_02_03.contains("mdm_internal.normalize_text"));
    assert!(sql_02_03.contains("mdm_internal.normalize_date"));
    let sql_03_04 = fs::read_to_string(&upgrade_03_04).expect("read 0.3.0 to 0.4.0");
    assert!(sql_03_04.contains("mdm.create"));
    assert!(sql_03_04.contains("mdm.describe"));
    let sql_04_05 = fs::read_to_string(&upgrade_04_05).expect("read 0.4.0 to 0.5.0");
    assert!(sql_04_05.contains("steward_decisions"));
    assert!(sql_04_05.contains("decision_epoch"));
    assert!(sql_04_05.contains("mdm_steward.decide"));
    let sql_05_06 = fs::read_to_string(&upgrade_05_06).expect("read 0.5.0 to 0.6.0");
    assert!(sql_05_06.contains("clustering resolver"));
    let sql_06_07 = fs::read_to_string(&upgrade_06_07).expect("read 0.6.0 to 0.7.0");
    assert!(sql_06_07.contains("identity_registry"));
    assert!(sql_06_07.contains("golden_override_directives"));
    assert!(sql_06_07.contains("mdm.explain"));
    let sql_07_08 = fs::read_to_string(&upgrade_07_08).expect("read 0.7.0 to 0.8.0");
    assert!(sql_07_08.contains("mdm_internal.graph_bindings"));
    assert!(sql_07_08.contains("mdm_admin.drop_entity"));
}
