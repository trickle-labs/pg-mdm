use pgrx::JsonB;
use pgrx::prelude::*;
use serde_json::{Value, json};

use crate::definition::{Entity, Source};
use crate::error::MdmError;

#[derive(Clone, Debug)]
pub(crate) struct ValidatedSource {
    pub name: String,
    pub relation_name: String,
    pub relation_oid: pg_sys::Oid,
    pub key_contract: Value,
    pub identity_digest: Vec<u8>,
    pub binding_fingerprint: Value,
}

#[derive(Debug)]
struct RelationInfo {
    oid: pg_sys::Oid,
    kind: String,
    persistence: String,
    row_security: bool,
    force_row_security: bool,
    owner_privileges: bool,
    policies: Value,
}

#[derive(Debug)]
struct ColumnInfo {
    type_name: String,
    base_type: String,
    collation: Option<String>,
}

fn relation_info(source: &Source) -> Result<RelationInfo, MdmError> {
    let oid = Spi::get_one_with_args::<pg_sys::Oid>(
        "SELECT pg_catalog.to_regclass($1)::pg_catalog.oid",
        &[source.relation.clone().into()],
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .ok_or_else(|| {
        MdmError::SourceInvalid(format!("relation {} does not exist", source.relation))
    })?;
    // Keep the source descriptor stable until catalog persistence commits.
    // SAFETY: relation_open checks this catalog OID and acquires the lock;
    // NoLock closes its descriptor while retaining the lock until transaction end.
    unsafe {
        let relation = pg_sys::relation_open(oid, pg_sys::AccessShareLock as pg_sys::LOCKMODE);
        pg_sys::relation_close(relation, pg_sys::NoLock as pg_sys::LOCKMODE);
    }
    Spi::connect(|client| {
        let table = client
            .select(
                "SELECT c.relkind::text, c.relpersistence::text, c.relrowsecurity, c.relforcerowsecurity, pg_catalog.pg_has_role(current_user, c.relowner, 'USAGE'), COALESCE((SELECT jsonb_agg(jsonb_build_object('name', p.polname::text, 'permissive', p.polpermissive, 'roles', p.polroles::text, 'using', pg_catalog.pg_get_expr(p.polqual, p.polrelid), 'check', pg_catalog.pg_get_expr(p.polwithcheck, p.polrelid)) ORDER BY p.polname) FROM pg_catalog.pg_policy p WHERE p.polrelid = c.oid), '[]'::jsonb) FROM pg_catalog.pg_class c WHERE c.oid = $1",
                Some(1),
                &[oid.into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if table.is_empty() {
            return Err(MdmError::SourceInvalid(format!(
                "relation {} disappeared",
                source.relation
            )));
        }
        let row = table.first();
        Ok(RelationInfo {
            oid,
            kind: row
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("relation kind is NULL".into()))?,
            persistence: row
                .get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("relation persistence is NULL".into()))?,
            row_security: row
                .get::<bool>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
            force_row_security: row
                .get::<bool>(4)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false),
            owner_privileges: row
                .get::<bool>(5)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("relation ownership check is NULL".into()))?,
            policies: row
                .get::<JsonB>(6)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .map(|value| value.0)
                .unwrap_or_else(|| json!([])),
        })
    })
}

fn column_info(relation_oid: pg_sys::Oid, name: &str) -> Result<ColumnInfo, MdmError> {
    Spi::connect(|client| {
        let table = client
            .select(
                "SELECT pg_catalog.format_type(a.atttypid, a.atttypmod), pg_catalog.format_type(a.atttypid, NULL), CASE WHEN a.attcollation = 0 THEN NULL ELSE a.attcollation::pg_catalog.regcollation::text END FROM pg_catalog.pg_attribute a WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped AND a.attname = $2",
                Some(1),
                &[relation_oid.into(), name.into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        if table.is_empty() {
            return Err(MdmError::SourceInvalid(format!(
                "column {name} does not exist"
            )));
        }
        let row = table.first();
        Ok(ColumnInfo {
            type_name: row
                .get::<String>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("column type is NULL".into()))?,
            base_type: row
                .get::<String>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .ok_or_else(|| MdmError::Spi("column type is NULL".into()))?,
            collation: row
                .get::<String>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?,
        })
    })
}

fn has_select(relation_oid: pg_sys::Oid, column: Option<&str>) -> Result<bool, MdmError> {
    let sql = match column {
        Some(_) => "SELECT pg_catalog.has_column_privilege(current_user, $1, $2, 'SELECT')",
        None => "SELECT pg_catalog.has_table_privilege(current_user, $1, 'SELECT')",
    };
    let args = match column {
        Some(column) => vec![relation_oid.into(), column.into()],
        None => vec![relation_oid.into()],
    };
    Spi::get_one_with_args::<bool>(sql, &args)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::Spi("privilege check returned NULL".into()))
}

fn key_contract(
    relation: &RelationInfo,
    relation_name: &str,
    source_id: &[String],
) -> Result<Value, MdmError> {
    let mut types = Vec::with_capacity(source_id.len());
    let mut collations = Vec::with_capacity(source_id.len());
    for name in source_id {
        let column = column_info(relation.oid, name)?;
        types.push(column.type_name);
        collations.push(column.collation);
    }

    let index = Spi::connect(|client| {
        let table = client
            .select(
                "SELECT i.indisprimary, i.indisunique, i.indimmediate, i.indisvalid, i.indisready, i.indpred IS NULL, i.indexprs IS NULL, i.indnullsnotdistinct, COALESCE(array_agg(a.attname::text ORDER BY k.ord) FILTER (WHERE k.ord <= i.indnkeyatts AND k.attnum > 0), ARRAY[]::text[]), bool_and(CASE WHEN k.ord <= i.indnkeyatts AND k.attnum > 0 THEN a.attnotnull ELSE true END), bool_or(k.ord <= i.indnkeyatts AND k.attnum = 0) FROM pg_catalog.pg_index i CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY k(attnum, ord) LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum WHERE i.indrelid = $1 AND i.indisvalid AND i.indisready GROUP BY i.indexrelid, i.indisprimary, i.indisunique, i.indimmediate, i.indpred, i.indexprs, i.indnullsnotdistinct ORDER BY i.indisprimary DESC, i.indexrelid LIMIT 32",
                None,
                &[relation.oid.into()],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        for row in table {
            let primary = row
                .get::<bool>(1)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false);
            let unique = row
                .get::<bool>(2)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false);
            let immediate = row
                .get::<bool>(3)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false);
            let no_predicate = row
                .get::<bool>(6)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false);
            let no_expression = row
                .get::<bool>(7)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false);
            let nulls_not_distinct = row
                .get::<bool>(8)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false);
            let columns = row
                .get::<Vec<String>>(9)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or_default();
            let all_not_null = row
                .get::<bool>(10)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(false);
            let has_expression = row
                .get::<bool>(11)
                .map_err(|error| MdmError::Spi(error.to_string()))?
                .unwrap_or(true);
            if (primary || unique)
                && immediate
                && no_predicate
                && no_expression
                && !has_expression
                && columns == source_id
                && (all_not_null || nulls_not_distinct)
            {
                return Ok(Some((primary, nulls_not_distinct)));
            }
        }
        Ok::<_, MdmError>(None)
    })?;
    let Some((primary, nulls_not_distinct)) = index else {
        return Err(MdmError::SourceInvalid(format!(
            "source key {:?} is not backed by a valid immediate unique index",
            source_id
        )));
    };
    Ok(json!({
        "relation_name": relation_name,
        "key_columns": source_id,
        "key_types": types,
        "key_collations": collations,
        "source_key_encoding": 2,
        "primary": primary,
        "nulls_not_distinct": nulls_not_distinct
    }))
}

fn logical_type_matches(logical: &str, physical: &str) -> bool {
    match logical {
        "text" => matches!(physical, "text" | "character varying" | "character"),
        "timestamp" => matches!(
            physical,
            "timestamp without time zone" | "timestamp with time zone"
        ),
        other => physical == other,
    }
}

fn validate_mapping(
    relation: &RelationInfo,
    source: &Source,
    entity: &Entity,
) -> Result<(), MdmError> {
    for (field_name, mapping) in &source.fields {
        if !entity.fields.iter().any(|field| field.name == *field_name) {
            return Err(MdmError::DefinitionInvalid(format!(
                "source {} maps unknown field {field_name}",
                source.name
            )));
        }
        let (value_name, state_name) = match mapping {
            Value::Null => continue,
            Value::String(value) => (value.as_str(), None),
            Value::Object(object) => (
                object.get("value").and_then(Value::as_str).ok_or_else(|| {
                    MdmError::SourceInvalid("field mapping value is required".into())
                })?,
                object
                    .get("state")
                    .map(|state| {
                        state.as_str().ok_or_else(|| {
                            MdmError::SourceInvalid(
                                "field mapping state must be a column name".into(),
                            )
                        })
                    })
                    .transpose()?,
            ),
            _ => {
                return Err(MdmError::SourceInvalid(
                    "field mapping must be a column name or object".into(),
                ));
            }
        };
        let field = entity
            .fields
            .iter()
            .find(|field| field.name == *field_name)
            .expect("checked above");
        let value = column_info(relation.oid, value_name)?;
        if !has_select(relation.oid, Some(value_name))? {
            return Err(MdmError::SourceInvalid(format!(
                "execution role cannot SELECT column {value_name}"
            )));
        }
        if !logical_type_matches(&field.logical_type, &value.base_type) {
            return Err(MdmError::SourceInvalid(format!(
                "field {field_name} expects {}, got {}",
                field.logical_type, value.type_name
            )));
        }
        if let Some(state_name) = state_name {
            let state = column_info(relation.oid, state_name)?;
            if state.base_type != "text" && state.base_type != "character varying" {
                return Err(MdmError::SourceInvalid(format!(
                    "state column {state_name} must be text"
                )));
            }
            if !has_select(relation.oid, Some(state_name))? {
                return Err(MdmError::SourceInvalid(format!(
                    "execution role cannot SELECT column {state_name}"
                )));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_source(
    source: &Source,
    entity: &Entity,
) -> Result<ValidatedSource, MdmError> {
    if source.source_id.is_empty() || source.source_id.iter().any(|name| name.is_empty()) {
        return Err(MdmError::SourceInvalid(format!(
            "source {} has an empty key",
            source.name
        )));
    }
    if source.mode != "tracked" && source.mode != "soft_delete" {
        return Err(MdmError::SourceInvalid(format!(
            "source {} has unsupported mode",
            source.name
        )));
    }
    let relation = relation_info(source)?;
    if relation.kind != "r" && relation.kind != "p" {
        return Err(MdmError::SourceInvalid(format!(
            "source {} must be an ordinary or partitioned table",
            source.name
        )));
    }
    if relation.persistence == "t" {
        return Err(MdmError::SourceInvalid(format!(
            "source {} cannot be temporary",
            source.name
        )));
    }
    if !has_select(relation.oid, None)? {
        return Err(MdmError::SourceInvalid(format!(
            "execution role cannot SELECT {}",
            source.relation
        )));
    }
    if relation.row_security && relation.owner_privileges && !relation.force_row_security {
        return Err(MdmError::SourceInvalid(format!(
            "source {} has RLS enabled but the execution role has owner privileges; use FORCE ROW LEVEL SECURITY",
            source.name
        )));
    }
    let key_contract = key_contract(&relation, &source.relation, &source.source_id)?;
    validate_mapping(&relation, source, entity)?;

    if let Some(column) = &source.row_changed_at {
        let info = column_info(relation.oid, column)?;
        if !matches!(
            info.base_type.as_str(),
            "timestamp without time zone" | "timestamp with time zone" | "date"
        ) {
            return Err(MdmError::SourceInvalid(format!(
                "row_changed_at column {column} is not temporal"
            )));
        }
        if !has_select(relation.oid, Some(column))? {
            return Err(MdmError::SourceInvalid(format!(
                "execution role cannot SELECT column {column}"
            )));
        }
    }
    if source.mode == "soft_delete" {
        let predicate = source.soft_delete_when.as_ref().ok_or_else(|| {
            MdmError::SourceInvalid(format!("source {} requires soft_delete_when", source.name))
        })?;
        let object = predicate
            .as_object()
            .ok_or_else(|| MdmError::SourceInvalid("soft_delete_when must be an object".into()))?;
        let column = object
            .get("column")
            .and_then(Value::as_str)
            .ok_or_else(|| MdmError::SourceInvalid("soft_delete_when.column is required".into()))?;
        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| MdmError::SourceInvalid("soft_delete_when.kind is required".into()))?;
        if !matches!(kind, "is_true" | "is_not_null") {
            return Err(MdmError::SourceInvalid(
                "soft_delete_when.kind must be is_true or is_not_null".into(),
            ));
        }
        let info = column_info(relation.oid, column)?;
        if kind == "is_true" && info.base_type != "boolean" {
            return Err(MdmError::SourceInvalid(format!(
                "soft-delete column {column} must be boolean"
            )));
        }
        if !has_select(relation.oid, Some(column))? {
            return Err(MdmError::SourceInvalid(format!(
                "execution role cannot SELECT column {column}"
            )));
        }
    } else if source.soft_delete_when.is_some() {
        return Err(MdmError::SourceInvalid(
            "tracked sources cannot define soft_delete_when".into(),
        ));
    }
    let relation_name = source.relation.clone();
    let identity_digest = crate::definition::canonical::digest(
        "pg_mdm/source-identity/v1",
        &[&crate::definition::canonical::json_bytes(&key_contract)],
    );
    let binding_fingerprint = json!({
        "relation_oid": relation.oid.to_u32(),
        "relation_name": source.relation,
        "relkind": relation.kind,
        "relpersistence": relation.persistence,
        "row_security": relation.row_security,
        "force_row_security": relation.force_row_security,
        "policies": relation.policies
    });
    Ok(ValidatedSource {
        name: source.name.clone(),
        relation_name,
        relation_oid: relation.oid,
        key_contract,
        identity_digest,
        binding_fingerprint,
    })
}
