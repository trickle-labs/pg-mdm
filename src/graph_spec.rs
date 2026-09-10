use serde_json::{Value, json};

use crate::definition::{Entity, Field, MatchRule, Source};
use crate::source_record::quote_identifier;

pub const COMPILER_VERSION: i32 = 2;
pub const ARTIFACT_FORMAT_VERSION: i32 = 1;

fn node(id: String, dependencies: Vec<String>, sql: String, schema: Value) -> Value {
    json!({
        "logical_id": id,
        "dependencies": dependencies,
        "output_schema": schema,
        "defining_sql": sql,
        "initialize": false,
        "orchestration_mode": "EXTERNAL",
        "executable": false
    })
}

pub fn source_record_sql(entity_name: &str, source: &Source) -> String {
    let mut key_cols = Vec::new();
    for col in &source.source_id {
        key_cols.push(quote_identifier(col));
    }

    let entity_name_escaped = entity_name.replace('\'', "''");
    let source_name_escaped = source.name.replace('\'', "''");

    let mut select_items = vec![
        format!("'{}'::text AS source_name", source_name_escaped),
        format!(
            "pgtrickle.encode_row_id_v2('MDM_SOURCE_KEY_V1', ROW((SELECT entity_id FROM mdm_internal.entities WHERE entity_name = '{entity_name_escaped}'), (SELECT source_identity_id FROM mdm_internal.source_identities WHERE entity_id = (SELECT entity_id FROM mdm_internal.entities WHERE entity_name = '{entity_name_escaped}') AND source_name = '{source_name_escaped}'), {})) AS source_record_key",
            key_cols.join(", ")
        ),
    ];

    for (field_name, mapping) in &source.fields {
        let (val_col, state_col) = match mapping {
            Value::String(col) => (col.as_str(), None),
            Value::Object(obj) => {
                let v = obj.get("value").and_then(Value::as_str).unwrap_or("");
                let s = obj.get("state").and_then(Value::as_str);
                (v, s)
            }
            _ => continue,
        };

        if !val_col.is_empty() {
            select_items.push(format!(
                "{} AS {}",
                quote_identifier(val_col),
                quote_identifier(field_name)
            ));
            let state_expr = match state_col {
                Some(s) if !s.is_empty() => quote_identifier(s),
                _ => "'present'::text".into(),
            };
            select_items.push(format!(
                "{state_expr} AS {}",
                quote_identifier(&format!("{field_name}_state"))
            ));
        }
    }

    if let Some(col) = &source.row_changed_at {
        select_items.push(format!("{} AS row_changed_at", quote_identifier(col)));
    }

    let mut sql = format!(
        "SELECT {}\nFROM {}",
        select_items.join(", "),
        source.relation
    );

    if source.mode == "soft_delete"
        && let Some(predicate) = &source.soft_delete_when
        && let Some(col) = predicate.get("column").and_then(Value::as_str)
        && let Some(kind) = predicate.get("kind").and_then(Value::as_str)
    {
        let quoted = quote_identifier(col);
        let cond = match kind {
            "is_true" => format!("{quoted} IS NOT TRUE"),
            "is_not_null" => format!("{quoted} IS NULL"),
            _ => "true".into(),
        };
        sql.push_str(&format!("\nWHERE {cond}"));
    }

    sql
}

fn source_node(entity_name: &str, source: &Source) -> Value {
    let mut schema = json!({
        "source_name": "text",
        "source_record_key": "bytea"
    });
    let schema_obj = schema.as_object_mut().expect("schema is object");
    for field_name in source.fields.keys() {
        schema_obj.insert(field_name.clone(), json!("text"));
        schema_obj.insert(format!("{field_name}_state"), json!("text"));
    }
    if source.row_changed_at.is_some() {
        schema_obj.insert("row_changed_at".into(), json!("timestamptz"));
    }

    let sql = source_record_sql(entity_name, source);

    node(format!("records/{}", source.name), Vec::new(), sql, schema)
}

pub fn normalized_field_sql(field: &Field, sources: &[Source]) -> String {
    let options_val = serde_json::to_value(&field.cleaner_options).unwrap_or_else(|_| json!({}));
    let options_str = serde_json::to_string(&options_val).unwrap_or_else(|_| "{}".into());
    let options_escaped = options_str.replace('\'', "''");

    let cleaner_escaped = field.cleaner.replace('\'', "''");
    let field_escaped = field.name.replace('\'', "''");

    let mut parts = Vec::new();

    for source in sources {
        if !source.fields.contains_key(&field.name) {
            continue;
        }

        let normalize_call = if field.cleaner == "date" || field.logical_type == "date" {
            format!(
                "mdm_internal.normalize_date({}, '{cleaner_escaped}', 1, {}, '{options_escaped}'::jsonb)",
                quote_identifier(&field.name),
                quote_identifier(&format!("{}_state", field.name))
            )
        } else {
            format!(
                "mdm_internal.normalize_text({}, '{cleaner_escaped}', 1, {}, '{options_escaped}'::jsonb)",
                quote_identifier(&field.name),
                quote_identifier(&format!("{}_state", field.name))
            )
        };

        let source_escaped = source.name.replace('\'', "''");
        let part_sql = format!(
            "SELECT '{source_escaped}'::text AS source_name, source_record_key, '{field_escaped}'::text AS field_name, (n).state, (n).normalized, (n).canonical_bytes\nFROM (SELECT source_record_key, {normalize_call} AS n FROM records_{}) s",
            source.name
        );
        parts.push(part_sql);
    }

    if parts.is_empty() {
        format!(
            "SELECT NULL::text AS source_name, NULL::bytea AS source_record_key, '{field_escaped}'::text AS field_name, NULL::text AS state, NULL::text AS normalized, NULL::bytea AS canonical_bytes WHERE false"
        )
    } else {
        parts.join("\nUNION ALL\n")
    }
}

fn normalized_node(field: &Field, sources: &[Source]) -> Value {
    let deps: Vec<String> = sources
        .iter()
        .filter(|source| source.fields.contains_key(&field.name))
        .map(|source| format!("records/{}", source.name))
        .collect();

    let sql = normalized_field_sql(field, sources);

    node(
        format!("normalized/{}", field.name),
        deps,
        sql,
        json!({
            "source_name": "text",
            "source_record_key": "bytea",
            "field_name": "text",
            "state": "text",
            "normalized": "text",
            "canonical_bytes": "bytea"
        }),
    )
}

fn match_node(rule: &MatchRule, fields: &[Field]) -> Value {
    node(
        format!("blocks/{}", rule.name),
        rule.fields
            .iter()
            .filter(|name| fields.iter().any(|field| &field.name == *name))
            .map(|name| format!("normalized/{name}"))
            .collect(),
        format!("SELECT * FROM candidate_channel_{}", rule.name),
        json!({"source_name":"text","left_source_id":"jsonb","right_source_id":"jsonb"}),
    )
}

pub fn compile(entity: &Entity) -> Value {
    let mut nodes = Vec::new();
    for source in &entity.sources {
        nodes.push(source_node(&entity.name, source));
    }
    for field in &entity.fields {
        nodes.push(normalized_node(field, &entity.sources));
    }
    for rule in &entity.matches {
        if rule.candidate.is_some() {
            nodes.push(match_node(rule, &entity.fields));
        }
    }
    let blocks: Vec<String> = entity
        .matches
        .iter()
        .filter(|rule| rule.candidate.is_some())
        .map(|rule| format!("blocks/{}", rule.name))
        .collect();
    nodes.push(node(
        format!("pairs/{}", entity.name),
        blocks,
        "SELECT DISTINCT left_source_id, right_source_id FROM candidate_blocks".into(),
        json!({"left_source_id":"jsonb","right_source_id":"jsonb"}),
    ));
    nodes.push(node(
        format!("evidence/{}", entity.name),
        vec![format!("pairs/{}", entity.name)],
        "SELECT * FROM pair_evidence".into(),
        json!({"left_source_id":"jsonb","right_source_id":"jsonb","decision":"text"}),
    ));
    nodes.push(node(
        format!("golden/{}", entity.name),
        vec![format!("evidence/{}", entity.name)],
        "SELECT * FROM golden_candidates".into(),
        json!({"mdm_id":"uuid"}),
    ));
    json!({
        "format_version": ARTIFACT_FORMAT_VERSION,
        "compiler_version": COMPILER_VERSION,
        "executable": false,
        "nodes": nodes
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::parse_entity;

    #[test]
    fn development_graph_is_external_and_not_initialized() {
        let entity = parse_entity(json!({
            "name":"customer",
            "sources":[],"fields":[],"matches":[],"golden_values":[],
            "preset":null,"limits":{},"execution_role":null
        }))
        .expect("test entity parses");
        let graph = compile(&entity);
        assert_eq!(graph["executable"], false);
        assert_eq!(graph["compiler_version"], 2);
        assert!(graph["nodes"].as_array().unwrap().iter().all(|node| {
            node["initialize"] == false && node["orchestration_mode"] == "EXTERNAL"
        }));
    }

    #[test]
    fn record_and_normalized_sql_generation() {
        let entity = parse_entity(json!({
            "name": "customer",
            "sources": [{
                "name": "crm",
                "relation": "public.crm_customer",
                "source_id": ["id"],
                "mode": "tracked",
                "fields": {"email": "email_addr"},
                "row_changed_at": null,
                "soft_delete_when": null,
                "authority": {}
            }],
            "fields": [{
                "name": "email",
                "type": "text",
                "cleaner": "email",
                "cleaner_options": {},
                "display": "masked"
            }],
            "matches": [],
            "golden_values": [],
            "preset": null,
            "limits": {},
            "execution_role": null
        }))
        .expect("parses");

        let graph = compile(&entity);
        let nodes = graph["nodes"].as_array().unwrap();
        let record_node = nodes
            .iter()
            .find(|n| n["logical_id"] == "records/crm")
            .unwrap();
        assert!(
            record_node["defining_sql"]
                .as_str()
                .unwrap()
                .contains("encode_row_id_v2")
        );
        assert!(
            record_node["defining_sql"]
                .as_str()
                .unwrap()
                .contains("FROM public.crm_customer")
        );

        let norm_node = nodes
            .iter()
            .find(|n| n["logical_id"] == "normalized/email")
            .unwrap();
        assert!(
            norm_node["defining_sql"]
                .as_str()
                .unwrap()
                .contains("mdm_internal.normalize_text")
        );
        assert!(
            norm_node["defining_sql"]
                .as_str()
                .unwrap()
                .contains("FROM records_crm")
        );
    }
}
