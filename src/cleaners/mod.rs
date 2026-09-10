pub(crate) mod date;
pub(crate) mod email;
pub(crate) mod phone;
pub(crate) mod tax_id;
pub(crate) mod text;

use serde_json::Value;
use std::collections::BTreeMap;

use crate::cleaners::date::clean_date_str;
use crate::cleaners::email::clean_email;
use crate::cleaners::phone::clean_phone;
use crate::cleaners::tax_id::clean_tax_id;
use crate::cleaners::text::{CleanResult, clean_company_name, clean_person_name, clean_text};
use crate::error::MdmError;
use crate::normalization::{LogicalType, NormalizedState, NormalizedValue, make_canonical_bytes};

pub(crate) const V1_CLEANERS: &[&str] = &[
    "company_name",
    "date",
    "email",
    "none",
    "person_name",
    "phone",
    "tax_id",
    "text",
];

pub(crate) fn is_supported_cleaner(name: &str) -> bool {
    V1_CLEANERS.contains(&name)
}

pub(crate) fn default_max_len(cleaner: &str) -> usize {
    match cleaner {
        "email" => 320,
        "phone" => 64,
        "tax_id" => 128,
        "date" => 64,
        _ => 4096,
    }
}

pub(crate) fn validate_cleaner_options(
    cleaner: &str,
    version: i32,
    options: &Value,
) -> Result<usize, MdmError> {
    if !is_supported_cleaner(cleaner) {
        return Err(MdmError::DefinitionInvalid(format!(
            "unsupported cleaner: {cleaner}"
        )));
    }
    if version != 1 {
        return Err(MdmError::CleanerVersion {
            cleaner: cleaner.to_string(),
            version,
        });
    }

    let default_len = default_max_len(cleaner);
    let Some(obj) = options.as_object() else {
        return Err(MdmError::DefinitionInvalid(format!(
            "cleaner options for {cleaner} must be an object"
        )));
    };

    let mut max_len = default_len;
    for (key, val) in obj {
        match key.as_str() {
            "max_length" => {
                let Some(len) = val.as_u64() else {
                    return Err(MdmError::DefinitionInvalid(
                        "cleaner option max_length must be a positive integer".into(),
                    ));
                };
                if len == 0 {
                    return Err(MdmError::DefinitionInvalid(
                        "cleaner option max_length must be greater than zero".into(),
                    ));
                }
                max_len = (len as usize).min(default_len);
            }
            unknown => {
                return Err(MdmError::DefinitionInvalid(format!(
                    "cleaner {cleaner} does not support option {unknown}"
                )));
            }
        }
    }
    Ok(max_len)
}

pub(crate) fn v1_cleaner_registry() -> BTreeMap<String, i32> {
    let mut map = BTreeMap::new();
    for cleaner in V1_CLEANERS {
        map.insert((*cleaner).to_string(), 1);
    }
    map
}

pub(crate) fn clean_none(input: &str, max_len: usize) -> CleanResult {
    if input.chars().count() > max_len {
        return CleanResult::Invalid;
    }
    let trimmed = input.trim();
    if trimmed.is_empty() {
        CleanResult::Empty
    } else {
        CleanResult::Value(input.to_string())
    }
}

pub fn execute_text_cleaner(
    cleaner: &str,
    version: i32,
    input: Option<&str>,
    source_state: &str,
    options: &Value,
) -> Result<NormalizedValue, MdmError> {
    let max_len = validate_cleaner_options(cleaner, version, options)?;

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

    let Some(raw) = input else {
        return Ok(NormalizedValue {
            state: NormalizedState::Absent,
            normalized: None,
            canonical_bytes: None,
        });
    };

    let result = match cleaner {
        "text" => clean_text(raw, max_len),
        "person_name" => clean_person_name(raw, max_len),
        "company_name" => clean_company_name(raw, max_len),
        "email" => clean_email(raw, max_len),
        "phone" => clean_phone(raw, max_len),
        "tax_id" => clean_tax_id(raw, max_len),
        "date" => clean_date_str(raw),
        "none" => clean_none(raw, max_len),
        unknown => {
            return Err(MdmError::CleanerExecution(format!(
                "unsupported cleaner: {unknown}"
            )));
        }
    };

    let logical_type = if cleaner == "date" {
        LogicalType::Date
    } else {
        LogicalType::Text
    };

    match result {
        CleanResult::Value(norm) => {
            let bytes = make_canonical_bytes(logical_type, &norm);
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
