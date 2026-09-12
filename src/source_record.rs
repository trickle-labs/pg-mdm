use crate::definition::source::ValidatedSource;
use crate::error::MdmError;

pub fn quote_identifier(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

pub fn source_key_sql(source: &ValidatedSource) -> Result<String, MdmError> {
    let key_columns = source
        .key_contract
        .get("key_columns")
        .and_then(|v| v.as_array())
        .ok_or_else(|| MdmError::SourceRecord("key_columns missing from key_contract".into()))?;

    if key_columns.is_empty() {
        return Err(MdmError::SourceRecord("source key has no columns".into()));
    }

    let mut quoted_cols = Vec::with_capacity(key_columns.len());
    for col in key_columns {
        let col_name = col
            .as_str()
            .ok_or_else(|| MdmError::SourceRecord("key column name must be a string".into()))?;
        quoted_cols.push(quote_identifier(col_name));
    }

    let entity_name_escaped = source.entity_name.replace('\'', "''");
    let source_name_escaped = source.name.replace('\'', "''");

    let sql = format!(
        "pgtrickle.encode_row_id_v2('SCAN_KEY', ROW((SELECT entity_id FROM mdm_internal.entities WHERE entity_name = '{entity_name_escaped}'), (SELECT source_identity_id FROM mdm_internal.source_identities WHERE entity_id = (SELECT entity_id FROM mdm_internal.entities WHERE entity_name = '{entity_name_escaped}') AND source_name = '{source_name_escaped}'), {}))",
        quoted_cols.join(", ")
    );

    Ok(sql)
}
