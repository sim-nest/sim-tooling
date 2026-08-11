//! Canonical JSON rendering for committed generated artifacts.

use serde_json::Value;

pub(crate) fn compact(mut value: Value, context: &str) -> Result<String, String> {
    value.sort_all_objects();
    serde_json::to_string(&value).map_err(|err| format!("serialize {context}: {err}"))
}

pub(crate) fn pretty(mut value: Value, context: &str) -> Result<String, String> {
    value.sort_all_objects();
    let mut out = serde_json::to_string_pretty(&value)
        .map_err(|err| format!("serialize {context}: {err}"))?;
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value};

    use super::{compact, pretty};

    #[test]
    fn renderers_ignore_nested_object_insertion_order() {
        let first = nested_object([("zeta", 2), ("alpha", 1)]);
        let second = nested_object([("alpha", 1), ("zeta", 2)]);

        assert_eq!(
            compact(first.clone(), "fixture").unwrap(),
            compact(second.clone(), "fixture").unwrap()
        );
        assert_eq!(
            pretty(first, "fixture").unwrap(),
            pretty(second, "fixture").unwrap()
        );
    }

    fn nested_object(entries: [(&str, i64); 2]) -> Value {
        let mut nested = Map::new();
        for (key, value) in entries {
            nested.insert(key.to_owned(), Value::from(value));
        }
        let mut root = Map::new();
        root.insert("nested".to_owned(), Value::Object(nested));
        root.insert("schema".to_owned(), Value::from("sim.contract"));
        Value::Object(root)
    }
}
