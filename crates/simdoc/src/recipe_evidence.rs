// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Whether a repository's recipes are checked: only by a declared validation
//! consumer (see [`recipe_harness`]).

use std::{collections::BTreeSet, path::Path};

/// The exact commands that count as running a repository's recipes, and how
/// each is recorded.
const RECIPE_CONSUMERS: [(&str, &str); 2] = [
    ("cargo run -p xtask -- check-recipes", "xtask check-recipes"),
    ("sh scripts/check-recipes.sh", "sh scripts/check-recipes.sh"),
];

/// How this repository's recipes are checked: `Some` only when a declared
/// validation consumer runs them. A consumer is an entry of the root
/// manifest's `[workspace.metadata.sim] validation-commands`, or a `run:`
/// step of a tracked CI workflow, equal to one of [`RECIPE_CONSUMERS`]
/// exactly. A script or an xtask command that merely exists proves nothing:
/// if nothing declared runs it, no recipe is claimed runnable or checked.
pub(crate) fn recipe_harness(repo: &Path) -> Option<String> {
    let declared = declared_validation_commands(repo);
    RECIPE_CONSUMERS
        .iter()
        .find(|(command, _)| {
            declared.contains(*command)
                && if *command == "sh scripts/check-recipes.sh" {
                    crate::owned::is_owned_file(repo.join("scripts/check-recipes.sh"))
                        && crate::owned::read_to_string(repo.join("scripts/check-recipes.sh"))
                            .is_ok_and(|text| standard_script(&text))
                } else {
                    // The xtask must really have the subcommand.
                    crate::owned::read_to_string(repo.join("xtask/src/main.rs"))
                        .is_ok_and(|text| dispatches_check_recipes(&text))
                }
        })
        .map(|(_, recorded)| (*recorded).to_owned())
}

/// The `run:` commands of the steps a GitHub Actions workflow really runs,
/// read from the parsed workflow tree (see [`crate::workflow_yaml`]): the
/// workflow triggers on `push` or `pull_request`; it has no `defaults`; the job
/// has no `if`, `continue-on-error`, or `defaults`; the step's `run` is a
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

/// Whether a tracked recipe script is a standard runner with known
/// semantics: a POSIX shell script that stops on error (`set -e`), runs at
/// least one `cargo test` or `cargo run` (as a line of its own, not
/// commented), and cannot swallow a failure (`|| true`, `|| :`, `exit 0`,
/// `set +e`, `-e` cleared). A script that does nothing, or hides what it
/// does, proves nothing.
pub(crate) fn standard_script(text: &str) -> bool {
    let code = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    let stops_on_error = code.iter().any(|line| {
        line.strip_prefix("set -").is_some_and(|flags| {
            flags
                .split_whitespace()
                .next()
                .is_some_and(|word| word.contains('e'))
        })
    });
    let runs_cargo = code
        .iter()
        .any(|line| line.starts_with("cargo test") || line.starts_with("cargo run"));
    let swallows = code.iter().any(|line| {
        line.contains("|| true")
            || line.contains("|| :")
            || line.contains("||true")
            || line.starts_with("exit 0")
            || line.contains("; exit 0")
            || line.contains("set +e")
            || line.contains("> /dev/null 2>&1 ||")
    });
    stops_on_error && runs_cargo && !swallows
}

/// Whether a source file dispatches the `check-recipes` subcommand to code
/// that does something: a match arm whose pattern is the string literal
/// `"check-recipes"` and whose body calls a function or method (an empty
/// arm, `()`, `Ok(())`, or a literal proves nothing). Parsed, not searched.
fn dispatches_check_recipes(source: &str) -> bool {
    use syn::visit::Visit;
    fn effective(expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Call(call) => !matches!(
                &*call.func,
                syn::Expr::Path(path)
                    if path.path.segments.last().is_some_and(|last| {
                        ["Ok", "Err", "Some", "None"].contains(&last.ident.to_string().as_str())
                    })
            ),
            syn::Expr::MethodCall(_) => true,
            syn::Expr::Try(inner) => effective(&inner.expr),
            syn::Expr::Paren(inner) => effective(&inner.expr),
            syn::Expr::Block(block) => block.block.stmts.iter().any(|statement| match statement {
                syn::Stmt::Expr(expr, _) => effective(expr),
                syn::Stmt::Local(local) => local
                    .init
                    .as_ref()
                    .is_some_and(|init| effective(&init.expr)),
                _ => false,
            }),
            syn::Expr::Return(ret) => ret.expr.as_deref().is_some_and(effective),
            _ => false,
        }
    }
    #[derive(Default)]
    struct Arms(bool);
    impl<'ast> Visit<'ast> for Arms {
        fn visit_arm(&mut self, arm: &'ast syn::Arm) {
            let pat = &arm.pat;
            let text = quote::quote!(#pat).to_string();
            if text.contains("\"check-recipes\"") && effective(&arm.body) {
                self.0 = true;
            }
            syn::visit::visit_arm(self, arm);
        }
    }
    let Ok(file) = syn::parse_file(source) else {
        return false;
    };
    let mut arms = Arms::default();
    arms.visit_file(&file);
    arms.0
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
