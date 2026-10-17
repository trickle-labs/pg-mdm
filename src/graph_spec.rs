use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::candidate::{CandidateChannel, CandidatePlan, ChannelKind};
use crate::definition::{Entity, Field, MatchRule, Source};
use crate::error::MdmError;
use crate::semantics;
use crate::source_record::quote_identifier;

pub const COMPILER_VERSION: i32 = 11;
pub const ARTIFACT_FORMAT_VERSION: i32 = 1;

fn node_with_refresh_mode(
    id: String,
    dependencies: Vec<String>,
    sql: String,
    schema: Value,
    refresh_mode: &str,
) -> Value {
    json!({
        "logical_id": id,
        "dependencies": dependencies,
        "output_schema": schema,
        "defining_sql": sql,
        "refresh_mode": refresh_mode,
        "initialize": false,
        "orchestration_mode": "EXTERNAL",
        "executable": true
    })
}

fn node_ref(logical_id: &str) -> String {
    format!("@{{{logical_id}}}")
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GraphNode {
    pub logical_id: String,
    pub dependencies: Vec<String>,
    pub output_schema: Value,
    pub defining_sql: String,
    pub refresh_mode: String,
    pub initialize: bool,
    pub orchestration_mode: String,
    pub executable: bool,
}

pub fn artifact_nodes(artifact: &[u8]) -> Result<(Vec<GraphNode>, Vec<String>), MdmError> {
    let value: Value = serde_json::from_slice(artifact)
        .map_err(|error| MdmError::GraphArtifact(format!("artifact JSON is invalid: {error}")))?;
    if value["format_version"] != ARTIFACT_FORMAT_VERSION
        || value["compiler_version"] != COMPILER_VERSION
        || value["executable"] != true
    {
        return Err(MdmError::GraphArtifact(
            "artifact format, compiler, or executable flag is unsupported".into(),
        ));
    }
    let nodes: Vec<GraphNode> = serde_json::from_value(value["nodes"].clone())
        .map_err(|error| MdmError::GraphArtifact(format!("nodes are invalid: {error}")))?;
    let roots: Vec<String> = serde_json::from_value(value["roots"].clone())
        .map_err(|error| MdmError::GraphArtifact(format!("roots are invalid: {error}")))?;
    if nodes.is_empty() || roots.is_empty() {
        return Err(MdmError::GraphArtifact(
            "artifact must contain nodes and roots".into(),
        ));
    }

    let mut by_id = BTreeMap::new();
    for node in &nodes {
        if !node.executable
            || node.initialize
            || node.orchestration_mode != "EXTERNAL"
            || !matches!(node.refresh_mode.as_str(), "AUTO" | "FULL" | "DIFFERENTIAL")
        {
            return Err(MdmError::GraphArtifact(format!(
                "node {} is not an executable external node",
                node.logical_id
            )));
        }
        if by_id.insert(node.logical_id.clone(), node).is_some() {
            return Err(MdmError::GraphArtifact(format!(
                "duplicate logical node {}",
                node.logical_id
            )));
        }
    }
    for root in &roots {
        if !by_id.contains_key(root) {
            return Err(MdmError::GraphArtifact(format!(
                "missing graph root {root}"
            )));
        }
    }

    let mut indegree = BTreeMap::<String, usize>::new();
    let mut dependents = BTreeMap::<String, Vec<String>>::new();
    for node in &nodes {
        indegree.insert(node.logical_id.clone(), node.dependencies.len());
        for dependency in &node.dependencies {
            if !by_id.contains_key(dependency) {
                return Err(MdmError::GraphArtifact(format!(
                    "node {} depends on missing node {dependency}",
                    node.logical_id
                )));
            }
            dependents
                .entry(dependency.clone())
                .or_default()
                .push(node.logical_id.clone());
        }
    }
    let mut ready: BTreeSet<String> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut ordered = Vec::with_capacity(nodes.len());
    while let Some(id) = ready.pop_first() {
        ordered.push(by_id[&id].clone());
        for dependent in dependents.get(&id).into_iter().flatten() {
            let degree = indegree
                .get_mut(dependent)
                .expect("dependent was inserted with an indegree");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(dependent.clone());
            }
        }
    }
    if ordered.len() != nodes.len() {
        return Err(MdmError::GraphArtifact(
            "graph dependencies contain a cycle".into(),
        ));
    }
    Ok((ordered, roots))
}

pub fn render_sql(sql: &str, relations: &BTreeMap<String, String>) -> Result<String, MdmError> {
    let mut rendered = sql.to_string();
    for (logical_id, relation) in relations {
        rendered = rendered.replace(&node_ref(logical_id), relation);
    }
    if rendered.contains("@{") {
        return Err(MdmError::GraphArtifact(
            "defining SQL contains an unresolved logical relation".into(),
        ));
    }
    Ok(rendered)
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
            "pgtrickle.encode_row_id_v2('SCAN_KEY', ROW((SELECT entity_id FROM mdm_graph.source_identity_map WHERE entity_name = '{entity_name_escaped}' AND source_name = '{source_name_escaped}'), (SELECT source_identity_id FROM mdm_graph.source_identity_map WHERE entity_name = '{entity_name_escaped}' AND source_name = '{source_name_escaped}'), {})) AS source_record_key",
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

    node_with_refresh_mode(
        format!("records/{}", source.name),
        Vec::new(),
        sql,
        schema,
        "DIFFERENTIAL",
    )
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

        let field_value = format!("r.{}", quote_identifier(&field.name));
        let field_state = format!("r.{}", quote_identifier(&format!("{}_state", field.name)));
        let normalize_call = if field.cleaner == "date" || field.logical_type == "date" {
            format!(
                "mdm_graph.normalize_date(({field_value})::date, '{cleaner_escaped}', 1, {field_state}, '{options_escaped}'::jsonb)"
            )
        } else {
            format!(
                "mdm_graph.normalize_text(({field_value})::text, '{cleaner_escaped}', 1, {field_state}, '{options_escaped}'::jsonb)"
            )
        };

        let raw_value = format!("{field_value}::text");
        let row_changed_at: String = source.row_changed_at.as_deref().map_or_else(
            || "NULL::timestamptz".into(),
            |_| "r.row_changed_at::timestamptz".into(),
        );
        let source_escaped = source.name.replace('\'', "''");
        let records_relation = node_ref(&format!("records/{}", source.name));
        let part_sql = format!(
            "SELECT '{source_escaped}'::text AS source_name, r.source_record_key AS source_record_key, sr.source_record_id AS source_record_id, sr.source_record_key AS source_sort_key, '{field_escaped}'::text AS field_name, {raw_value} AS raw_value, {row_changed_at} AS row_changed_at, n.state AS state, n.normalized AS normalized, n.canonical_bytes AS canonical_bytes\nFROM {records_relation} AS r\nCROSS JOIN LATERAL {normalize_call} AS n\nJOIN mdm_graph.source_records sr ON sr.source_record_key = r.source_record_key AND sr.active\nJOIN mdm_graph.source_identity_map si ON si.source_identity_id = sr.source_identity_id AND si.source_name = '{source_escaped}'",
        );
        parts.push(part_sql);
    }

    if parts.is_empty() {
        let fallback = sources
            .first()
            .map(|source| node_ref(&format!("records/{}", source.name)))
            .unwrap_or_else(|| "pg_catalog.pg_class".into());
        format!(
            "SELECT NULL::text AS source_name, NULL::bytea AS source_record_key, NULL::uuid AS source_record_id, NULL::bytea AS source_sort_key, '{field_escaped}'::text AS field_name, NULL::text AS raw_value, NULL::timestamptz AS row_changed_at, NULL::text AS state, NULL::text AS normalized, NULL::bytea AS canonical_bytes FROM {fallback} AS empty WHERE false"
        )
    } else {
        parts.join("\nUNION ALL\n")
    }
}

fn normalized_node(field: &Field, sources: &[Source]) -> Value {
    let mut deps: Vec<String> = sources
        .iter()
        .filter(|source| source.fields.contains_key(&field.name))
        .map(|source| format!("records/{}", source.name))
        .collect();
    if deps.is_empty()
        && let Some(source) = sources.first()
    {
        deps.push(format!("records/{}", source.name));
    }

    let sql = normalized_field_sql(field, sources);

    node_with_refresh_mode(
        format!("normalized/{}", field.name),
        deps,
        sql,
        json!({
            "source_name": "text",
            "source_record_key": "bytea",
            "source_record_id": "uuid",
            "source_sort_key": "bytea",
            "field_name": "text",
            "raw_value": "text",
            "row_changed_at": "timestamptz",
            "state": "text",
            "normalized": "text",
            "canonical_bytes": "bytea"
        }),
        "DIFFERENTIAL",
    )
}

fn limits_for_entity(entity: &Entity) -> crate::candidate::CandidateLimits {
    let mut limits = entity.limits.clone();
    match semantics::expand_limits(&mut limits) {
        Ok(limits) => limits,
        Err(_) => semantics::candidate_limits(),
    }
}

fn normalized_relation(field: &str) -> String {
    node_ref(&format!("normalized/{field}"))
}

fn sql_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn immutable_text_join(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return "''::text".into();
    };
    let mut parts = vec![format!("COALESCE({first}, ''::text)")];
    for (index, value) in values.iter().enumerate().skip(1) {
        let prior_non_null = values[..index]
            .iter()
            .map(|prior| format!("{prior} IS NOT NULL"))
            .collect::<Vec<_>>()
            .join(" OR ");
        parts.push(format!(
            "CASE WHEN {value} IS NULL OR NOT ({prior_non_null}) THEN ''::text ELSE pg_catalog.chr(31) END"
        ));
        parts.push(format!("COALESCE({value}, ''::text)"));
    }
    parts.join(" || ")
}

fn candidate_evidence_column(alias: &str, side: &str, field: &str, attribute: &str) -> String {
    format!(
        "{alias}.{}",
        quote_identifier(&format!("{side}_evidence_{field}_{attribute}"))
    )
}

fn evidence_rule_expressions(
    rule: &MatchRule,
    entity_name: &str,
    alias: &str,
) -> (String, String, String, String, String) {
    evidence_rule_expressions_with_columns(rule, entity_name, |side, field, attribute| {
        candidate_evidence_column(alias, side, field, attribute)
    })
}

fn joined_evidence_rule_expressions(
    rule: &MatchRule,
    entity_name: &str,
    fields: &[String],
) -> (String, String, String, String, String) {
    evidence_rule_expressions_with_columns(rule, entity_name, |side, field, attribute| {
        let index = fields
            .iter()
            .position(|candidate| candidate == field)
            .expect("every rule field has a normalized join");
        format!("evidence_{side}_{index}.{attribute}")
    })
}

fn evidence_rule_expressions_with_columns(
    rule: &MatchRule,
    entity_name: &str,
    column: impl Fn(&str, &str, &str) -> String,
) -> (String, String, String, String, String) {
    let mut usable = Vec::new();
    let mut exact_equal = Vec::new();
    let mut left_values = Vec::new();
    let mut right_values = Vec::new();
    for field in &rule.fields {
        let left_state = column("left", field, "state");
        let left_bytes = column("left", field, "canonical_bytes");
        let left_normalized = column("left", field, "normalized");
        let right_state = column("right", field, "state");
        let right_bytes = column("right", field, "canonical_bytes");
        let right_normalized = column("right", field, "normalized");
        usable.push(format!(
            "{left_state} = 'value' AND {right_state} = 'value' AND {left_bytes} IS NOT NULL AND {right_bytes} IS NOT NULL"
        ));
        exact_equal.push(format!("{left_bytes} = {right_bytes}"));
        left_values.push(left_normalized);
        right_values.push(right_normalized);
    }
    let usable = usable.join(" AND ");
    let exact_equal = exact_equal.join(" AND ");
    let left_text = immutable_text_join(&left_values);
    let right_text = immutable_text_join(&right_values);
    let score = if rule.comparison == "exact" {
        "NULL::smallint".into()
    } else {
        format!(
            "mdm_graph.normalized_levenshtein_score({left_text}, {right_text}, (SELECT COALESCE((d.expanded_definition->'limits'->>'max_comparator_work')::bigint, {}::bigint) FROM mdm_graph.definition_limits d WHERE d.entity_name = {}::text))::smallint",
            crate::comparators::DEFAULT_MAX_COMPARATOR_WORK,
            sql_text(entity_name)
        )
    };
    let class = if rule.comparison == "exact" {
        format!(
            "CASE WHEN {usable} THEN CASE WHEN {exact_equal} THEN 'agree'::text ELSE 'disagree'::text END ELSE 'no_evidence'::text END"
        )
    } else {
        let threshold = rule.threshold.unwrap_or(10_000);
        format!(
            "CASE WHEN {usable} THEN CASE WHEN {score} >= {threshold} THEN 'agree'::text ELSE 'disagree'::text END ELSE 'no_evidence'::text END"
        )
    };
    let score = if rule.comparison == "exact" {
        "NULL::smallint".into()
    } else {
        score
    };
    (usable, class, score, left_text, right_text)
}

fn evidence_value_column(rule_index: usize, attribute: &str) -> String {
    quote_identifier(&format!("evidence_rule_{rule_index}_{attribute}"))
}

fn candidate_evidence_result_columns(rules: &[MatchRule], entity_name: &str) -> Vec<String> {
    candidate_evidence_result_columns_with(rules, |rule| {
        evidence_rule_expressions(rule, entity_name, "candidate")
    })
}

fn joined_evidence_result_columns(
    rules: &[MatchRule],
    entity_name: &str,
    fields: &[String],
) -> Vec<String> {
    candidate_evidence_result_columns_with(rules, |rule| {
        joined_evidence_rule_expressions(rule, entity_name, fields)
    })
}

fn candidate_evidence_result_columns_with(
    rules: &[MatchRule],
    mut expressions: impl FnMut(&MatchRule) -> (String, String, String, String, String),
) -> Vec<String> {
    let mut columns = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        let (usable, class, score, left_text, right_text) = expressions(rule);
        columns.push(format!(
            "{class} AS {}",
            evidence_value_column(index, "class")
        ));
        columns.push(format!(
            "CASE WHEN {usable} THEN {score} ELSE NULL::smallint END AS {}",
            evidence_value_column(index, "score")
        ));
        columns.push(format!(
            "CASE WHEN {usable} THEN mdm_graph.evidence_digest({left_text}) ELSE NULL::bytea END AS {}",
            evidence_value_column(index, "left_value_digest")
        ));
        columns.push(format!(
            "CASE WHEN {usable} THEN mdm_graph.evidence_digest({right_text}) ELSE NULL::bytea END AS {}",
            evidence_value_column(index, "right_value_digest")
        ));
    }
    columns
}

fn evidence_rule_sql(rule: &MatchRule, rule_index: usize, pair_relation: &str) -> String {
    format!(
        "SELECT p.left_source_record_id, p.right_source_record_id, p.left_sort_key, p.right_sort_key, {rule}::text AS rule, {group}::text AS evidence_group, p.{class} AS class, p.{score} AS score, {comparator}::text AS comparator, {version}::smallint AS comparator_version, p.{left_digest} AS left_value_digest, p.{right_digest} AS right_value_digest\nFROM {pairs} AS p",
        rule = sql_text(&rule.name),
        group = sql_text(&rule.evidence_group),
        class = evidence_value_column(rule_index, "class"),
        score = evidence_value_column(rule_index, "score"),
        comparator =
            sql_text(crate::comparators::comparator_name(&rule.comparison).unwrap_or("unknown_v1")),
        version = crate::comparators::comparator_version(&rule.comparison).unwrap_or(1),
        left_digest = evidence_value_column(rule_index, "left_value_digest"),
        right_digest = evidence_value_column(rule_index, "right_value_digest"),
        pairs = pair_relation,
    )
}

fn pair_evidence_sql_with_relation(rules: &[MatchRule], pair_relation: &str) -> String {
    if rules.is_empty() {
        return format!(
            "SELECT NULL::uuid AS left_source_record_id, NULL::uuid AS right_source_record_id, NULL::bytea AS left_sort_key, NULL::bytea AS right_sort_key, NULL::text AS rule, NULL::text AS evidence_group, NULL::text AS class, NULL::smallint AS score, NULL::text AS comparator, NULL::smallint AS comparator_version, NULL::bytea AS left_value_digest, NULL::bytea AS right_value_digest FROM {pair_relation} AS empty WHERE false",
        );
    }
    rules
        .iter()
        .enumerate()
        .map(|(index, rule)| evidence_rule_sql(rule, index, pair_relation))
        .collect::<Vec<_>>()
        .join("\nUNION ALL\n")
}

pub fn pair_evidence_sql_from_relation(rules: &[MatchRule], pair_relation: &str) -> String {
    pair_evidence_sql_with_relation(rules, pair_relation)
}

pub fn pair_evidence_sql(entity_name: &str, rules: &[MatchRule]) -> String {
    pair_evidence_sql_with_relation(rules, &node_ref(&format!("pairs/{entity_name}")))
}

fn candidate_evidence_sql(entity_name: &str, rules: &[MatchRule]) -> String {
    pair_evidence_sql_with_relation(rules, &node_ref(&format!("pairs/{entity_name}")))
}

fn block_relation(channel: &CandidateChannel) -> String {
    node_ref(&format!("blocks/{}", channel.channel_id))
}

fn stats_relation(channel: &CandidateChannel) -> String {
    node_ref(&format!("block-stats/{}", channel.channel_id))
}

fn block_overflow_relation(channel: &CandidateChannel) -> String {
    node_ref(&format!("block-overflow/{}", channel.channel_id))
}

fn candidate_result_relation(entity_name: &str) -> String {
    node_ref(&format!("candidate-set/{entity_name}"))
}

fn pair_stats_relation(entity_name: &str) -> String {
    node_ref(&format!("pair-stats/{entity_name}"))
}

fn channel_membership_sql(channel: &CandidateChannel) -> String {
    let relation = normalized_relation(&channel.fields[0]);
    let field = channel.fields[0].replace('\'', "''");
    let where_value = format!("field_name = '{field}' AND state = 'value'");
    match channel.kind {
        ChannelKind::Exact => format!(
            "SELECT source_record_id, field_name, '{channel_id}'::text AS channel_id, canonical_bytes AS block_key, source_sort_key\nFROM {relation}\nWHERE {where_value} AND canonical_bytes IS NOT NULL",
            channel_id = channel.channel_id.replace('\'', "''")
        ),
        ChannelKind::CompositeExact => {
            let aliases = channel
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let relation = normalized_relation(field);
                    let alias = format!("n{index}");
                    let field = field.replace('\'', "''");
                    (relation, alias, field)
                })
                .collect::<Vec<_>>();
            let from = format!(
                "FROM {} {}\n{}",
                aliases[0].0,
                aliases[0].1,
                aliases
                    .iter()
                    .skip(1)
                    .map(|(relation, alias, _)| {
                        format!("JOIN {relation} {alias} USING (source_record_id, source_sort_key)")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let value_filters = aliases
                .iter()
                .map(|(_, alias, field)| {
                    format!("{alias}.field_name = '{field}' AND {alias}.state = 'value'")
                })
                .collect::<Vec<_>>()
                .join(" AND ");
            let block_values = aliases
                .iter()
                .map(|(_, alias, _)| {
                    format!(
                        "pg_catalog.int4send(pg_catalog.octet_length({alias}.canonical_bytes)) || {alias}.canonical_bytes"
                    )
                })
                .collect::<Vec<_>>()
                .join(" || ");
            format!(
                "SELECT n0.source_record_id, n0.field_name, '{channel_id}'::text AS channel_id, {block_values} AS block_key, n0.source_sort_key\n{from}\nWHERE {value_filters}",
                channel_id = channel.channel_id.replace('\'', "''"),
                block_values = block_values,
                from = from,
                value_filters = value_filters,
            )
        }
        ChannelKind::Prefix => format!(
            "SELECT source_record_id, field_name, '{channel_id}'::text AS channel_id, pg_catalog.left(normalized, {length}) AS block_key, source_sort_key\nFROM {relation}\nWHERE {where_value} AND normalized IS NOT NULL",
            channel_id = channel.channel_id.replace('\'', "''"),
            length = channel.prefix_length.expect("validated prefix length")
        ),
        ChannelKind::Token => format!(
            "SELECT source_record_id, field_name, '{channel_id}'::text AS channel_id, token AS block_key, source_sort_key\nFROM {relation}\nCROSS JOIN LATERAL pg_catalog.regexp_split_to_table(normalized, '[[:space:]]+') AS token(token)\nWHERE {where_value} AND pg_catalog.char_length(token) >= {min_length}\nGROUP BY source_record_id, field_name, token, source_sort_key",
            channel_id = channel.channel_id.replace('\'', "''"),
            min_length = channel.token_min_length.expect("validated token length")
        ),
    }
}

pub fn candidate_block_sql(channel: &CandidateChannel) -> String {
    channel_membership_sql(channel)
}

pub fn candidate_block_stats_sql(channel: &CandidateChannel) -> String {
    let relation = block_relation(channel);
    format!(
        "SELECT channel_id, block_key, pg_catalog.count(*)::bigint AS block_records\nFROM {relation}\nGROUP BY channel_id, block_key"
    )
}

pub fn candidate_block_overflow_sql(
    channel: &CandidateChannel,
    limits: &crate::candidate::CandidateLimits,
) -> String {
    let relation = stats_relation(channel);
    format!(
        "SELECT channel_id, block_key, block_records, {limit}::bigint AS max_block_records\nFROM {relation}\nWHERE block_records > {limit}",
        limit = limits.max_block_records
    )
}

fn candidate_pair_source_sql_with_relation(
    channels: &[CandidateChannel],
    limits: &crate::candidate::CandidateLimits,
    fallback_relation: &str,
) -> String {
    let pairs = channels
        .iter()
        .map(|channel| {
            let blocks = block_relation(channel);
            let stats = stats_relation(channel);
            let overflow = block_overflow_relation(channel);
            format!(
                "SELECT '{channel_id}'::text AS candidate_channel, l.block_key::text AS candidate_block_key, l.source_record_id AS left_source_record_id, r.source_record_id AS right_source_record_id, l.source_sort_key AS left_sort_key, r.source_sort_key AS right_sort_key\nFROM {blocks} l\nJOIN {stats} s ON s.channel_id = l.channel_id AND s.block_key = l.block_key\nJOIN {blocks} r ON r.channel_id = l.channel_id AND r.block_key = l.block_key AND l.source_sort_key < r.source_sort_key\nLEFT JOIN {overflow} AS overflow ON overflow.channel_id = l.channel_id AND overflow.block_key = l.block_key\nWHERE s.block_records <= {limit} AND overflow.channel_id IS NULL",
                channel_id = channel.channel_id.replace('\'', "''"),
                limit = limits.max_block_records,
            )
        })
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        return format!(
            "SELECT NULL::text AS candidate_channel, NULL::text AS candidate_block_key, NULL::uuid AS left_source_record_id, NULL::uuid AS right_source_record_id, NULL::bytea AS left_sort_key, NULL::bytea AS right_sort_key FROM {fallback_relation} AS empty WHERE false"
        );
    }
    pairs.join("\nUNION ALL\n")
}

fn candidate_evidence_fields(rules: &[MatchRule]) -> Vec<String> {
    rules
        .iter()
        .flat_map(|rule| rule.fields.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn candidate_evidence_attributes() -> [&'static str; 3] {
    ["state", "normalized", "canonical_bytes"]
}

fn candidate_set_schema() -> Value {
    json!({
        "left_source_record_id":"uuid",
        "right_source_record_id":"uuid",
        "left_sort_key":"bytea",
        "right_sort_key":"bytea"
    })
}

fn candidate_set_sql(entity_name: &str) -> String {
    let source = node_ref(&format!("candidate-raw/{entity_name}"));
    format!(
        "SELECT candidate.left_source_record_id, candidate.right_source_record_id, pg_catalog.min(candidate.left_sort_key) AS left_sort_key, pg_catalog.min(candidate.right_sort_key) AS right_sort_key\nFROM {source} AS candidate\nGROUP BY candidate.left_source_record_id, candidate.right_source_record_id"
    )
}

fn candidate_pair_schema(fields: &[String], rules: &[MatchRule]) -> Value {
    let mut schema = serde_json::Map::new();
    for (name, kind) in [
        ("left_source_record_id", "uuid"),
        ("right_source_record_id", "uuid"),
        ("left_sort_key", "bytea"),
        ("right_sort_key", "bytea"),
    ] {
        schema.insert(name.into(), json!(kind));
    }
    for field in fields {
        for side in ["left", "right"] {
            for attribute in candidate_evidence_attributes() {
                let kind = if attribute == "canonical_bytes" {
                    "bytea"
                } else {
                    "text"
                };
                schema.insert(format!("{side}_evidence_{field}_{attribute}"), json!(kind));
            }
        }
    }
    for index in 0..rules.len() {
        for (attribute, kind) in [
            ("class", "text"),
            ("score", "smallint"),
            ("left_value_digest", "bytea"),
            ("right_value_digest", "bytea"),
        ] {
            schema.insert(
                evidence_value_column(index, attribute).replace('"', ""),
                json!(kind),
            );
        }
    }
    Value::Object(schema)
}

fn evidence_schema() -> Value {
    json!({
        "left_source_record_id":"uuid",
        "right_source_record_id":"uuid",
        "left_sort_key":"bytea",
        "right_sort_key":"bytea",
        "rule":"text",
        "evidence_group":"text",
        "class":"text",
        "score":"smallint",
        "comparator":"text",
        "comparator_version":"smallint",
        "left_value_digest":"bytea",
        "right_value_digest":"bytea"
    })
}

fn candidate_enrichment_joins(fields: &[String]) -> String {
    let mut joins = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        let relation = normalized_relation(field);
        let left = format!("evidence_left_{index}");
        let right = format!("evidence_right_{index}");
        joins.push(format!(
            "LEFT JOIN {relation} AS {left} ON {left}.source_record_id = candidate.left_source_record_id AND {left}.field_name = {field}\nLEFT JOIN {relation} AS {right} ON {right}.source_record_id = candidate.right_source_record_id AND {right}.field_name = {field}",
            field = sql_text(field)
        ));
    }
    if joins.is_empty() {
        String::new()
    } else {
        format!("\n{}", joins.join("\n"))
    }
}

fn deduplicated_candidate_pairs_sql(
    candidate_relation: &str,
    evidence_fields: &[String],
    entity_name: &str,
    rules: &[MatchRule],
) -> String {
    let mut columns = [
        "candidate.left_source_record_id".to_owned(),
        "candidate.right_source_record_id".to_owned(),
        "candidate.left_sort_key".to_owned(),
        "candidate.right_sort_key".to_owned(),
    ]
    .to_vec();
    for field in evidence_fields {
        for side in ["left", "right"] {
            for attribute in candidate_evidence_attributes() {
                columns.push(format!(
                    "candidate.{}",
                    quote_identifier(&format!("{side}_evidence_{field}_{attribute}"))
                ));
            }
        }
    }
    columns.extend(candidate_evidence_result_columns(rules, entity_name));
    format!(
        "SELECT {}\nFROM {candidate_relation} AS candidate\nWHERE NOT EXISTS (SELECT 1 FROM {candidate_relation} AS prior WHERE prior.left_source_record_id = candidate.left_source_record_id AND prior.right_source_record_id = candidate.right_source_record_id AND (prior.candidate_channel < candidate.candidate_channel OR (prior.candidate_channel = candidate.candidate_channel AND prior.candidate_block_key < candidate.candidate_block_key)))",
        columns.join(", "),
    )
}

fn candidate_pairs_sql_with_relation(
    channels: &[CandidateChannel],
    limits: &crate::candidate::CandidateLimits,
    fallback_relation: &str,
) -> String {
    let pairs = candidate_pair_source_sql_with_relation(channels, limits, fallback_relation);
    let relation = format!("({pairs})");
    deduplicated_candidate_pairs_sql(&relation, &[], "", &[])
}

pub fn candidate_pairs_sql(
    channels: &[CandidateChannel],
    limits: &crate::candidate::CandidateLimits,
) -> String {
    candidate_pairs_sql_with_relation(channels, limits, "pg_catalog.pg_class")
}

fn candidate_pair_stats_from_relation(entity_name: &str) -> String {
    format!(
        "SELECT pg_catalog.count(*)::bigint AS candidate_pairs FROM {}",
        candidate_result_relation(entity_name)
    )
}

pub fn candidate_pair_stats_sql(
    channels: &[CandidateChannel],
    limits: &crate::candidate::CandidateLimits,
) -> String {
    let pairs = candidate_pairs_sql_with_relation(channels, limits, "pg_catalog.pg_class");
    format!("SELECT pg_catalog.count(*)::bigint AS candidate_pairs FROM ({pairs}) AS pairs")
}

fn candidate_pair_output_sql(entity_name: &str, fields: &[String], rules: &[MatchRule]) -> String {
    let candidate = node_ref(&format!("candidate-set/{entity_name}"));
    let overflow = node_ref(&format!("pair-overflow/{entity_name}"));
    let mut columns = [
        "candidate.left_source_record_id".to_owned(),
        "candidate.right_source_record_id".to_owned(),
        "candidate.left_sort_key".to_owned(),
        "candidate.right_sort_key".to_owned(),
    ]
    .to_vec();
    for (index, field) in fields.iter().enumerate() {
        for side in ["left", "right"] {
            for attribute in candidate_evidence_attributes() {
                columns.push(format!(
                    "evidence_{side}_{index}.{attribute} AS {}",
                    quote_identifier(&format!("{side}_evidence_{field}_{attribute}")),
                ));
            }
        }
    }
    columns.extend(joined_evidence_result_columns(rules, entity_name, fields));
    format!(
        "SELECT {}\nFROM {candidate} AS candidate{}\nLEFT JOIN {overflow} AS overflow ON true\nWHERE overflow.candidate_pairs IS NULL",
        columns.join(", "),
        candidate_enrichment_joins(fields),
    )
}

pub fn candidate_pair_overflow_sql(
    entity_name: &str,
    limits: &crate::candidate::CandidateLimits,
) -> String {
    let relation = pair_stats_relation(entity_name);
    format!(
        "SELECT candidate_pairs, {limit}::bigint AS max_candidate_pairs\nFROM {relation}\nWHERE candidate_pairs > {limit}",
        relation = relation,
        limit = limits.max_candidate_pairs
    )
}

fn candidate_block_key_type(channel: &CandidateChannel) -> &'static str {
    match channel.kind {
        ChannelKind::Prefix | ChannelKind::Token => "text",
        ChannelKind::Exact | ChannelKind::CompositeExact => "bytea",
    }
}

fn match_node(channel: &CandidateChannel) -> Value {
    node_with_refresh_mode(
        format!("blocks/{}", channel.channel_id),
        channel
            .fields
            .iter()
            .map(|name| format!("normalized/{name}"))
            .collect(),
        candidate_block_sql(channel),
        json!({"source_record_id":"uuid","field_name":"text","channel_id":"text","block_key":candidate_block_key_type(channel),"source_sort_key":"bytea"}),
        "DIFFERENTIAL",
    )
}

fn relation_joins(dependency_ids: &[String]) -> String {
    dependency_ids
        .iter()
        .enumerate()
        .map(|(index, logical_id)| {
            format!(
                "LEFT JOIN {} AS dependency_{index} ON false",
                node_ref(logical_id),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn golden_sql(entity: &Entity, fallback_relation: &str, dependency_ids: &[String]) -> String {
    let rows = entity
        .golden_values
        .iter()
        .map(|golden| {
            let priority = entity
                .sources
                .iter()
                .enumerate()
                .map(|(index, source)| {
                    format!(
                        "WHEN {} THEN {}",
                        sql_text(&source.name),
                        index
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                "SELECT n.source_record_id, n.source_name, n.field_name, CASE n.source_name {priority} ELSE 2147483647 END::integer AS source_priority, n.row_changed_at, false AS authoritative, n.source_sort_key, n.raw_value, n.state, n.normalized, n.canonical_bytes\nFROM {} n\n{}\nWHERE n.field_name = {}",
                node_ref(&format!("normalized/{}", golden.field)),
                relation_joins(dependency_ids),
                sql_text(&golden.field),
            )
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        let dependency_joins = relation_joins(dependency_ids);
        format!(
            "SELECT NULL::uuid AS source_record_id, NULL::text AS source_name, NULL::text AS field_name, NULL::integer AS source_priority, NULL::timestamptz AS row_changed_at, NULL::boolean AS authoritative, NULL::bytea AS source_sort_key, NULL::text AS raw_value, NULL::text AS state, NULL::text AS normalized, NULL::bytea AS canonical_bytes FROM {fallback_relation} AS empty\n{dependency_joins}\nWHERE false",
            dependency_joins = dependency_joins,
        )
    } else {
        rows.join("\nUNION ALL\n")
    }
}

pub fn compile(entity: &Entity) -> Value {
    let mut nodes = Vec::new();
    for source in &entity.sources {
        nodes.push(source_node(&entity.name, source));
    }
    for field in &entity.fields {
        nodes.push(normalized_node(field, &entity.sources));
    }
    let limits = limits_for_entity(entity);
    let plan = CandidatePlan::from_entity(entity).unwrap_or_else(|_| CandidatePlan {
        channels: Vec::new(),
    });
    let pair_refresh_mode = "DIFFERENTIAL";
    let fallback_logical_id = entity
        .sources
        .first()
        .map(|source| format!("records/{}", source.name));
    let fallback_relation = fallback_logical_id
        .as_deref()
        .map(node_ref)
        .unwrap_or_else(|| "pg_catalog.pg_class".into());
    for channel in &plan.channels {
        nodes.push(match_node(channel));
        nodes.push(node_with_refresh_mode(
            format!("block-stats/{}", channel.channel_id),
            vec![format!("blocks/{}", channel.channel_id)],
            candidate_block_stats_sql(channel),
            json!({"channel_id":"text","block_key":candidate_block_key_type(channel),"block_records":"bigint"}),
            "DIFFERENTIAL",
        ));
        nodes.push(node_with_refresh_mode(
            format!("block-overflow/{}", channel.channel_id),
            vec![format!("block-stats/{}", channel.channel_id)],
            candidate_block_overflow_sql(channel, &limits),
            json!({"channel_id":"text","block_key":candidate_block_key_type(channel),"block_records":"bigint","max_block_records":"bigint"}),
            "DIFFERENTIAL",
        ));
    }
    let mut candidate_dependencies: Vec<String> = plan
        .channels
        .iter()
        .flat_map(|channel| {
            [
                format!("blocks/{}", channel.channel_id),
                format!("block-stats/{}", channel.channel_id),
                format!("block-overflow/{}", channel.channel_id),
            ]
        })
        .collect();
    if plan.channels.is_empty()
        && let Some(fallback) = &fallback_logical_id
    {
        candidate_dependencies.push(fallback.clone());
    }
    let candidate_raw_id = format!("candidate-raw/{}", entity.name);
    nodes.push(node_with_refresh_mode(
        candidate_raw_id.clone(),
        candidate_dependencies,
        candidate_pair_source_sql_with_relation(&plan.channels, &limits, &fallback_relation),
        json!({"candidate_channel":"text","candidate_block_key":"text","left_source_record_id":"uuid","right_source_record_id":"uuid","left_sort_key":"bytea","right_sort_key":"bytea"}),
        pair_refresh_mode,
    ));
    let evidence_fields = candidate_evidence_fields(&entity.matches);
    let candidate_set_id = format!("candidate-set/{}", entity.name);
    nodes.push(node_with_refresh_mode(
        candidate_set_id.clone(),
        vec![candidate_raw_id],
        candidate_set_sql(&entity.name),
        candidate_set_schema(),
        pair_refresh_mode,
    ));
    let mut enrichment_dependencies = vec![candidate_set_id.clone()];
    enrichment_dependencies.extend(
        evidence_fields
            .iter()
            .map(|field| format!("normalized/{field}")),
    );
    nodes.push(node_with_refresh_mode(
        format!("pair-stats/{}", entity.name),
        vec![candidate_set_id.clone()],
        candidate_pair_stats_from_relation(&entity.name),
        json!({"candidate_pairs":"bigint"}),
        pair_refresh_mode,
    ));
    nodes.push(node_with_refresh_mode(
        format!("pair-overflow/{}", entity.name),
        vec![format!("pair-stats/{}", entity.name)],
        candidate_pair_overflow_sql(&entity.name, &limits),
        json!({"candidate_pairs":"bigint","max_candidate_pairs":"bigint"}),
        pair_refresh_mode,
    ));
    nodes.push(node_with_refresh_mode(
        format!("pairs/{}", entity.name),
        enrichment_dependencies
            .iter()
            .cloned()
            .chain([format!("pair-overflow/{}", entity.name)])
            .collect(),
        candidate_pair_output_sql(&entity.name, &evidence_fields, &entity.matches),
        candidate_pair_schema(&evidence_fields, &entity.matches),
        pair_refresh_mode,
    ));
    nodes.push(node_with_refresh_mode(
        format!("evidence/{}", entity.name),
        vec![format!("pairs/{}", entity.name)],
        candidate_evidence_sql(&entity.name, &entity.matches),
        evidence_schema(),
        pair_refresh_mode,
    ));
    let golden_dependencies = entity
        .golden_values
        .iter()
        .map(|golden| format!("normalized/{}", golden.field))
        .chain(
            entity
                .fields
                .iter()
                .filter(|field| {
                    !evidence_fields.contains(&field.name)
                        && !entity
                            .golden_values
                            .iter()
                            .any(|golden| golden.field == field.name)
                })
                .map(|field| format!("normalized/{}", field.name)),
        )
        .chain([
            format!("pairs/{}", entity.name),
            format!("evidence/{}", entity.name),
        ])
        .collect::<Vec<_>>();
    nodes.push(node_with_refresh_mode(
        format!("golden/{}", entity.name),
        golden_dependencies.clone(),
        golden_sql(entity, &fallback_relation, &golden_dependencies),
        json!({
            "source_record_id":"uuid",
            "source_name":"text",
            "field_name":"text",
            "source_priority":"integer",
            "row_changed_at":"timestamptz",
            "authoritative":"boolean",
            "source_sort_key":"bytea",
            "raw_value":"text",
            "state":"text",
            "normalized":"text",
            "canonical_bytes":"bytea"
        }),
        "DIFFERENTIAL",
    ));
    json!({
        "format_version": ARTIFACT_FORMAT_VERSION,
        "compiler_version": COMPILER_VERSION,
        "executable": true,
        "roots": [format!("golden/{}", entity.name)],
        "nodes": nodes
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::parse_entity;
    use sha2::{Digest, Sha256};

    #[test]
    fn compiler_v5_fixture_is_exact_and_executable() {
        let bytes = include_bytes!("../tests/fixtures/graph_compiler_v5_customer.json");
        assert_eq!(
            crate::definition::canonical::hex(&Sha256::digest(bytes)),
            "8df1b622d391eac317cda276ef50cf77eaf657173c44ff8cd1d5632c05f90f01"
        );
        let artifact: Value = serde_json::from_slice(bytes).expect("fixture is valid JSON");
        assert_eq!(artifact["format_version"], 1);
        assert_eq!(artifact["compiler_version"], 5);
        assert_eq!(artifact["executable"], true);
        assert_eq!(artifact["roots"], json!(["golden/customer"]));
        assert!(artifact["nodes"].as_array().unwrap().iter().all(|node| {
            node["executable"] == true
                && node["initialize"] == false
                && node["orchestration_mode"] == "EXTERNAL"
                && node["defining_sql"]
                    .as_str()
                    .is_some_and(|sql| !sql.is_empty())
        }));
    }

    #[test]
    fn development_graph_is_external_and_not_initialized() {
        let entity = parse_entity(json!({
            "name":"customer",
            "sources":[],"fields":[],"matches":[],"golden_values":[],
            "preset":null,"limits":{},"execution_role":null
        }))
        .expect("test entity parses");
        let graph = compile(&entity);
        assert_eq!(graph["executable"], true);
        assert_eq!(graph["compiler_version"], 11);
        assert!(graph["nodes"].as_array().unwrap().iter().all(|node| {
            node["initialize"] == false && node["orchestration_mode"] == "EXTERNAL"
        }));
        artifact_nodes(&serde_json::to_vec(&graph).unwrap()).expect("artifact validates");
    }

    #[test]
    fn unused_normalized_fields_remain_connected_to_golden_root() {
        let entity = parse_entity(json!({
            "name":"customer", "sources":[],
            "fields":[
                {"name":"label","type":"text","cleaner":"none","cleaner_options":{},"display":"masked"},
                {"name":"amount","type":"numeric","cleaner":"none","cleaner_options":{},"display":"masked"}
            ],
            "matches":[],"golden_values":[],
            "preset":null,"limits":{},"execution_role":null
        }))
        .expect("test entity parses");
        let graph = compile(&entity);
        assert_eq!(graph["roots"], json!(["golden/customer"]));
        assert_eq!(
            graph["nodes"].as_array().unwrap().last().unwrap()["dependencies"],
            json!([
                "normalized/label",
                "normalized/amount",
                "pairs/customer",
                "evidence/customer"
            ])
        );
        artifact_nodes(&serde_json::to_vec(&graph).unwrap()).expect("artifact validates");
    }

    #[test]
    fn phase_two_graph_qualification_has_differential_modes_and_minimal_golden_fan_in() {
        let entity = parse_entity(json!({
            "name": "customer",
            "sources": [{
                "name": "crm",
                "relation": "public.crm_customer",
                "source_id": ["id"],
                "mode": "tracked",
                "fields": {"name": "display_name", "email": "email_address"},
                "row_changed_at": null,
                "soft_delete_when": null,
                "authority": {}
            }],
            "fields": [
                {"name": "name", "type": "text", "cleaner": "company_name", "cleaner_options": {}, "display": "masked"},
                {"name": "email", "type": "text", "cleaner": "email", "cleaner_options": {}, "display": "masked"}
            ],
            "matches": [{
                "name": "same_email",
                "fields": ["email"],
                "comparison": "exact",
                "strength": "identity",
                "evidence_group": "email",
                "candidate": {"kind": "exact", "field": "email"}
            }],
            "golden_values": [{"field": "name", "policy": "prefer_source", "sources": ["crm"]}],
            "preset": null,
            "limits": {},
            "execution_role": null
        }))
        .expect("parses");
        let graph = compile(&entity);
        let nodes = graph["nodes"].as_array().expect("nodes are an array");
        let channel_id = CandidatePlan::from_entity(&entity)
            .expect("candidate plan compiles")
            .channels[0]
            .channel_id
            .clone();
        let node = |logical_id: &str| {
            nodes
                .iter()
                .find(|node| node["logical_id"] == logical_id)
                .unwrap_or_else(|| panic!("missing node {logical_id}"))
        };

        for (logical_id, mode) in [
            "records/crm",
            "normalized/name",
            "normalized/email",
            "golden/customer",
        ]
        .into_iter()
        .map(|id| (id, "DIFFERENTIAL"))
        .chain([
            ("candidate-set/customer", "DIFFERENTIAL"),
            ("evidence/customer", "DIFFERENTIAL"),
        ]) {
            assert_eq!(node(logical_id)["refresh_mode"], mode);
        }
        for logical_id in [
            format!("blocks/{channel_id}"),
            format!("block-stats/{channel_id}"),
            format!("block-overflow/{channel_id}"),
            "pairs/customer".to_owned(),
            "pair-stats/customer".to_owned(),
            "pair-overflow/customer".to_owned(),
        ] {
            assert_eq!(
                node(&logical_id)["refresh_mode"],
                "DIFFERENTIAL",
                "{logical_id}"
            );
        }
        assert_eq!(
            node("golden/customer")["dependencies"],
            json!(["normalized/name", "pairs/customer", "evidence/customer"])
        );
        assert!(
            node("golden/customer")["dependencies"]
                .as_array()
                .unwrap()
                .iter()
                .all(|dependency| {
                    dependency
                        .as_str()
                        .is_some_and(|id| !id.starts_with("block-"))
                })
        );
        let evidence_sql = node("evidence/customer")["defining_sql"].as_str().unwrap();
        assert!(!evidence_sql.contains("count("));
        assert_eq!(
            node("evidence/customer")["dependencies"],
            json!(["pairs/customer"])
        );
        assert!(evidence_sql.contains(&node_ref("pairs/customer")));
        assert!(!evidence_sql.contains("normalized/"));
        assert!(!evidence_sql.contains("GROUP BY"));
        for logical_id in [
            format!("blocks/{channel_id}"),
            format!("block-stats/{channel_id}"),
            format!("block-overflow/{channel_id}"),
            "pair-stats/customer".to_owned(),
        ] {
            assert!(
                !evidence_sql.contains(&node_ref(&logical_id)),
                "{logical_id}"
            );
        }
        let golden_sql = node("golden/customer")["defining_sql"].as_str().unwrap();
        assert!(!golden_sql.contains("count("));
        assert!(golden_sql.contains(&node_ref("evidence/customer")));
        assert!(golden_sql.contains(&node_ref("pairs/customer")));

        let relations = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                (
                    node["logical_id"].as_str().unwrap().to_owned(),
                    format!("relation_{index}"),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for graph_node in nodes {
            let rendered = render_sql(graph_node["defining_sql"].as_str().unwrap(), &relations)
                .expect("all logical references render");
            assert!(!rendered.contains("@{"), "{}", graph_node["logical_id"]);
        }

        let mut reachable = BTreeSet::from(["golden/customer".to_owned()]);
        while let Some(dependency) = nodes
            .iter()
            .filter(|node| reachable.contains(node["logical_id"].as_str().unwrap()))
            .flat_map(|node| node["dependencies"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .find(|dependency| !reachable.contains(*dependency))
        {
            reachable.insert(dependency.to_owned());
        }
        assert!(
            [
                "records/crm",
                "normalized/email",
                "pairs/customer",
                "candidate-set/customer",
                "evidence/customer"
            ]
            .into_iter()
            .all(|logical_id| reachable.contains(logical_id))
        );
        assert!(reachable.contains(&format!("blocks/{channel_id}")));
        assert!(reachable.contains(&format!("block-stats/{channel_id}")));
        assert!(reachable.contains(&format!("block-overflow/{channel_id}")));
        assert!(reachable.contains("evidence/customer"));
        assert!(reachable.contains("candidate-raw/customer"));

        let candidate_raw = node("candidate-raw/customer");
        let candidate_set = node("candidate-set/customer");
        let pair_stats = node("pair-stats/customer");
        let pairs = node("pairs/customer");
        assert_eq!(
            candidate_set["dependencies"],
            json!(["candidate-raw/customer"])
        );
        assert!(candidate_set["defining_sql"].as_str().unwrap().contains(
            "GROUP BY candidate.left_source_record_id, candidate.right_source_record_id"
        ));
        assert_eq!(
            pair_stats["dependencies"],
            json!(["candidate-set/customer"])
        );
        assert_eq!(
            pairs["dependencies"],
            json!([
                "candidate-set/customer",
                "normalized/email",
                "pair-overflow/customer"
            ])
        );
        assert!(
            pair_stats["defining_sql"]
                .as_str()
                .unwrap()
                .contains("@{candidate-set/customer}")
        );
        assert!(
            !pair_stats["defining_sql"]
                .as_str()
                .unwrap()
                .contains("JOIN")
        );
        assert!(
            candidate_raw["defining_sql"]
                .as_str()
                .unwrap()
                .contains("@{blocks/same_email}")
        );
        assert!(
            !candidate_raw["defining_sql"]
                .as_str()
                .unwrap()
                .contains("UNION")
        );
        assert!(
            !candidate_raw["defining_sql"]
                .as_str()
                .unwrap()
                .contains("GROUP BY")
        );
        assert_eq!(candidate_set["defining_sql"], candidate_set_sql("customer"));
        assert!(
            pairs["defining_sql"]
                .as_str()
                .unwrap()
                .contains("FROM @{candidate-set/customer} AS candidate")
        );
        assert!(
            pairs["defining_sql"]
                .as_str()
                .unwrap()
                .contains("LEFT JOIN @{normalized/email} AS evidence_left_0")
        );
    }

    #[test]
    fn candidate_pairs_sql_returns_unique_pairs() {
        let channel = |channel_id: &str, field: &str| CandidateChannel {
            channel_id: channel_id.into(),
            kind: ChannelKind::Exact,
            fields: vec![field.into()],
            prefix_length: None,
            token_min_length: None,
            owners: Vec::new(),
        };
        let sql = candidate_pairs_sql(
            &[channel("email", "email"), channel("name", "name")],
            &crate::candidate::CandidateLimits {
                max_block_records: 10_000,
                max_candidate_pairs: 100_000,
            },
        );
        assert!(sql.contains("WHERE NOT EXISTS"));
        assert!(sql.contains("prior.candidate_channel < candidate.candidate_channel"));
        assert!(sql.contains("prior.candidate_block_key < candidate.candidate_block_key"));
        assert!(sql.contains("'email'::text AS candidate_channel"));
        assert!(sql.contains("'name'::text AS candidate_channel"));
        assert!(sql.contains("\nUNION ALL\n"));
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
            "matches": [{
                "name": "same_email",
                "fields": ["email"],
                "comparison": "exact",
                "strength": "identity",
                "evidence_group": "email",
                "candidate": {"kind": "exact", "field": "email"}
            }],
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
                .contains("mdm_graph.normalize_text")
        );
        assert!(
            norm_node["defining_sql"]
                .as_str()
                .unwrap()
                .contains("FROM @{records/crm}")
        );
        let evidence_node = nodes
            .iter()
            .find(|n| n["logical_id"] == "evidence/customer")
            .unwrap();
        assert_eq!(evidence_node["refresh_mode"], "DIFFERENTIAL");
        let evidence_sql = evidence_node["defining_sql"].as_str().unwrap();
        assert!(!evidence_sql.contains("concat_ws"));
        assert!(!evidence_sql.contains("array_to_string"));

        for logical_id in [
            "blocks/same_email",
            "block-stats/same_email",
            "block-overflow/same_email",
            "pairs/customer",
            "pair-stats/customer",
            "pair-overflow/customer",
        ] {
            let node = nodes
                .iter()
                .find(|node| node["logical_id"] == logical_id)
                .unwrap();
            assert_eq!(node["refresh_mode"], "DIFFERENTIAL", "{logical_id}");
        }
    }

    #[test]
    fn immutable_text_join_matches_concat_ws_null_skipping() {
        assert_eq!(
            immutable_text_join(&["l0".into(), "l1".into()]),
            "COALESCE(l0, ''::text) || CASE WHEN l1 IS NULL OR NOT (l0 IS NOT NULL) THEN ''::text ELSE pg_catalog.chr(31) END || COALESCE(l1, ''::text)"
        );
    }
}
