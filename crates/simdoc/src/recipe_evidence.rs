// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Whether a repository DECLARES a consumer for its recipes (see
//! [`declared_consumer`]).
//!
//! This is a structural fact about tracked files, never a semantic one:
//! simdoc does not know, and does not claim, that the consumer runs the
//! recipes, that it fails when a recipe fails, or that CI executes it. The
//! generated evidence says "declared consumer", not "checked".

use std::{collections::BTreeSet, path::Path};

/// The exact commands that count as a declared consumer of a repository's
/// recipes, and how each is recorded.
const RECIPE_CONSUMERS: [(&str, &str); 2] = [
    ("cargo run -p xtask -- check-recipes", "xtask check-recipes"),
    ("sh scripts/check-recipes.sh", "sh scripts/check-recipes.sh"),
];

/// The wording generated evidence carries for a declared consumer. It never
/// says the recipes were checked or validated.
pub(crate) fn declared_consumer_evidence(recorded: &str) -> String {
    format!("declared consumer, not validated by simdoc: {recorded}")
}

/// The declared consumer of this repository's recipes: `Some` only when a
/// tracked, structurally valid declaration names one of [`RECIPE_CONSUMERS`]
/// exactly, and the file it names is tracked (the script, or the xtask's
/// `main.rs`). A declaration is an entry of the root manifest's
/// `[workspace.metadata.sim] validation-commands`, or a `run:` step of a
/// tracked workflow directly in `.github/workflows` (see
/// [`workflow_run_steps`]). Nothing here reads the script or the xtask: what
/// the consumer does is not asserted.
pub(crate) fn declared_consumer(repo: &Path) -> Option<String> {
    let declared = declared_validation_commands(repo);
    RECIPE_CONSUMERS
        .iter()
        .find(|(command, _)| {
            declared.contains(*command)
                && crate::owned::is_owned_file(repo.join(
                    if *command == "sh scripts/check-recipes.sh" {
                        "scripts/check-recipes.sh"
                    } else {
                        "xtask/src/main.rs"
                    },
                ))
        })
        .map(|(_, recorded)| (*recorded).to_owned())
}

/// The `run:` commands of the steps a GitHub Actions workflow really runs,
/// read from the parsed workflow tree (see [`crate::workflow_yaml`]): the
/// workflow triggers on `push` or `pull_request`; it has no `defaults`; the job
/// has `runs-on` and a `steps` list and no `if`, `continue-on-error`, or `defaults`; the step's `run` is a
/// single command (a one-line block scalar counts) and the step has no `if`,
/// `continue-on-error`, `working-directory`, or `shell`. A workflow the reader
/// cannot parse yields nothing.
fn workflow_run_steps(text: &str) -> Vec<String> {
    use crate::workflow_yaml::{Yaml, parse};
    let Ok(workflow) = parse(text) else {
        return Vec::new();
    };
    let triggers = |value: &Yaml| -> bool {
        let names = match value {
            Yaml::Scalar(name) => vec![name.as_str()],
            Yaml::Seq(items) => items.iter().filter_map(Yaml::scalar).collect(),
            Yaml::Map(entries) => entries.iter().map(|(name, _)| name.as_str()).collect(),
        };
        names
            .iter()
            .any(|name| *name == "push" || *name == "pull_request")
    };
    if workflow.get("defaults").is_some() || !workflow.get("on").is_some_and(triggers) {
        return Vec::new();
    }
    let Some(Yaml::Map(jobs)) = workflow.get("jobs") else {
        return Vec::new();
    };
    let mut commands = Vec::new();
    for (_, job) in jobs {
        if ["if", "continue-on-error", "defaults"]
            .iter()
            .any(|key| job.get(key).is_some())
        {
            continue;
        }
        if !job.get("runs-on").is_some_and(|runner| match runner {
            Yaml::Scalar(name) => !name.trim().is_empty(),
            Yaml::Seq(items) => !items.is_empty(),
            Yaml::Map(_) => true,
        }) {
            continue;
        }
        let Some(Yaml::Seq(steps)) = job.get("steps") else {
            continue;
        };
        for step in steps {
            if ["if", "continue-on-error", "working-directory", "shell"]
                .iter()
                .any(|key| step.get(key).is_some())
            {
                continue;
            }
            let Some(run) = step.get("run").and_then(Yaml::scalar) else {
                continue;
            };
            let lines = run
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>();
            if let [only] = lines.as_slice() {
                commands.push(only.trim().to_owned());
            }
        }
    }
    commands
}

/// Every command the repository declares it validates with.
fn declared_validation_commands(repo: &Path) -> BTreeSet<String> {
    let mut commands = BTreeSet::new();
    if let Ok(text) = crate::owned::read_to_string(repo.join("Cargo.toml"))
        && let Ok(table) = text.parse::<toml::Table>()
        && let Some(entries) = table
            .get("workspace")
            .and_then(|value| value.get("metadata"))
            .and_then(|value| value.get("sim"))
            .and_then(|value| value.get("validation-commands"))
            .and_then(toml::Value::as_array)
    {
        commands.extend(
            entries
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_owned),
        );
    }
    let workflows = repo.join(".github/workflows");
    for file in crate::owned::files_under(&workflows) {
        let direct = file.parent() == Some(workflows.as_path());
        let yaml = file
            .extension()
            .is_some_and(|extension| extension == "yml" || extension == "yaml");
        if !direct || !yaml {
            continue;
        }
        if let Ok(text) = crate::owned::read_to_string(&file) {
            commands.extend(workflow_run_steps(&text));
        }
    }
    commands
}

#[cfg(test)]
#[path = "recipe_evidence_tests.rs"]
mod tests;
