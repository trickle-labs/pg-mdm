use serde_json::{Value, json};

use crate::definition::{Entity, Field, MatchRule, Source};

pub(crate) const COMPILER_VERSION: i32 = 1;
pub(crate) const ARTIFACT_FORMAT_VERSION: i32 = 1;

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

fn source_node(source: &Source) -> Value {
    node(
        format!("records/{}", source.name),
        Vec::new(),
        format!("SELECT * FROM {}", source.relation),
        json!({"source_name":"text","source_id":"jsonb"}),
    )
}

fn normalized_node(field: &Field, sources: &[Source]) -> Value {
    node(
        format!("normalized/{}", field.name),
        sources
            .iter()
            .map(|source| format!("records/{}", source.name))
            .collect(),
        format!("SELECT normalized FROM pg_mdm_normalize_{}", field.cleaner),
        json!({"source_name":"text","field_name":"text","normalized":"text"}),
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

pub(crate) fn compile(entity: &Entity) -> Value {
    let mut nodes = Vec::new();
    for source in &entity.sources {
        nodes.push(source_node(source));
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
        assert!(graph["nodes"].as_array().unwrap().iter().all(|node| {
            node["initialize"] == false && node["orchestration_mode"] == "EXTERNAL"
        }));
    }
}
