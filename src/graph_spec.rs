use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::candidate::{CandidateChannel, CandidatePlan, ChannelKind};
use crate::definition::{Entity, Field, MatchRule, Source};
use crate::error::MdmError;
use crate::semantics;
use crate::source_record::quote_identifier;

pub const COMPILER_VERSION: i32 = 7;
pub const ARTIFACT_FORMAT_VERSION: i32 = 1;

fn node(id: String, dependencies: Vec<String>, sql: String, schema: Value) -> Value {
    node_with_refresh_mode(id, dependencies, sql, schema, "AUTO")
}

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
            || !matches!(node.refresh_mode.as_str(), "AUTO" | "FULL")
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

    node(
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

fn evidence_rule_sql(rule: &MatchRule, entity_name: &str) -> String {
    let mut joins = Vec::new();
    let mut usable = Vec::new();
    let mut exact_equal = Vec::new();
    let mut left_values = Vec::new();
    let mut right_values = Vec::new();
    for (index, field) in rule.fields.iter().enumerate() {
        let left = format!("l{index}");
        let right = format!("r{index}");
        let field_sql = sql_text(field);
        let relation = normalized_relation(field);
        joins.push(format!(
            "LEFT JOIN {relation} {left} ON {left}.source_record_id = p.left_source_record_id AND {left}.field_name = {field_sql}\nLEFT JOIN {relation} {right} ON {right}.source_record_id = p.right_source_record_id AND {right}.field_name = {field_sql}"
        ));
        usable.push(format!(
            "{left}.state = 'value' AND {right}.state = 'value' AND {left}.canonical_bytes IS NOT NULL AND {right}.canonical_bytes IS NOT NULL"
        ));
        exact_equal.push(format!("{left}.canonical_bytes = {right}.canonical_bytes"));
        left_values.push(format!("{left}.normalized"));
        right_values.push(format!("{right}.normalized"));
    }
    let usable = usable.join(" AND ");
    let exact_equal = exact_equal.join(" AND ");
    let left_text = format!(
        "pg_catalog.concat_ws(pg_catalog.chr(31), {})",
        left_values.join(", ")
    );
    let right_text = format!(
        "pg_catalog.concat_ws(pg_catalog.chr(31), {})",
        right_values.join(", ")
    );
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
    format!(
        "SELECT p.left_source_record_id, p.right_source_record_id, p.left_sort_key, p.right_sort_key, {rule}::text AS rule, {group}::text AS evidence_group, {class} AS class, CASE WHEN {usable} THEN {score} ELSE NULL::smallint END AS score, {comparator}::text AS comparator, {version}::smallint AS comparator_version, CASE WHEN {usable} THEN mdm_graph.evidence_digest({left_text}) ELSE NULL::bytea END AS left_value_digest, CASE WHEN {usable} THEN mdm_graph.evidence_digest({right_text}) ELSE NULL::bytea END AS right_value_digest\nFROM {pairs} p\n{}",
        joins.join("\n"),
        rule = sql_text(&rule.name),
        group = sql_text(&rule.evidence_group),
        class = class,
        score = score,
        comparator =
            sql_text(crate::comparators::comparator_name(&rule.comparison).unwrap_or("unknown_v1")),
        version = crate::comparators::comparator_version(&rule.comparison).unwrap_or(1),
        pairs = node_ref(&format!("pairs/{entity_name}")),
        usable = usable,
        left_text = left_text,
        right_text = right_text,
    )
}

fn pair_evidence_sql_with_relation(
    entity_name: &str,
    rules: &[MatchRule],
    fallback_relation: &str,
) -> String {
    if rules.is_empty() {
        return format!(
            "SELECT NULL::uuid AS left_source_record_id, NULL::uuid AS right_source_record_id, NULL::bytea AS left_sort_key, NULL::bytea AS right_sort_key, NULL::text AS rule, NULL::text AS evidence_group, NULL::text AS class, NULL::smallint AS score, NULL::text AS comparator, NULL::smallint AS comparator_version, NULL::bytea AS left_value_digest, NULL::bytea AS right_value_digest FROM {fallback_relation} AS empty WHERE false"
        );
    }
    rules
        .iter()
        .map(|rule| evidence_rule_sql(rule, entity_name))
        .collect::<Vec<_>>()
        .join("\nUNION ALL\n")
}

pub fn pair_evidence_sql(entity_name: &str, rules: &[MatchRule]) -> String {
    pair_evidence_sql_with_relation(entity_name, rules, "pg_catalog.pg_class")
}

fn block_relation(channel: &CandidateChannel) -> String {
    node_ref(&format!("blocks/{}", channel.channel_id))
}

fn stats_relation(channel: &CandidateChannel) -> String {
    node_ref(&format!("block-stats/{}", channel.channel_id))
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
            "SELECT '{channel_id}'::text AS channel_id, canonical_bytes AS block_key, source_record_id, source_sort_key\nFROM {relation}\nWHERE {where_value} AND canonical_bytes IS NOT NULL",
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
                "SELECT '{channel_id}'::text AS channel_id, {block_values} AS block_key, n0.source_record_id, n0.source_sort_key\n{from}\nWHERE {value_filters}",
                channel_id = channel.channel_id.replace('\'', "''"),
                block_values = block_values,
                from = from,
                value_filters = value_filters,
            )
        }
        ChannelKind::Prefix => format!(
            "SELECT '{channel_id}'::text AS channel_id, pg_catalog.convert_to(pg_catalog.left(normalized, {length}), 'UTF8') AS block_key, source_record_id, source_sort_key\nFROM {relation}\nWHERE {where_value} AND normalized IS NOT NULL",
            channel_id = channel.channel_id.replace('\'', "''"),
            length = channel.prefix_length.expect("validated prefix length")
        ),
        ChannelKind::Token => format!(
            "SELECT '{channel_id}'::text AS channel_id, pg_catalog.convert_to(token, 'UTF8') AS block_key, source_record_id, source_sort_key\nFROM {relation}\nCROSS JOIN LATERAL pg_catalog.regexp_split_to_table(normalized, '[[:space:]]+') AS token\nWHERE {where_value} AND pg_catalog.char_length(token) >= {min_length}\nGROUP BY token, source_record_id, source_sort_key",
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

fn candidate_pairs_sql_with_relation(
    channels: &[CandidateChannel],
    limits: &crate::candidate::CandidateLimits,
    fallback_relation: &str,
) -> String {
    let pairs = channels
        .iter()
        .map(|channel| {
            let relation = block_relation(channel);
            let stats = stats_relation(channel);
            let dedupe = if channel.kind == ChannelKind::Token {
                "\nGROUP BY l.source_record_id, r.source_record_id, l.source_sort_key, r.source_sort_key"
            } else {
                ""
            };
            format!(
                "SELECT l.source_record_id AS left_source_record_id, r.source_record_id AS right_source_record_id, l.source_sort_key AS left_sort_key, r.source_sort_key AS right_sort_key\nFROM {relation} l\nJOIN {stats} s ON s.channel_id = l.channel_id AND s.block_key = l.block_key\nJOIN {relation} r ON r.channel_id = l.channel_id AND r.block_key = l.block_key AND l.source_sort_key < r.source_sort_key\nWHERE s.block_records <= {limit}{dedupe}",
                limit = limits.max_block_records,
            )
        })
        .collect::<Vec<_>>();
    if pairs.is_empty() {
        return format!(
            "SELECT NULL::uuid AS left_source_record_id, NULL::uuid AS right_source_record_id, NULL::bytea AS left_sort_key, NULL::bytea AS right_sort_key FROM {fallback_relation} AS empty WHERE false"
        );
    }
    pairs.join("\nUNION\n")
}

pub fn candidate_pairs_sql(
    channels: &[CandidateChannel],
    limits: &crate::candidate::CandidateLimits,
) -> String {
    candidate_pairs_sql_with_relation(channels, limits, "pg_catalog.pg_class")
}

fn candidate_pair_stats_sql_with_relation(
    channels: &[CandidateChannel],
    limits: &crate::candidate::CandidateLimits,
    fallback_relation: &str,
) -> String {
    let pairs = candidate_pairs_sql_with_relation(channels, limits, fallback_relation);
    format!("SELECT pg_catalog.count(*)::bigint AS candidate_pairs FROM ({pairs}) AS pairs")
}

pub fn candidate_pair_stats_sql(
    channels: &[CandidateChannel],
    limits: &crate::candidate::CandidateLimits,
) -> String {
    candidate_pair_stats_sql_with_relation(channels, limits, "pg_catalog.pg_class")
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

fn match_node(channel: &CandidateChannel) -> Value {
    node(
        format!("blocks/{}", channel.channel_id),
        channel
            .fields
            .iter()
            .map(|name| format!("normalized/{name}"))
            .collect(),
        candidate_block_sql(channel),
        json!({"channel_id":"text","block_key":"bytea","source_record_id":"uuid","source_sort_key":"bytea"}),
    )
}

fn golden_sql(entity: &Entity, fallback_relation: &str, dependency_ids: &[String]) -> String {
    let guards = dependency_ids
        .iter()
        .map(|logical_id| {
            format!(
                "(SELECT pg_catalog.count(*) FROM {}) >= 0",
                node_ref(logical_id)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let guards = if guards.is_empty() {
        "true".into()
    } else {
        guards
    };
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
                "SELECT n.source_record_id, n.source_name, n.field_name, CASE n.source_name {priority} ELSE 2147483647 END::integer AS source_priority, n.row_changed_at, false AS authoritative, n.source_sort_key, n.raw_value, n.state, n.normalized, n.canonical_bytes\nFROM {} n\nWHERE n.field_name = {} AND {guards}",
                node_ref(&format!("normalized/{}", golden.field)),
                sql_text(&golden.field),
                guards = guards
            )
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        format!(
            "SELECT NULL::uuid AS source_record_id, NULL::text AS source_name, NULL::text AS field_name, NULL::integer AS source_priority, NULL::timestamptz AS row_changed_at, NULL::boolean AS authoritative, NULL::bytea AS source_sort_key, NULL::text AS raw_value, NULL::text AS state, NULL::text AS normalized, NULL::bytea AS canonical_bytes FROM {fallback_relation} AS empty WHERE false AND {guards} /* {} */",
            node_ref(&format!("evidence/{}", entity.name)),
            guards = guards
        )
    } else {
        format!(
            "{}\n/* {} */",
            rows.join("\nUNION ALL\n"),
            node_ref(&format!("evidence/{}", entity.name))
        )
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
    // ponytail: grouped token pairs use FULL refresh because pg_trickle 0.105.1 emits invalid deltas; restore AUTO when it supports unique-pair deltas.
    let pair_refresh_mode = if plan
        .channels
        .iter()
        .any(|channel| channel.kind == ChannelKind::Token)
    {
        "FULL"
    } else {
        "AUTO"
    };
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
        nodes.push(node(
            format!("block-stats/{}", channel.channel_id),
            vec![format!("blocks/{}", channel.channel_id)],
            candidate_block_stats_sql(channel),
            json!({"channel_id":"text","block_key":"bytea","block_records":"bigint"}),
        ));
        nodes.push(node(
            format!("block-overflow/{}", channel.channel_id),
            vec![format!("block-stats/{}", channel.channel_id)],
            candidate_block_overflow_sql(channel, &limits),
            json!({"channel_id":"text","block_key":"bytea","block_records":"bigint","max_block_records":"bigint"}),
        ));
    }
    let mut blocks: Vec<String> = plan
        .channels
        .iter()
        .map(|channel| format!("blocks/{}", channel.channel_id))
        .chain(
            plan.channels
                .iter()
                .map(|channel| format!("block-stats/{}", channel.channel_id)),
        )
        .chain(
            plan.channels
                .iter()
                .map(|channel| format!("block-overflow/{}", channel.channel_id)),
        )
        .chain(std::iter::once(format!("pair-stats/{}", entity.name)))
        .chain(std::iter::once(format!("pair-overflow/{}", entity.name)))
        .collect();
    if plan.channels.is_empty()
        && let Some(fallback) = &fallback_logical_id
    {
        blocks.push(fallback.clone());
    }
    nodes.push(node_with_refresh_mode(
        format!("pairs/{}", entity.name),
        blocks,
        candidate_pairs_sql_with_relation(&plan.channels, &limits, &fallback_relation),
        json!({"left_source_record_id":"uuid","right_source_record_id":"uuid","left_sort_key":"bytea","right_sort_key":"bytea"}),
        pair_refresh_mode,
    ));
    let mut pair_stats_dependencies = plan
        .channels
        .iter()
        .map(|channel| format!("blocks/{}", channel.channel_id))
        .chain(
            plan.channels
                .iter()
                .map(|channel| format!("block-stats/{}", channel.channel_id)),
        )
        .chain(
            plan.channels
                .iter()
                .map(|channel| format!("block-overflow/{}", channel.channel_id)),
        )
        .collect::<Vec<_>>();
    if plan.channels.is_empty()
        && let Some(fallback) = &fallback_logical_id
    {
        pair_stats_dependencies.push(fallback.clone());
    }
    nodes.push(node_with_refresh_mode(
        format!("pair-stats/{}", entity.name),
        pair_stats_dependencies,
        candidate_pair_stats_sql_with_relation(&plan.channels, &limits, &fallback_relation),
        json!({"candidate_pairs":"bigint"}),
        pair_refresh_mode,
    ));
    nodes.push(node(
        format!("pair-overflow/{}", entity.name),
        vec![format!("pair-stats/{}", entity.name)],
        candidate_pair_overflow_sql(&entity.name, &limits),
        json!({"candidate_pairs":"bigint","max_candidate_pairs":"bigint"}),
    ));
    let evidence_dependencies = plan
        .channels
        .iter()
        .map(|channel| format!("blocks/{}", channel.channel_id))
        .chain(
            entity
                .fields
                .iter()
                .map(|field| format!("normalized/{}", field.name)),
        )
        .chain(std::iter::once(format!("pairs/{}", entity.name)))
        .chain(
            entity
                .matches
                .is_empty()
                .then_some(fallback_logical_id.clone())
                .flatten(),
        )
        .collect::<Vec<_>>();
    nodes.push(node(
        format!("evidence/{}", entity.name),
        evidence_dependencies,
        pair_evidence_sql_with_relation(&entity.name, &entity.matches, &fallback_relation),
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
        }),
    ));
    let golden_dependencies = nodes
        .iter()
        .filter_map(|node| node["logical_id"].as_str().map(String::from))
        .collect::<Vec<_>>();
    nodes.push(node(
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
        assert_eq!(graph["compiler_version"], 7);
        assert!(graph["nodes"].as_array().unwrap().iter().all(|node| {
            node["initialize"] == false && node["orchestration_mode"] == "EXTERNAL"
        }));
        artifact_nodes(&serde_json::to_vec(&graph).unwrap()).expect("artifact validates");
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
                .contains("mdm_graph.normalize_text")
        );
        assert!(
            norm_node["defining_sql"]
                .as_str()
                .unwrap()
                .contains("FROM @{records/crm}")
        );
    }
}
