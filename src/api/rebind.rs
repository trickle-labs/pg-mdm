use pgrx::prelude::*;
use pgrx::{Internal, JsonB};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog;
use crate::definition::source::{ValidatedSource, validate_source};
use crate::definition::{Source, parse_entity};
use crate::error::MdmError;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct SourceIdentity {
    source_identity_id: String,
    source_name: String,
    relation_name: String,
    key_contract: Value,
    identity_digest: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Snapshot {
    entity_id: String,
    entity_name: String,
    desired_version: i64,
    execution_role_name: String,
    expanded_definition: Value,
    sources: Vec<SourceIdentity>,
}

struct RebindRequest {
    snapshot: Snapshot,
    sources: Vec<ValidatedSource>,
}

fn selected_admin_role() -> Result<String, MdmError> {
    // SAFETY: PostgreSQL calls extension functions on its backend thread.
    if !unsafe {
        pg_sys::superuser_arg(pg_sys::GetAuthenticatedUserId())
            && pg_sys::superuser_arg(catalog::session_user_id())
    } {
        return Err(MdmError::Unauthorized(
            "rebind requires an authenticated superuser session with SET ROLE to the execution role"
                .into(),
        ));
    }
    let helper_owner = catalog::validate_helper_owner()?;
    let selected = catalog::outer_user_id();
    if selected == helper_owner.oid {
        return Err(MdmError::Unauthorized(
            "the helper owner cannot be the execution role".into(),
        ));
    }
    Spi::get_one_with_args::<String>(
        "SELECT r.rolname::text FROM pg_catalog.pg_roles r WHERE r.oid = $1 AND NOT r.rolsuper AND NOT r.rolbypassrls AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension e WHERE e.extname = 'pg_mdm' AND e.extowner = r.oid)",
        &[selected.into()],
    )
    .map_err(|error| MdmError::Spi(error.to_string()))?
    .ok_or_else(|| MdmError::Unauthorized("SET ROLE to the entity's safe execution role before rebinding".into()))
}

fn snapshot(
    client: &pgrx::spi::SpiClient<'_>,
    entity_name: &str,
    lock: bool,
) -> Result<Snapshot, MdmError> {
    let query = if lock {
        "SELECT jsonb_build_object('entity_id', e.entity_id::text, 'entity_name', e.entity_name::text, 'desired_version', e.desired_version, 'execution_role_name', e.execution_role_name, 'expanded_definition', d.expanded_definition, 'sources', '[]'::jsonb) FROM mdm_internal.entities e JOIN mdm_internal.definitions d ON d.entity_id = e.entity_id AND d.definition_version = e.desired_version WHERE e.entity_name::text = $1 FOR UPDATE OF e"
    } else {
        "SELECT jsonb_build_object('entity_id', e.entity_id::text, 'entity_name', e.entity_name::text, 'desired_version', e.desired_version, 'execution_role_name', e.execution_role_name, 'expanded_definition', d.expanded_definition, 'sources', '[]'::jsonb) FROM mdm_internal.entities e JOIN mdm_internal.definitions d ON d.entity_id = e.entity_id AND d.definition_version = e.desired_version WHERE e.entity_name::text = $1"
    };
    let table = client
        .select(query, Some(1), &[entity_name.into()])
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    if table.is_empty() {
        return Err(MdmError::DefinitionInvalid(format!(
            "entity {entity_name} does not exist"
        )));
    }
    let value = table
        .first()
        .get::<JsonB>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| {
            MdmError::DefinitionInvalid(format!("entity {entity_name} does not exist"))
        })?;
    let mut snapshot: Snapshot = serde_json::from_value(value.0)
        .map_err(|error| MdmError::OperationState(error.to_string()))?;
    let query = if lock {
        "SELECT COALESCE(jsonb_agg(jsonb_build_object('source_identity_id', s.source_identity_id::text, 'source_name', s.source_name::text, 'relation_name', s.relation_name, 'key_contract', s.key_contract, 'identity_digest', encode(s.identity_digest, 'hex')) ORDER BY s.source_name), '[]'::jsonb) FROM (SELECT * FROM mdm_internal.source_identities WHERE entity_id = $1::pg_catalog.uuid FOR UPDATE) s"
    } else {
        "SELECT COALESCE(jsonb_agg(jsonb_build_object('source_identity_id', s.source_identity_id::text, 'source_name', s.source_name::text, 'relation_name', s.relation_name, 'key_contract', s.key_contract, 'identity_digest', encode(s.identity_digest, 'hex')) ORDER BY s.source_name), '[]'::jsonb) FROM (SELECT * FROM mdm_internal.source_identities WHERE entity_id = $1::pg_catalog.uuid) s"
    };
    let table = client
        .select(query, Some(1), &[snapshot.entity_id.clone().into()])
        .map_err(|error| MdmError::Spi(error.to_string()))?;
    let sources = table
        .first()
        .get::<JsonB>(1)
        .map_err(|error| MdmError::Spi(error.to_string()))?
        .ok_or_else(|| MdmError::OperationState("source identities are NULL".into()))?;
    snapshot.sources = serde_json::from_value(sources.0)
        .map_err(|error| MdmError::OperationState(error.to_string()))?;
    Ok(snapshot)
}

#[pg_extern(
    name = "rebind",
    requires = [prepare_rebind, persist_rebind],
    sql = "CREATE FUNCTION mdm_admin.rebind(entity_name text) RETURNS jsonb LANGUAGE c AS 'MODULE_PATHNAME', 'rebind_wrapper';"
)]
pub(crate) fn rebind(entity_name: String) -> JsonB {
    let result = (|| {
        let value = catalog::call_helper("prepare_rebind", entity_name)?;
        let snapshot: Snapshot = serde_json::from_value(value.0)
            .map_err(|error| MdmError::OperationState(error.to_string()))?;
        let entity = parse_entity(snapshot.expanded_definition.clone())
            .map_err(MdmError::DefinitionInvalid)?;
        // All source validation runs with the selected execution role's privileges and RLS.
        for source in &entity.sources {
            validate_source(source, &entity)?;
        }
        let mut sources = Vec::with_capacity(snapshot.sources.len());
        for identity in &snapshot.sources {
            // Historical sources retain their frozen key contract even when the desired
            // definition no longer maps their fields.
            let source = Source {
                name: identity.source_name.clone(),
                relation: identity.relation_name.clone(),
                source_id: serde_json::from_value(identity.key_contract["key_columns"].clone())
                    .map_err(|error| MdmError::SourceInvalid(error.to_string()))?,
                mode: "tracked".into(),
                fields: Default::default(),
                row_changed_at: None,
                soft_delete_when: None,
                authority: Default::default(),
            };
            let validated = validate_source(&source, &entity)?;
            if validated.key_contract != identity.key_contract
                || crate::definition::validate::digest_hex(&validated.identity_digest)
                    != identity.identity_digest
            {
                return Err(MdmError::SourceInvalid(format!(
                    "source identity {} changed; rebinding cannot change its frozen contract",
                    identity.source_name
                )));
            }
            sources.push(validated);
        }
        catalog::call_helper("persist_rebind", RebindRequest { snapshot, sources })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "prepare_rebind",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.prepare_rebind(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'prepare_rebind_wrapper';"
)]
pub(crate) fn prepare_rebind(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: call_helper passes a String through PostgreSQL's SQL-inexpressible
        // internal type. SQL callers can supply only NULL, which is rejected here.
        let entity_name = unsafe { request.get::<String>() }
            .ok_or_else(|| MdmError::Unauthorized("rebind request is NULL".into()))?;
        let selected = selected_admin_role()?;
        let snapshot = Spi::connect(|client| snapshot(client, entity_name, false))?;
        if snapshot.execution_role_name != selected {
            return Err(MdmError::Unauthorized(
                "SET ROLE to the entity's stored execution role before rebinding".into(),
            ));
        }
        Ok(JsonB(
            serde_json::to_value(snapshot).expect("snapshot is serializable"),
        ))
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}

#[pg_extern(
    name = "persist_rebind",
    security_definer,
    sql = "CREATE FUNCTION mdm_internal.persist_rebind(request internal) RETURNS jsonb SECURITY DEFINER SET search_path TO pg_catalog, mdm_internal, pg_temp LANGUAGE c AS 'MODULE_PATHNAME', 'persist_rebind_wrapper';"
)]
pub(crate) fn persist_rebind(request: Internal) -> JsonB {
    let result = (|| {
        // SAFETY: only rebind passes a RebindRequest through call_helper. SQL cannot
        // construct a non-NULL internal argument.
        let request = unsafe { request.get::<RebindRequest>() }
            .ok_or_else(|| MdmError::Unauthorized("rebind request is NULL".into()))?;
        let selected = selected_admin_role()?;
        if request.snapshot.execution_role_name != selected {
            return Err(MdmError::Unauthorized(
                "execution role changed during rebinding".into(),
            ));
        }
        Spi::connect_mut(|client| {
            let current = snapshot(client, &request.snapshot.entity_name, true)?;
            if current != request.snapshot {
                return Err(MdmError::VersionConflict(
                    "entity definition or source identities changed during rebinding".into(),
                ));
            }
            client.update(
                "INSERT INTO mdm_internal.execution_role_bindings (entity_id, role_oid) VALUES ($1::pg_catalog.uuid, $2) ON CONFLICT (entity_id) DO UPDATE SET role_oid = EXCLUDED.role_oid, bound_at = pg_catalog.statement_timestamp()",
                None,
                &[current.entity_id.clone().into(), catalog::outer_user_id().into()],
            ).map_err(|error| MdmError::Spi(error.to_string()))?;
            for (identity, source) in current.sources.iter().zip(&request.sources) {
                client.update(
                    "INSERT INTO mdm_internal.source_bindings (source_identity_id, relation_oid, binding_fingerprint) VALUES ($1::pg_catalog.uuid, $2, $3) ON CONFLICT (source_identity_id) DO UPDATE SET relation_oid = EXCLUDED.relation_oid, binding_fingerprint = EXCLUDED.binding_fingerprint, bound_at = pg_catalog.statement_timestamp()",
                    None,
                    &[identity.source_identity_id.clone().into(), source.relation_oid.into(), JsonB(source.binding_fingerprint.clone()).into()],
                ).map_err(|error| MdmError::Spi(error.to_string()))?;
            }
            let outcome = JsonB(json!({
                "entity_name": current.entity_name,
                "desired_version": current.desired_version,
                "rebound_sources": request.sources.len()
            }));
            client.update(
                "INSERT INTO mdm_internal.operations (operation_kind, entity_name, status, result_code, outcome, actor_name, actor_role_name, completed_at) VALUES ('rebind', $1::pg_catalog.name, 'succeeded', 'MDM_OK', $2, SESSION_USER::text, $3, pg_catalog.statement_timestamp())",
                None,
                &[current.entity_name.into(), JsonB(outcome.0.clone()).into(), selected.into()],
            ).map_err(|error| MdmError::Spi(error.to_string()))?;
            Ok(outcome)
        })
    })();
    result.unwrap_or_else(|error| crate::raise(error))
}
