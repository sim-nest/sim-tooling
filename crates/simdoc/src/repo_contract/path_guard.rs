// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Nothing outside the repository root is recorded by path.
//!
//! Generated contract artifacts are published with the repository. A path
//! outside it (a sibling checkout, a private control plane, a gitignored
//! override, a home directory, or this machine's absolute layout) must never
//! appear in them. Every artifact is refused when it contains the
//! repository's absolute location, its parent directory, or the home
//! directory; JSON artifacts are additionally refused when any string value
//! is an absolute, home-relative, or parent-escaping path.

use std::{collections::BTreeMap, env, path::Path};

use serde_json::Value;

/// Refuses artifacts that record any location outside `repo` (canonical).
pub(crate) fn ensure_repository_local(
    repo: &Path,
    files: &BTreeMap<&'static str, String>,
) -> Result<(), String> {
    let mut locations = vec![repo.to_string_lossy().into_owned()];
    if let Some(parent) = repo.parent().filter(|parent| parent.parent().is_some()) {
        locations.push(parent.to_string_lossy().into_owned());
    }
    if let Some(home) = env::var_os("HOME").filter(|home| home.len() > 1) {
        locations.push(Path::new(&home).to_string_lossy().into_owned());
    }
    for (name, content) in files {
        if let Some(location) = locations
            .iter()
            .find(|location| content.contains(location.as_str()))
        {
            return Err(format!(
                "generated {name} records the local path {location}; nothing outside the \
                 repository may be recorded by path"
            ));
        }
        if name.ends_with(".json") {
            let value: Value = serde_json::from_str(content)
                .map_err(|err| format!("parse generated {name}: {err}"))?;
            if let Some(path) = outside_path(&value) {
                return Err(format!(
                    "generated {name} records {path:?}, a path outside the repository"
                ));
            }
        }
    }
    Ok(())
}

fn outside_path(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => is_outside_path(text).then_some(text.as_str()),
        Value::Array(items) => items.iter().find_map(outside_path),
        Value::Object(fields) => fields.values().find_map(outside_path),
        _ => None,
    }
}

fn is_outside_path(text: &str) -> bool {
    text.starts_with('/')
        || text.starts_with("~/")
        || text == ".."
        || text.starts_with("../")
        || text.contains("/../")
        || text.ends_with("/..")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(name: &'static str, content: &str) -> BTreeMap<&'static str, String> {
        BTreeMap::from([(name, content.to_owned())])
    }

    #[test]
    fn repository_relative_artifacts_pass() {
        let repo = Path::new("/work/constellation/sim-fixture");
        assert!(
            ensure_repository_local(
                repo,
                &files(
                    "provenance.json",
                    r#"{"api_docs":"target/doc/","manifest":"crates/a/Cargo.toml"}"#
                )
            )
            .is_ok()
        );
        assert!(ensure_repository_local(repo, &files("repo-contract.md", "`src/lib.rs`")).is_ok());
    }

    #[test]
    fn any_location_outside_the_repository_is_refused() {
        let repo = Path::new("/work/constellation/sim-fixture");
        for (name, content) in [
            (
                "provenance.json",
                r#"{"manifest":"../sim-private/.meta-workspace/Cargo.toml"}"#,
            ),
            (
                "provenance.json",
                r#"{"resolver":{"manifest":"/opt/meta/Cargo.toml"}}"#,
            ),
            (
                "repo-contract.json",
                r#"{"packages":[{"src":"crates/../../x.rs"}]}"#,
            ),
            ("repo-contract.json", r#"{"home":"~/cache"}"#),
            (
                "repo-contract.md",
                "built in /work/constellation/sim-private",
            ),
            (
                "sim-index-fragment.sx",
                "(unit /work/constellation/sim-fixture/src)",
            ),
        ] {
            let err = ensure_repository_local(repo, &files(name, content)).unwrap_err();
            assert!(err.contains(name), "{content}: {err}");
        }
    }
}
