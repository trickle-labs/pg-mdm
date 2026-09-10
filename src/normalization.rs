use pgrx::heap_tuple::PgHeapTuple;
use pgrx::prelude::*;
use pgrx::{JsonB, default};
use serde::{Deserialize, Serialize};

use crate::cleaners::date::clean_date_parts;
use crate::cleaners::execute_text_cleaner;
use crate::cleaners::text::CleanResult;
use crate::error::MdmError;

pub(crate) const CANONICAL_ENCODING_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormalizedState {
    Value,
    Absent,
    Empty,
    Invalid,
    Unknown,
    Redacted,
    Unsupported,
}

impl NormalizedState {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Absent => "absent",
            Self::Empty => "empty",
            Self::Invalid => "invalid",
            Self::Unknown => "unknown",
            Self::Redacted => "redacted",
            Self::Unsupported => "unsupported",
        }
    }

    pub const fn can_supply_evidence(&self) -> bool {
        matches!(self, Self::Value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalType {
    Text = 1,
    Date = 2,
}

pub fn make_canonical_bytes(logical_type: LogicalType, normalized: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(2 + normalized.len());
    bytes.push(CANONICAL_ENCODING_VERSION);
    bytes.push(logical_type as u8);
    bytes.extend_from_slice(normalized.as_bytes());
    bytes
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedValue {
    pub state: NormalizedState,
    pub normalized: Option<String>,
    pub canonical_bytes: Option<Vec<u8>>,
}

pub fn normalize_text_pure(
    value: Option<&str>,
    cleaner: &str,
    cleaner_version: i32,
    source_state: &str,
    options: &serde_json::Value,
) -> Result<NormalizedValue, MdmError> {
    execute_text_cleaner(cleaner, cleaner_version, value, source_state, options)
}

pub fn normalize_date_pure(
    value: Option<(i32, u8, u8)>,
    cleaner: &str,
    cleaner_version: i32,
    source_state: &str,
    options: &serde_json::Value,
) -> Result<NormalizedValue, MdmError> {
    crate::cleaners::validate_cleaner_options(cleaner, cleaner_version, options)?;
    if cleaner != "date" {
        return Err(MdmError::DefinitionInvalid(format!(
            "cleaner {cleaner} is not valid for date"
        )));
    }

    match source_state {
        "unknown" => {
            return Ok(NormalizedValue {
                state: NormalizedState::Unknown,
                normalized: None,
                canonical_bytes: None,
            });
        }
        "redacted" => {
            return Ok(NormalizedValue {
                state: NormalizedState::Redacted,
                normalized: None,
                canonical_bytes: None,
            });
        }
        "present" => {}
        other => {
            return Err(MdmError::CleanerExecution(format!(
                "invalid source state: {other}"
            )));
        }
    }

    let Some((year, month, day)) = value else {
        return Ok(NormalizedValue {
            state: NormalizedState::Absent,
            normalized: None,
            canonical_bytes: None,
        });
    };

    let result = clean_date_parts(year, month, day);
    match result {
        CleanResult::Value(norm) => {
            let bytes = make_canonical_bytes(LogicalType::Date, &norm);
            Ok(NormalizedValue {
                state: NormalizedState::Value,
                normalized: Some(norm),
                canonical_bytes: Some(bytes),
            })
        }
        CleanResult::Empty => Ok(NormalizedValue {
            state: NormalizedState::Empty,
            normalized: None,
            canonical_bytes: None,
        }),
        CleanResult::Invalid => Ok(NormalizedValue {
            state: NormalizedState::Invalid,
            normalized: None,
            canonical_bytes: None,
        }),
        CleanResult::Unsupported => Ok(NormalizedValue {
            state: NormalizedState::Unsupported,
            normalized: None,
            canonical_bytes: None,
        }),
    }
}

fn into_heap_tuple(
    val: NormalizedValue,
) -> pgrx::composite_type!('static, "mdm_internal.normalized_value") {
    let mut tuple = match PgHeapTuple::new_composite_type("mdm_internal.normalized_value") {
        Ok(t) => t,
        Err(e) => pgrx::error!("failed to create normalized_value tuple: {e}"),
    };
    if let Err(e) = tuple.set_by_name("state", val.state.as_str()) {
        pgrx::error!("failed to set state attribute: {e}");
    }
    if let Err(e) = tuple.set_by_name("normalized", val.normalized) {
        pgrx::error!("failed to set normalized attribute: {e}");
    }
    if let Err(e) = tuple.set_by_name("canonical_bytes", val.canonical_bytes) {
        pgrx::error!("failed to set canonical_bytes attribute: {e}");
    }
    tuple
}

#[pg_extern(
    name = "normalize_text",
    requires = ["pg_mdm_foundation"],
    sql = "CREATE FUNCTION mdm_internal.normalize_text(value text, cleaner text, cleaner_version integer, source_state text DEFAULT 'present', options jsonb DEFAULT '{}'::jsonb) RETURNS mdm_internal.normalized_value IMMUTABLE PARALLEL SAFE SET search_path = pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'normalize_text_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn normalize_text(
    value: Option<String>,
    cleaner: String,
    cleaner_version: i32,
    source_state: default!(Option<String>, "'present'"),
    options: default!(Option<JsonB>, "'{}'::jsonb"),
) -> pgrx::composite_type!('static, "mdm_internal.normalized_value") {
    let state_str = source_state.as_deref().unwrap_or("present");
    let options_val = options
        .map(|j| j.0)
        .unwrap_or_else(|| serde_json::json!({}));
    match normalize_text_pure(
        value.as_deref(),
        &cleaner,
        cleaner_version,
        state_str,
        &options_val,
    ) {
        Ok(res) => into_heap_tuple(res),
        Err(err) => crate::raise(err),
    }
}

#[pg_extern(
    name = "normalize_date",
    requires = ["pg_mdm_foundation"],
    sql = "CREATE FUNCTION mdm_internal.normalize_date(value date, cleaner text DEFAULT 'date', cleaner_version integer DEFAULT 1, source_state text DEFAULT 'present', options jsonb DEFAULT '{}'::jsonb) RETURNS mdm_internal.normalized_value IMMUTABLE PARALLEL SAFE SET search_path = pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'normalize_date_wrapper';"
)]
#[search_path(pg_catalog, mdm_internal, pg_temp)]
pub(crate) fn normalize_date(
    value: Option<pgrx::datum::Date>,
    cleaner: default!(Option<String>, "'date'"),
    cleaner_version: default!(Option<i32>, "1"),
    source_state: default!(Option<String>, "'present'"),
    options: default!(Option<JsonB>, "'{}'::jsonb"),
) -> pgrx::composite_type!('static, "mdm_internal.normalized_value") {
    let cleaner_str = cleaner.as_deref().unwrap_or("date");
    let cleaner_ver = cleaner_version.unwrap_or(1);
    let state_str = source_state.as_deref().unwrap_or("present");
    let options_val = options
        .map(|j| j.0)
        .unwrap_or_else(|| serde_json::json!({}));

    let date_parts = value.and_then(|d| {
        if d.is_infinity() || d.is_neg_infinity() {
            None
        } else {
            Some((d.year(), d.month(), d.day()))
        }
    });

    let res = if let Some(d) = value
        && (d.is_infinity() || d.is_neg_infinity())
    {
        NormalizedValue {
            state: NormalizedState::Unsupported,
            normalized: None,
            canonical_bytes: None,
        }
    } else {
        match normalize_date_pure(
            date_parts,
            cleaner_str,
            cleaner_ver,
            state_str,
            &options_val,
        ) {
            Ok(res) => res,
            Err(err) => crate::raise(err),
        }
    };

    into_heap_tuple(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_bytes_total_binary_order() {
        let text_a = make_canonical_bytes(LogicalType::Text, "apple");
        let text_b = make_canonical_bytes(LogicalType::Text, "banana");
        let text_c = make_canonical_bytes(LogicalType::Text, "cherry");
        assert!(text_a < text_b);
        assert!(text_b < text_c);

        let date_1 = make_canonical_bytes(LogicalType::Date, "2026-01-01");
        let date_2 = make_canonical_bytes(LogicalType::Date, "2026-09-10");
        assert!(date_1 < date_2);

        // Different types compare by type tag first
        assert!(text_a < date_1);
    }
}
