use pgrx::Uuid;
use pgrx::prelude::*;

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
        "pgtrickle.encode_row_id_v2('MDM_SOURCE_KEY_V1', ROW((SELECT entity_id FROM mdm_internal.entities WHERE entity_name = '{entity_name_escaped}'), (SELECT source_identity_id FROM mdm_internal.source_identities WHERE entity_id = (SELECT entity_id FROM mdm_internal.entities WHERE entity_name = '{entity_name_escaped}') AND source_name = '{source_name_escaped}'), {}))",
        quoted_cols.join(", ")
    );

    Ok(sql)
}

pub fn get_or_create_source_record(
    entity_id: Uuid,
    source_identity_id: Uuid,
    source_record_key: &[u8],
) -> Result<Uuid, MdmError> {
    Spi::connect_mut(|client| {
        let rows = client
            .update(
                "INSERT INTO mdm_internal.source_records \
                    (entity_id, source_identity_id, source_record_key, active, first_seen_at, last_seen_at) \
                 VALUES ($1, $2, $3, false, pg_catalog.statement_timestamp(), pg_catalog.statement_timestamp()) \
                 ON CONFLICT (entity_id, source_identity_id, source_record_key) \
                 DO UPDATE SET last_seen_at = pg_catalog.statement_timestamp() \
                 RETURNING source_record_id",
                Some(1),
                &[
                    entity_id.into(),
                    source_identity_id.into(),
                    source_record_key.into(),
                ],
            )
            .map_err(|e| MdmError::Spi(e.to_string()))?;

        if rows.is_empty() {
            return Err(MdmError::OperationState(
                "get_or_create_source_record returned no row".into(),
            ));
        }

        let id = rows
            .first()
            .get::<Uuid>(1)
            .map_err(|e| MdmError::Spi(e.to_string()))?
            .ok_or_else(|| MdmError::OperationState("source_record_id is NULL".into()))?;

        Ok(id)
    })
}
