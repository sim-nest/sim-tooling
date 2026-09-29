// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Canonical JSON rendering for committed generated artifacts.

use serde_json::Value;

pub(crate) fn compact(mut value: Value, context: &str) -> Result<String, String> {
    value.sort_all_objects();
    serde_json::to_string(&value).map_err(|err| format!("serialize {context}: {err}"))
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value};

    use super::compact;

    #[test]
    fn compact_rendering_ignores_nested_object_insertion_order() {
        let first = nested_object([("zeta", 2), ("alpha", 1)]);
        let second = nested_object([("alpha", 1), ("zeta", 2)]);

        assert_eq!(
            compact(first, "fixture").unwrap(),
            compact(second, "fixture").unwrap()
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
