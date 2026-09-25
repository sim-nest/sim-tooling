// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Recipe validation, owned by simdoc and executing nothing.
//!
//! Generating documentation never runs repository code: a repository's own
//! scripts and recipe binaries are validated by its own CI, not by the
//! generator that publishes its documentation. What simdoc does check, before
//! it generates anything, is that the recipe material it is about to publish
//! is well formed:
//!
//! - every `recipe.toml`, `book.toml`, and `chapter.toml` beneath a `recipes`
//!   directory is an ordinary file the Git worktree tracks (see
//!   [`crate::owned`]), read through the owned-read gate, and parses as TOML;
//! - every `recipe.toml` names itself with a non-empty `title` or `name`;
//! - a `setup` or `expected` value that names a file (no whitespace, an
//!   extension) is a relative path (no `..`, not absolute) below the recipe's
//!   own directory that the worktree tracks.

use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

const RECIPE_FILES: [&str; 3] = ["recipe.toml", "book.toml", "chapter.toml"];
const FILE_REFERENCES: [&str; 2] = ["setup", "expected"];

/// Validates every recipe file the worktree tracks beneath `root`, returning
/// how many recipes there are, or every problem found.
pub(crate) fn validate(root: &Path) -> Result<usize, String> {
    let owned = crate::owned::files_under(root)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut problems = Vec::new();
    let mut recipes = 0;
    for path in &owned {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let relative = path.strip_prefix(root).unwrap_or(path);
        let under_recipes = relative
            .parent()
            .is_some_and(|dir| dir.components().any(|part| part.as_os_str() == "recipes"));
        if !RECIPE_FILES.contains(&name) || !under_recipes {
            continue;
        }
        let shown = relative.display();
        let table = match crate::owned::read_to_string(path)
            .map_err(|err| err.to_string())
            .and_then(|text| text.parse::<toml::Table>().map_err(|err| err.to_string()))
        {
            Ok(table) => table,
            Err(err) => {
                problems.push(format!("{shown}: {err}"));
                continue;
            }
        };
        if name != "recipe.toml" {
            continue;
        }
        recipes += 1;
        if !["title", "name"].iter().any(|key| {
            table
                .get(*key)
                .and_then(toml::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        }) {
            problems.push(format!(
                "{shown}: names itself with neither `title` nor `name`"
            ));
        }
        for key in FILE_REFERENCES {
            let Some(value) = table.get(key) else {
                continue;
            };
            let Some(value) = value.as_str() else {
                problems.push(format!("{shown}: `{key}` is not a string"));
                continue;
            };
            if !looks_like_file(value) {
                continue;
            }
            let sibling = path.with_file_name(value);
            if !is_plain_relative(value) || !owned.contains(&sibling) {
                problems.push(format!(
                    "{shown}: `{key}` names {value:?}, which is not a tracked file below the \
                     recipe's directory"
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(recipes)
    } else {
        Err(format!("recipe validation failed: {}", problems.join("; ")))
    }
}

/// A value that reads as a file name rather than prose.
fn looks_like_file(value: &str) -> bool {
    !value.is_empty() && !value.contains(char::is_whitespace) && value.contains('.')
}

fn is_plain_relative(value: &str) -> bool {
    Path::new(value)
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
}

#[cfg(test)]
#[path = "recipe_gate_tests.rs"]
mod tests;
