use pgrx::prelude::*;
use pgrx::{Internal, JsonB};
use serde_json::json;

use crate::catalog;
use crate::error::MdmError;
use crate::source_record::quote_identifier;

struct DropRequest {
    entity_name: String,
    confirm: String,
}

#[pg_extern(
    name = "drop_entity",
    requires = [persist_drop_entity],
    sql = "CREATE FUNCTION mdm_admin.drop_entity(entity_name text, confirm text) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'drop_entity_wrapper';"
)]
pub(crate) fn drop_entity(entity_name: String, confirm: String) -> JsonB {
    catalog::call_helper(
        "persist_drop_entity",
        DropRequest {
            entity_name,
            confirm,
        },
    )
    .unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "persist_drop_entity",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_drop_entity(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_drop_entity_wrapper';"
)]
pub(crate) fn persist_drop_entity(request: Internal) -> JsonB {
    let result = (|| {
        let helper_owner = catalog::validate_helper_owner()?;
        let (_, selected) = catalog::validate_caller(&helper_owner)?;
        // SAFETY: only drop_entity constructs this request; SQL can provide only NULL.
        let request = unsafe { request.get::<DropRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("drop request is required".into()))?;
        if request.entity_name != request.confirm {
            return Err(MdmError::GraphLifecycle(
                "drop confirmation must equal the entity name".into(),
            ));
        }

        Spi::connect_mut(|client| {
            let entity = client
                .select(
                    "SELECT e.entity_id::text, e.execution_role_name, b.role_oid FROM mdm_internal.entities e LEFT JOIN mdm_internal.execution_role_bindings b ON b.entity_id = e.entity_id WHERE e.entity_name = $1::pg_catalog.name FOR UPDATE OF e",
                    Some(1),
                    &[request.entity_name.clone().into()],
                )
                .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?;
            if entity.is_empty() {
                return Err(MdmError::GraphLifecycle(format!(
                    "entity {} does not exist",
                    request.entity_name
                )));
            }
            let row = entity.first();
            let entity_id = row
                .get::<String>(1)
                .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?
                .ok_or_else(|| MdmError::GraphLifecycle("entity ID is NULL".into()))?;
            let role_name = row
                .get::<String>(2)
                .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?
                .ok_or_else(|| MdmError::GraphLifecycle("execution role is NULL".into()))?;
            let role_oid = row
                .get::<pg_sys::Oid>(3)
                .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?
                .ok_or_else(|| {
                    MdmError::GraphLifecycle("execution role binding is missing".into())
                })?;
            if role_name != selected.name || role_oid != selected.oid {
                return Err(MdmError::Unauthorized(
                    "entity is bound to another execution role".into(),
                ));
            }

            let outputs = client
                .select(
                    "SELECT output_name::text FROM mdm_internal.output_names WHERE entity_id = $1::pg_catalog.uuid",
                    None,
                    &[entity_id.clone().into()],
                )
                .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?;
            let output_names = outputs
                .into_iter()
                .map(|row| {
                    row.get::<String>(1)
                        .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?
                        .ok_or_else(|| MdmError::GraphLifecycle("output name is NULL".into()))
                })
                .collect::<Result<Vec<_>, _>>()?;

            let members = client
                .select(
                    "SELECT m.relation_name, m.relation_oid FROM mdm_internal.graph_members m JOIN mdm_internal.graph_bindings b ON b.graph_binding_id = m.graph_binding_id WHERE b.entity_id = $1::pg_catalog.uuid ORDER BY b.graph_generation DESC, m.topological_ordinal DESC",
                    None,
                    &[entity_id.clone().into()],
                )
                .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?;
            let mut dropped_members = 0_i64;
            for member in members {
                let relation_name = member
                    .get::<String>(1)
                    .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?
                    .ok_or_else(|| {
                        MdmError::GraphLifecycle("member relation name is NULL".into())
                    })?;
                let relation_oid = member
                    .get::<pg_sys::Oid>(2)
                    .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?
                    .ok_or_else(|| {
                        MdmError::GraphLifecycle("member relation OID is NULL".into())
                    })?;
                let current_oid = client
                    .select(
                        "SELECT pg_catalog.to_regclass($1)::pg_catalog.oid",
                        Some(1),
                        &[relation_name.clone().into()],
                    )
                    .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?
                    .first()
                    .get::<pg_sys::Oid>(1)
                    .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?;
                let owner = client
                    .select(
                        "SELECT pg_catalog.pg_get_userbyid(c.relowner)::text FROM pg_catalog.pg_class c WHERE c.oid = $1",
                        Some(1),
                        &[relation_oid.into()],
                    )
                    .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?
                    .first()
                    .get::<String>(1)
                    .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?;
                if current_oid != Some(relation_oid)
                    || owner.as_deref() != Some(selected.name.as_str())
                {
                    return Err(MdmError::GraphLifecycle(format!(
                        "member {relation_name} no longer matches its binding"
                    )));
                }
                client
                    .update(
                        "SELECT pgtrickle.drop_stream_table($1::text, false)",
                        Some(1),
                        &[relation_name.into()],
                    )
                    .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?;
                dropped_members += 1;
            }
            for output_name in output_names {
                client
                    .update(
                        &format!(
                            "DROP TABLE IF EXISTS mdm_out.{} CASCADE",
                            quote_identifier(&output_name)
                        ),
                        None,
                        &[],
                    )
                    .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?;
            }

            for sql in [
                "DELETE FROM mdm_graph.source_records WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_graph.source_identity_map WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_graph.definition_limits WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.graph_members WHERE graph_binding_id IN (SELECT graph_binding_id FROM mdm_internal.graph_bindings WHERE entity_id = $1::pg_catalog.uuid)",
                "DELETE FROM mdm_internal.graph_bindings WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.golden_provenance WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.publication_observations WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.publications WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.memberships WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.identity_aliases WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.identity_splits WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.identity_registry WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.resolution_facts WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.golden_override_directives WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.steward_decisions WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.source_records WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.source_bindings WHERE source_identity_id IN (SELECT source_identity_id FROM mdm_internal.source_identities WHERE entity_id = $1::pg_catalog.uuid)",
                "DELETE FROM mdm_internal.source_identities WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.definition_artifacts WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.definitions WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.output_names WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.execution_role_bindings WHERE entity_id = $1::pg_catalog.uuid",
                "DELETE FROM mdm_internal.operations WHERE entity_name = (SELECT entity_name FROM mdm_internal.entities WHERE entity_id = $1::pg_catalog.uuid)",
                "DELETE FROM mdm_internal.entities WHERE entity_id = $1::pg_catalog.uuid",
            ] {
                client
                    .update(sql, None, &[entity_id.clone().into()])
                    .map_err(|error| MdmError::GraphLifecycle(error.to_string()))?;
            }
            Ok(JsonB(json!({
                "entity_name": request.entity_name,
                "dropped_members": dropped_members
            })))
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
