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
                } else {
                    // The xtask must really have the subcommand.
                    crate::owned::read_to_string(repo.join("xtask/src/main.rs"))
                        .is_ok_and(|text| dispatches_check_recipes(&text))
                }
        })
        .map(|(_, recorded)| (*recorded).to_owned())
}

/// The `run:` commands of the steps a GitHub Actions workflow really runs,
/// read structurally (2-space indentation; anything else yields nothing):
/// the workflow has a top-level `on:` naming `push` or `pull_request`; the
/// step is a `- ` item of a job's `steps:` with an inline `run:` value (a
/// block scalar, or a `run:` deeper in a `with:` script, is not a step's
/// command); neither the job nor the step carries `if:`, `continue-on-error:`,
/// or (step) `working-directory:` or `shell:`, in any key order; and the
/// workflow has no `defaults:` (which can redirect every step).
fn workflow_run_steps(text: &str) -> Vec<String> {
    #[derive(Default)]
    struct Step {
        run: Option<String>,
        blocked: bool,
    }
    #[derive(Default)]
    struct Job {
        blocked: bool,
        steps: Vec<Step>,
    }
    fn property(step: &mut Step, line: &str) {
        if let Some(value) = line.strip_prefix("run:") {
            let value = value.trim();
            step.run = (!value.is_empty())
                .then(|| value.trim_matches(|ch| ch == '"' || ch == '\'').to_owned());
        } else if ["if:", "continue-on-error:", "working-directory:", "shell:"]
            .iter()
            .any(|key| line.starts_with(key))
        {
            step.blocked = true;
        }
    }
    let mut commands = Vec::new();
    let (mut has_trigger, mut in_on, mut in_jobs, mut in_steps) = (false, false, false, false);
    let mut job: Option<Job> = None;
    let flush_job = |job: &mut Option<Job>, commands: &mut Vec<String>| {
        if let Some(job) = job.take()
            && !job.blocked
        {
            commands.extend(
                job.steps
                    .into_iter()
                    .filter(|step| !step.blocked)
                    .filter_map(|step| step.run),
            );
        }
    };
    for raw in text.lines() {
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("defaults:") {
            return Vec::new();
        }
        let indent = raw.len() - line.len();
        let names_trigger = |text: &str| {
            let text = text.trim_start_matches("- ");
            ["push", "pull_request"]
                .iter()
                .any(|trigger| text.starts_with(trigger))
        };
        match indent {
            0 => {
                flush_job(&mut job, &mut commands);
                in_steps = false;
                in_jobs = line == "jobs:";
                in_on = line == "on:";
                if let Some(inline) = line.strip_prefix("on:") {
                    has_trigger |= inline.contains("push") || inline.contains("pull_request");
                }
            }
            _ if in_on => has_trigger |= names_trigger(line),
            _ if !in_jobs => {}
            2 => {
                flush_job(&mut job, &mut commands);
                in_steps = false;
                job = Some(Job::default());
            }
            4 => {
                in_steps = line.starts_with("steps:");
                if ["if:", "continue-on-error:", "defaults:"]
                    .iter()
                    .any(|key| line.starts_with(key))
                    && let Some(job) = job.as_mut()
                {
                    job.blocked = true;
                }
            }
            6 if in_steps && line.starts_with("- ") => {
                let mut step = Step::default();
                property(&mut step, line[2..].trim_start());
                if let Some(job) = job.as_mut() {
                    job.steps.push(step);
                }
            }
            8 if in_steps => {
                if let Some(step) = job.as_mut().and_then(|job| job.steps.last_mut()) {
                    property(step, line);
                }
            }
            _ => {}
        }
    }
    flush_job(&mut job, &mut commands);
    if has_trigger { commands } else { Vec::new() }
}

/// Whether a source file dispatches the `check-recipes` subcommand: a string
/// literal `"check-recipes"` in a match-arm pattern (`Some("check-recipes")`
/// or `"check-recipes" =>`), parsed, not searched for.
fn dispatches_check_recipes(source: &str) -> bool {
    use syn::visit::Visit;
    #[derive(Default)]
    struct Arms(bool);
    impl<'ast> Visit<'ast> for Arms {
        fn visit_arm(&mut self, arm: &'ast syn::Arm) {
            let pat = &arm.pat;
            self.0 |= quote::quote!(#pat)
                .to_string()
                .contains("\"check-recipes\"");
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
