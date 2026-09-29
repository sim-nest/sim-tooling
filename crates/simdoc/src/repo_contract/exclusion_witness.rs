// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The witness that an excluded fixture is consumed by something real.
//!
//! An exclusion removes Cargo manifests from the repository contract, so it
//! must be tied to a native consumer that the repository's own validation
//! reaches. The tie is *parsed*, never searched for as text:
//!
//! | class | the consumer | what is checked |
//! | --- | --- | --- |
//! | `test-fixture` | a `test` target source of a covered package | the source is parsed as Rust; a live `#[test]` function, or a live item it reaches by qualified path, calls `consume_fixture("<fixture path>")` (the fixture path relative to the repository, the package, or the consumer) |
//! | `recipe-fixture` | a `recipe.toml` under a covered package's `recipes/` | the file is parsed as TOML; its typed top-level `fixtures` array names the fixture path |
//! | `focused-test-harness` | one command of the root manifest's `validation-commands` | the command is `cargo test` or `cargo run` with only `--manifest-path` naming the harness manifest and a few plain flags (never `--no-run`, target or test filters, or program arguments), and the harness has a test target (for `test`) or a binary target (for `run`) that Cargo would discover |
//!
//! Comments, doc attributes, `#[cfg(...)]`-disabled items (other than
//! `cfg(test)`), `#[ignore]`d tests, `if false` bodies, macro definitions,
//! unrelated TOML keys, same-named functions in other modules, and prose that
//! merely mentions the directory therefore prove nothing.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::workspace_policy::{ContractExclusion, ExcludedManifests, ExclusionClass};
use crate::worktree::Worktree;

/// Proves the declared consumer of `entry`'s exclusion.
pub(super) fn prove(
    worktree: &Worktree,
    metadata: &Value,
    entry: &ExcludedManifests,
) -> Result<(), String> {
    let exclusion = &entry.exclusion;
    let Some(consumer) = &exclusion.consumer else {
        return Err(format!(
            "{} exclusion {} names no consumer",
            exclusion.class.as_str(),
            exclusion.path
        ));
    };
    match exclusion.class {
        ExclusionClass::TestFixture => prove_test_fixture(worktree, metadata, exclusion, consumer),
        ExclusionClass::RecipeFixture => {
            prove_recipe_fixture(worktree, metadata, exclusion, consumer)
        }
        ExclusionClass::FocusedTestHarness => prove_harness(worktree, metadata, entry, consumer),
    }
}

/// The package (root directory) whose target or recipe is `consumer`.
fn owner_package(
    metadata: &Value,
    class: ExclusionClass,
    consumer_path: &Path,
) -> Result<PathBuf, String> {
    for package in super::workspace_policy::member_packages(metadata)? {
        let manifest = package["manifest_path"].as_str().unwrap_or_default();
        let Some(root) = Path::new(manifest).parent() else {
            continue;
        };
        let native = match class {
            ExclusionClass::TestFixture => package["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|target| {
                    target["kind"]
                        .as_array()
                        .is_some_and(|kinds| kinds.iter().any(|kind| kind == "test"))
                        && target["test"] != false
                })
                .any(|target| target["src_path"].as_str() == consumer_path.to_str()),
            ExclusionClass::RecipeFixture => {
                consumer_path.starts_with(root)
                    && consumer_path
                        .file_name()
                        .is_some_and(|name| name == "recipe.toml")
                    && consumer_path.strip_prefix(root).is_ok_and(|rest| {
                        rest.components().any(|part| part.as_os_str() == "recipes")
                    })
            }
            ExclusionClass::FocusedTestHarness => false,
        };
        if native {
            return Ok(root.to_path_buf());
        }
    }
    Err(String::new())
}

/// The spellings under which a consumer may name the fixture: relative to the
/// repository, to the owning package, and to the consumer's own directory.
fn accepted_names(repo: &Path, fixture: &Path, owner: &Path, consumer: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for base in [repo, owner, consumer.parent().unwrap_or(repo)] {
        if let Ok(relative) = fixture.strip_prefix(base) {
            let name = relative
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if !name.is_empty() {
                names.push(name);
            }
        }
    }
    names
}

/// Whether `literal` is exactly one of `names` (optionally with a trailing
/// slash or a leading `./`).
fn names_fixture(literal: &str, names: &[String]) -> bool {
    let literal = literal.strip_prefix("./").unwrap_or(literal);
    let literal = literal.trim_end_matches('/');
    names.iter().any(|name| name == literal)
}

fn prove_test_fixture(
    worktree: &Worktree,
    metadata: &Value,
    exclusion: &ContractExclusion,
    consumer: &str,
) -> Result<(), String> {
    let repo = worktree.root();
    let consumer_path = repo.join(consumer);
    worktree.owned_file(&consumer_path, "exclusion consumer")?;
    let Ok(owner) = owner_package(metadata, exclusion.class, &consumer_path) else {
        return Err(format!(
            "test-fixture exclusion {} names consumer {consumer}, which is not a test target of \
             a contract package",
            exclusion.path
        ));
    };
    let names = accepted_names(repo, &repo.join(&exclusion.path), &owner, &consumer_path);
    let text = crate::owned::read_to_string(&consumer_path)
        .map_err(|err| format!("exclusion consumer {consumer}: {err}"))?;
    let consumed = super::fixture_consumption::live_test_fixtures(&text)
        .map_err(|err| format!("exclusion consumer {consumer} does not parse as Rust: {err}"))?;
    if consumed
        .iter()
        .any(|fixture| names_fixture(fixture, &names))
    {
        Ok(())
    } else {
        Err(format!(
            "test-fixture exclusion {} is not consumed by {consumer}: no live #[test] function \
             there (or item it reaches by qualified path) calls `consume_fixture(\"...\")` with \
             one of {names:?}",
            exclusion.path
        ))
    }
}

fn prove_recipe_fixture(
    worktree: &Worktree,
    metadata: &Value,
    exclusion: &ContractExclusion,
    consumer: &str,
) -> Result<(), String> {
    let repo = worktree.root();
    let consumer_path = repo.join(consumer);
    worktree.owned_file(&consumer_path, "exclusion consumer")?;
    let Ok(owner) = owner_package(metadata, exclusion.class, &consumer_path) else {
        return Err(format!(
            "recipe-fixture exclusion {} names consumer {consumer}, which is not a `recipe.toml` \
             under `recipes/` of a contract package",
            exclusion.path
        ));
    };
    let names = accepted_names(repo, &repo.join(&exclusion.path), &owner, &consumer_path);
    let text = crate::owned::read_to_string(&consumer_path)
        .map_err(|err| format!("exclusion consumer {consumer}: {err}"))?;
    let table = text
        .parse::<toml::Table>()
        .map_err(|err| format!("exclusion consumer {consumer} does not parse as TOML: {err}"))?;
    // The one typed field: a top-level `fixtures` array of path strings.
    let declared = match table.get("fixtures") {
        None => Vec::new(),
        Some(toml::Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                format!("{consumer}: every entry of `fixtures` must be a path string")
            })?,
        Some(_) => {
            return Err(format!(
                "{consumer}: `fixtures` must be an array of path strings"
            ));
        }
    };
    if declared.iter().any(|value| names_fixture(value, &names)) {
        Ok(())
    } else {
        Err(format!(
            "recipe-fixture exclusion {} is not consumed by {consumer}: its typed `fixtures` \
             array names none of {names:?}",
            exclusion.path
        ))
    }
}

fn prove_harness(
    worktree: &Worktree,
    metadata: &Value,
    entry: &ExcludedManifests,
    consumer: &str,
) -> Result<(), String> {
    let exclusion = &entry.exclusion;
    let declared = metadata["metadata"]["sim"]["validation-commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|command| command == consumer);
    if !declared {
        return Err(format!(
            "focused-test-harness exclusion {} names consumer {consumer:?}, which is not one \
             of the root manifest's `validation-commands`",
            exclusion.path
        ));
    }
    let (verb, manifest) = cargo_invocation(consumer).ok_or_else(|| {
        format!(
            "focused-test-harness exclusion {}: consumer {consumer:?} is not a plain \
             `cargo test` or `cargo run` command: only {ALLOWED_FLAGS:?} and `--manifest-path` \
             are accepted (no `--no-run`, target or test filters, or program arguments)",
            exclusion.path
        )
    })?;
    if !entry.manifests.contains(&manifest) {
        return Err(format!(
            "focused-test-harness exclusion {}: consumer {consumer:?} runs {manifest}, which is \
             not a manifest of the excluded harness ({:?})",
            exclusion.path, entry.manifests
        ));
    }
    let dir = worktree
        .root()
        .join(&manifest)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let table = super::workspace_policy::read_manifest(&worktree.root().join(&manifest))?;
    if !has_cargo_target(worktree, &dir, &table, verb) {
        return Err(format!(
            "focused-test-harness {manifest} has no {} target Cargo would discover for its \
             validation command {consumer:?}",
            if verb == "test" { "test" } else { "binary" }
        ));
    }
    Ok(())
}

/// The only flags a harness command may carry besides `--manifest-path`.
const ALLOWED_FLAGS: [&str; 7] = [
    "--locked",
    "--offline",
    "--frozen",
    "--quiet",
    "-q",
    "--release",
    "--all-features",
];

/// `(verb, manifest path)` of `cargo test|run` with only [`ALLOWED_FLAGS`]
/// and `--manifest-path P`. Anything else (`--no-run`, target selection,
/// test filters, `--`, program arguments) is not a plain execution of the
/// harness and returns `None`.
fn cargo_invocation(command: &str) -> Option<(&'static str, String)> {
    if command
        .chars()
        .any(|ch| ";&|<>$`\"'()\\".contains(ch) || ch.is_control())
    {
        return None;
    }
    let mut tokens = command.split_whitespace();
    if tokens.next()? != "cargo" {
        return None;
    }
    let verb = match tokens.next()? {
        "test" => "test",
        "run" => "run",
        _ => return None,
    };
    let mut manifest = None;
    while let Some(token) = tokens.next() {
        let path = if token == "--manifest-path" {
            tokens.next()?
        } else if let Some(path) = token.strip_prefix("--manifest-path=") {
            path
        } else if ALLOWED_FLAGS.contains(&token) {
            continue;
        } else {
            return None;
        };
        if manifest
            .replace(path.strip_prefix("./").unwrap_or(path).to_owned())
            .is_some()
        {
            return None;
        }
    }
    Some((verb, manifest?))
}

/// Whether Cargo would discover and run a test target (verb `test`) or a
/// binary target (verb `run`) in the package at `dir`: exactly the automatic
/// layout (`tests/*.rs`, `tests/*/main.rs`; `src/main.rs`, `src/bin/*.rs`,
/// `src/bin/*/main.rs`) unless `autotests`/`autobins = false`, or a declared
/// `[[test]]`/`[[bin]]` whose source is an owned file. A virtual manifest has
/// no target. A declared target with `required-features`, or a declared test
/// with `test = false`, is not run by the plain command and does not count,
/// and a declared target replaces the automatic one of the same name. With
/// several binaries, plain `cargo run` runs exactly the one `default-run`
/// names, so for `run` that target itself must be runnable by this same
/// definition; some other, unrelated binary being fine is not enough.
fn has_cargo_target(worktree: &Worktree, dir: &Path, table: &toml::Table, verb: &str) -> bool {
    let Some(package) = table.get("package").and_then(toml::Value::as_table) else {
        return false;
    };
    let name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    let (kind, auto_key) = if verb == "test" {
        ("test", "autotests")
    } else {
        ("bin", "autobins")
    };
    let owned = worktree
        .files_under(dir)
        .into_iter()
        .filter_map(|file| {
            file.strip_prefix(dir).ok().map(|rest| {
                rest.to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/")
            })
        })
        .collect::<BTreeSet<_>>();
    let mut declared_names = BTreeSet::new();
    let mut runnable_names = BTreeSet::new();
    for target in table
        .get(kind)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
    {
        let target_name = target
            .get("name")
            .and_then(toml::Value::as_str)
            .unwrap_or_default();
        // Two declared targets of the same name is a manifest real Cargo
        // itself refuses outright (binary/test names must be unique); a
        // plain command against it never succeeds, so this is not "no
        // target" but is just as much a refusal. Caught here, not by
        // silently deduplicating into one name and undercounting how many
        // targets a `default-run` would need to disambiguate between.
        if !declared_names.insert(target_name.to_owned()) {
            return false;
        }
        let runs = target
            .get("required-features")
            .and_then(toml::Value::as_array)
            .is_none_or(Vec::is_empty)
            && !(verb == "test"
                && target.get("test").and_then(toml::Value::as_bool) == Some(false));
        let has_source = match target.get("path").and_then(toml::Value::as_str) {
            Some(path) => owned.contains(path),
            None if verb == "test" => {
                owned.contains(&format!("tests/{target_name}.rs"))
                    || owned.contains(&format!("tests/{target_name}/main.rs"))
            }
            None => {
                (target_name == name && owned.contains("src/main.rs"))
                    || owned.contains(&format!("src/bin/{target_name}.rs"))
                    || owned.contains(&format!("src/bin/{target_name}/main.rs"))
            }
        };
        // A declared target (explicit `path` or the conventional one for
        // its name) naming a source file that does not exist is a manifest
        // real Cargo refuses outright, for every verb against this
        // package -- not merely "this one target contributes nothing",
        // which would let some OTHER, unrelated valid target's existence
        // wrongly prove the package still runs.
        if !has_source {
            return false;
        }
        if runs {
            runnable_names.insert(target_name.to_owned());
        }
    }
    let auto_enabled = package
        .get(auto_key)
        .and_then(toml::Value::as_bool)
        .unwrap_or(true);
    let mut automatic_names = BTreeSet::new();
    if auto_enabled {
        for path in &owned {
            let parts = path.split('/').collect::<Vec<_>>();
            let discovered = match (verb == "test", parts.as_slice()) {
                (true, ["tests", file]) => file.strip_suffix(".rs"),
                (true, ["tests", dir, "main.rs"]) => Some(*dir),
                (false, ["src", "main.rs"]) => Some(name),
                (false, ["src", "bin", file]) => file.strip_suffix(".rs"),
                (false, ["src", "bin", dir, "main.rs"]) => Some(*dir),
                _ => None,
            };
            if let Some(found) = discovered
                && !declared_names.contains(found)
            {
                automatic_names.insert(found.to_owned());
                runnable_names.insert(found.to_owned());
            }
        }
    }
    if verb == "run" {
        // `cargo run` in a package with several binaries needs `default-run`,
        // and plain `cargo run` runs exactly the binary it names -- not just
        // any runnable one. A `default-run` naming a target that is absent,
        // or itself required-features-gated, is not proof that the plain
        // command succeeds, even when some other binary in the package is
        // fine -- checked whenever `default-run` is declared at all, not
        // only when more than one binary makes it load-bearing: Cargo
        // itself refuses a `default-run` naming a nonexistent target
        // regardless of how many binaries the package actually has. The
        // count used for the no-`default-run` case is every distinct
        // target name Cargo would actually see: a declared target
        // replacing an automatic one of the same name is one binary, not
        // two, and disabled automatic discovery (`autobins = false`)
        // contributes none at all.
        return match package.get("default-run").and_then(toml::Value::as_str) {
            Some(default_run) => runnable_names.contains(default_run),
            None if declared_names.len() + automatic_names.len() > 1 => false,
            None => !runnable_names.is_empty(),
        };
    }
    !runnable_names.is_empty()
}

#[cfg(test)]
#[path = "exclusion_witness_tests.rs"]
mod tests;
