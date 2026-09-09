use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

fn sorted(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, sorted(value)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(sorted).collect()),
        other => other,
    }
}

pub(crate) fn canonical_definition(value: Value) -> Value {
    let mut value = sorted(value);
    if let Value::Object(object) = &mut value {
        for key in ["sources", "fields", "matches", "golden_values"] {
            if let Some(Value::Array(values)) = object.get_mut(key) {
                values.sort_by(|left, right| {
                    let left_name = left
                        .get(if key == "golden_values" {
                            "field"
                        } else {
                            "name"
                        })
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let right_name = right
                        .get(if key == "golden_values" {
                            "field"
                        } else {
                            "name"
                        })
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    left_name.cmp(right_name)
                });
            }
        }
    }
    value
}

pub(crate) fn json_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("JSON values are serializable")
}

pub(crate) fn digest(tag: &str, parts: &[&[u8]]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    write_part(&mut hasher, tag.as_bytes());
    for part in parts {
        write_part(&mut hasher, part);
    }
    hasher.finalize().to_vec()
}

fn write_part(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u32).to_be_bytes());
    hasher.update(bytes);
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_definition_sorts_nouns_but_not_meaningful_arrays() {
        let value = canonical_definition(json!({
            "sources": [{"name":"b"},{"name":"a"}],
            "source_id": ["z", "a"]
        }));
        assert_eq!(value["sources"][0]["name"], "a");
        assert_eq!(value["source_id"], json!(["z", "a"]));
    }

    #[test]
    fn digest_is_domain_separated() {
        assert_ne!(digest("a", &[b"b"]), digest("b", &[b"a"]));
    }
}
