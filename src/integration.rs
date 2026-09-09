use std::collections::BTreeMap;

use pgrx::JsonB;
use pgrx::prelude::*;
use serde::Serialize;
use serde_json::Value;

use crate::error::MdmError;
use crate::version::{DELTA_CAPABILITY, GRAPH_CAPABILITY};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Capability {
    pub major: i16,
    pub minor: i16,
    pub enabled: bool,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PgTrickleCapabilities {
    pub external_graph_refresh: Capability,
    pub output_delta_consumer: Option<Capability>,
}

#[derive(Debug, Clone)]
struct RawCapability {
    capability: String,
    major: i16,
    minor: i16,
    enabled: bool,
    details: String,
}

fn parse_capabilities(
    rows: impl IntoIterator<Item = RawCapability>,
) -> Result<PgTrickleCapabilities, MdmError> {
    let mut found = BTreeMap::new();
    for row in rows {
        if !matches!(row.capability.as_str(), GRAPH_CAPABILITY | DELTA_CAPABILITY) {
            continue;
        }
        if row.major < 0 || row.minor < 0 {
            return Err(MdmError::CapabilityInvalid(format!(
                "{} has a negative version",
                row.capability
            )));
        }
        let name = row.capability.clone();
        let capability = Capability {
            major: row.major,
            minor: row.minor,
            enabled: row.enabled,
            details: serde_json::from_str(&row.details).map_err(|error| {
                MdmError::CapabilityInvalid(format!("{name} details are not JSON: {error}"))
            })?,
        };
        if !capability.details.is_object() {
            return Err(MdmError::CapabilityInvalid(format!(
                "{name} details must be a JSON object"
            )));
        }
        if found.insert(row.capability, capability).is_some() {
            return Err(MdmError::CapabilityInvalid(format!("duplicate {name} row")));
        }
    }

    let graph = found
        .remove(GRAPH_CAPABILITY)
        .ok_or(MdmError::CapabilityMissing(GRAPH_CAPABILITY))?;
    if graph.major != 1 {
        return Err(MdmError::CapabilityVersion {
            capability: GRAPH_CAPABILITY.to_string(),
            major: graph.major,
        });
    }

    Ok(PgTrickleCapabilities {
        external_graph_refresh: graph,
        output_delta_consumer: found.remove(DELTA_CAPABILITY),
    })
}

pub(crate) fn integration_capabilities() -> Result<PgTrickleCapabilities, MdmError> {
    Spi::connect(|client| {
        let table = client
            .select(
                "SELECT capability::text, major_version, minor_version, enabled, details::text FROM pgtrickle.integration_capabilities()",
                None,
                &[],
            )
            .map_err(|error| MdmError::Spi(error.to_string()))?;
        let mut rows = Vec::with_capacity(table.len());
        for row in table {
            let required = |index, name| {
                row.get::<String>(index)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::CapabilityInvalid(format!("{name} is NULL")))
            };
            rows.push(RawCapability {
                capability: required(1, "capability")?,
                major: row
                    .get::<i16>(2)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::CapabilityInvalid("major_version is NULL".into()))?,
                minor: row
                    .get::<i16>(3)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::CapabilityInvalid("minor_version is NULL".into()))?,
                enabled: row
                    .get::<bool>(4)
                    .map_err(|error| MdmError::Spi(error.to_string()))?
                    .ok_or_else(|| MdmError::CapabilityInvalid("enabled is NULL".into()))?,
                details: required(5, "details")?,
            });
        }
        parse_capabilities(rows)
    })
}

pub(crate) fn require_graph_v1() -> Result<Capability, MdmError> {
    let capability = integration_capabilities()?.external_graph_refresh;
    capability
        .enabled
        .then_some(capability)
        .ok_or(MdmError::GraphCapabilityDisabled)
}

#[pg_extern(
    name = "integration_capabilities",
    requires = ["pg_mdm_foundation"],
    sql = "CREATE FUNCTION mdm_internal.integration_capabilities() RETURNS TABLE (capability text, major_version smallint, minor_version smallint, enabled boolean, details jsonb) STRICT LANGUAGE c AS 'MODULE_PATHNAME', 'capability_report_sql_wrapper';"
)]
pub(crate) fn capability_report_sql() -> TableIterator<
    'static,
    (
        name!(capability, String),
        name!(major_version, i16),
        name!(minor_version, i16),
        name!(enabled, bool),
        name!(details, JsonB),
    ),
> {
    let capabilities = match integration_capabilities() {
        Ok(capabilities) => capabilities,
        Err(error) => crate::raise(error),
    };
    let graph = capabilities.external_graph_refresh;
    let mut rows = vec![(
        GRAPH_CAPABILITY.to_string(),
        graph.major,
        graph.minor,
        graph.enabled,
        JsonB(graph.details),
    )];
    if let Some(delta) = capabilities.output_delta_consumer {
        rows.push((
            DELTA_CAPABILITY.to_string(),
            delta.major,
            delta.minor,
            delta.enabled,
            JsonB(delta.details),
        ));
    }
    TableIterator::new(rows)
}

#[pg_extern(
    name = "require_graph_v1",
    requires = [capability_report_sql],
    sql = "CREATE FUNCTION mdm_internal.require_graph_v1() RETURNS jsonb STRICT LANGUAGE c AS 'MODULE_PATHNAME', 'require_graph_v1_sql_wrapper';"
)]
pub(crate) fn require_graph_v1_sql() -> JsonB {
    match require_graph_v1() {
        Ok(capability) => {
            JsonB(serde_json::to_value(capability).unwrap_or_else(|error| {
                crate::raise(MdmError::CapabilityInvalid(error.to_string()))
            }))
        }
        Err(error) => crate::raise(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, major: i16, enabled: bool, details: &str) -> RawCapability {
        RawCapability {
            capability: name.into(),
            major,
            minor: 0,
            enabled,
            details: details.into(),
        }
    }

    #[test]
    fn parses_disabled_baseline() {
        let parsed = parse_capabilities([
            row(GRAPH_CAPABILITY, 1, false, r#"{"status":"experimental"}"#),
            row(DELTA_CAPABILITY, 1, false, r#"{"status":"experimental"}"#),
        ])
        .unwrap();
        assert!(!parsed.external_graph_refresh.enabled);
        assert!(!parsed.output_delta_consumer.unwrap().enabled);
    }

    #[test]
    fn accepts_missing_delta() {
        let parsed = parse_capabilities([row(GRAPH_CAPABILITY, 1, false, "{}")]).unwrap();
        assert!(parsed.output_delta_consumer.is_none());
    }

    #[test]
    fn rejects_missing_graph() {
        assert_eq!(
            parse_capabilities([row(DELTA_CAPABILITY, 1, false, "{}")]),
            Err(MdmError::CapabilityMissing(GRAPH_CAPABILITY))
        );
    }

    #[test]
    fn rejects_duplicate_graph() {
        let error = parse_capabilities([
            row(GRAPH_CAPABILITY, 1, false, "{}"),
            row(GRAPH_CAPABILITY, 1, false, "{}"),
        ])
        .unwrap_err();
        assert_eq!(error.code(), "MDM_PGT_CAPABILITY_INVALID");
    }

    #[test]
    fn rejects_malformed_details() {
        let error = parse_capabilities([row(GRAPH_CAPABILITY, 1, false, "[]")]).unwrap_err();
        assert_eq!(error.code(), "MDM_PGT_CAPABILITY_INVALID");
    }

    #[test]
    fn rejects_unknown_graph_major() {
        assert_eq!(
            parse_capabilities([row(GRAPH_CAPABILITY, 2, true, "{}")]),
            Err(MdmError::CapabilityVersion {
                capability: GRAPH_CAPABILITY.into(),
                major: 2,
            })
        );
    }

    #[test]
    fn disabled_graph_has_a_distinct_gate_error() {
        let capability = parse_capabilities([row(GRAPH_CAPABILITY, 1, false, "{}")]).unwrap();
        assert!(!capability.external_graph_refresh.enabled);
    }
}
