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
//! | `test-fixture` | a `test` target source of a covered package | the source is parsed as Rust; a string literal equal to the fixture path (relative to the repository, the package, or the consumer) must sit in a live `#[test]` function or in a live item that function reaches by name |
//! | `recipe-fixture` | a `recipe.toml`, `book.toml`, or `chapter.toml` under a covered package's `recipes/` | the file is parsed as TOML; a string value equal to the fixture path (relative to the repository, the package, or the recipe) must exist |
//! | `focused-test-harness` | one command of the root manifest's `validation-commands` | the command is `cargo test` or `cargo run` with `--manifest-path` naming the harness manifest, and the harness package has a test target (for `test`) or a binary target (for `run`) |
//!
//! Comments, doc attributes, `#[cfg(...)]`-disabled items (other than
//! `cfg(test)`), `#[ignore]`d tests, `if false` bodies, macro definitions,
//! and prose that merely mentions the directory therefore prove nothing.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use proc_macro2::{TokenStream, TokenTree};
use serde_json::Value;
use syn::{
    Attribute, Expr, ExprIf, ImplItemFn, ItemConst, ItemFn, ItemMacro, ItemStatic, Lit, Macro,
    visit::Visit,
};

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
                })
                .any(|target| target["src_path"].as_str() == consumer_path.to_str()),
            ExclusionClass::RecipeFixture => {
                consumer_path.starts_with(root)
                    && consumer_path.file_name().is_some_and(|name| {
                        name == "recipe.toml" || name == "book.toml" || name == "chapter.toml"
                    })
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

/// Whether `literal` names one of `names` (optionally with a trailing slash
/// or the fixture's own `Cargo.toml`).
fn names_fixture(literal: &str, names: &[String]) -> bool {
    let literal = literal.strip_prefix("./").unwrap_or(literal);
    let literal = literal.trim_end_matches('/');
    let literal = literal.strip_suffix("/Cargo.toml").unwrap_or(literal);
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
    let literals = live_test_literals(&text)
        .map_err(|err| format!("exclusion consumer {consumer} does not parse as Rust: {err}"))?;
    if literals
        .iter()
        .any(|literal| names_fixture(literal, &names))
    {
        Ok(())
    } else {
        Err(format!(
            "test-fixture exclusion {} is not consumed by {consumer}: no live #[test] function \
             there (or item it reaches) holds a string literal equal to one of {names:?}",
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
            "recipe-fixture exclusion {} names consumer {consumer}, which is not a recipe, \
             book, or chapter manifest under `recipes/` of a contract package",
            exclusion.path
        ));
    };
    let names = accepted_names(repo, &repo.join(&exclusion.path), &owner, &consumer_path);
    let text = crate::owned::read_to_string(&consumer_path)
        .map_err(|err| format!("exclusion consumer {consumer}: {err}"))?;
    let table = text
        .parse::<toml::Table>()
        .map_err(|err| format!("exclusion consumer {consumer} does not parse as TOML: {err}"))?;
    let mut values = Vec::new();
    toml_strings(&toml::Value::Table(table), &mut values);
    if values.iter().any(|value| names_fixture(value, &names)) {
        Ok(())
    } else {
        Err(format!(
            "recipe-fixture exclusion {} is not consumed by {consumer}: no string value there \
             is equal to one of {names:?}",
            exclusion.path
        ))
    }
}

fn toml_strings(value: &toml::Value, out: &mut Vec<String>) {
    match value {
        toml::Value::String(text) => out.push(text.clone()),
        toml::Value::Array(items) => items.iter().for_each(|item| toml_strings(item, out)),
        toml::Value::Table(table) => table.values().for_each(|item| toml_strings(item, out)),
        _ => {}
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
             `cargo test` or `cargo run` command with `--manifest-path`",
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
    if !has_executable_target(worktree, &dir, &table, verb) {
        return Err(format!(
            "focused-test-harness {manifest} has no {} target for its validation command \
             {consumer:?}",
            if verb == "test" { "test" } else { "binary" }
        ));
    }
    Ok(())
}

/// `(verb, manifest path)` of a plain `cargo test|run ... --manifest-path P`.
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
    let mut tokens = tokens.peekable();
    while let Some(token) = tokens.next() {
        if token == "--manifest-path" {
            let path = tokens.next()?;
            return Some((verb, path.strip_prefix("./").unwrap_or(path).to_owned()));
        }
        if let Some(path) = token.strip_prefix("--manifest-path=") {
            return Some((verb, path.strip_prefix("./").unwrap_or(path).to_owned()));
        }
    }
    None
}

/// Whether the package at `dir` has a test target (verb `test`) or a binary
/// target (verb `run`): a declared `[[test]]`/`[[bin]]` whose source is an
/// owned file, or the automatic layout's owned source files.
fn has_executable_target(worktree: &Worktree, dir: &Path, table: &toml::Table, verb: &str) -> bool {
    let (declared, automatic): (&str, &[&str]) = if verb == "test" {
        ("test", &["tests"])
    } else {
        ("bin", &["src/main.rs", "src/bin"])
    };
    let owned = |relative: &str| {
        worktree
            .owned_file(&dir.join(relative), "harness target")
            .is_ok()
    };
    let declared_target = table
        .get(declared)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|target| target.get("path").and_then(toml::Value::as_str))
        .any(owned);
    declared_target
        || automatic.iter().any(|pattern| {
            worktree
                .files_under(&dir.join(pattern))
                .iter()
                .any(|file| file.extension().is_some_and(|ext| ext == "rs"))
                || owned(pattern)
        })
}

/// String literals a normal `cargo test` run can reach: those inside a live
/// `#[test]` function, or inside a live function, constant, or static that
/// such a function names, transitively.
pub(super) fn live_test_literals(source: &str) -> Result<Vec<String>, String> {
    let file = syn::parse_file(source).map_err(|err| err.to_string())?;
    let mut items = Items::default();
    items.visit_file(&file);
    let mut reached = BTreeSet::new();
    let mut pending = items
        .facts
        .iter()
        .filter(|(_, facts)| facts.is_test)
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    while let Some(name) = pending.pop() {
        if !reached.insert(name.clone()) {
            continue;
        }
        if let Some(facts) = items.facts.get(&name) {
            pending.extend(
                facts
                    .idents
                    .iter()
                    .filter(|ident| items.facts.contains_key(*ident))
                    .cloned(),
            );
        }
    }
    Ok(reached
        .iter()
        .filter_map(|name| items.facts.get(name))
        .flat_map(|facts| facts.literals.iter().cloned())
        .collect())
}

#[derive(Default)]
struct Facts {
    is_test: bool,
    literals: Vec<String>,
    idents: BTreeSet<String>,
}

#[derive(Default)]
struct Items {
    facts: BTreeMap<String, Facts>,
}

/// Whether an item's attributes take it out of a normal test build: a
/// `cfg` other than `cfg(test)`, or `#[ignore]`.
fn disabled(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        let text = quote::quote!(#attribute).to_string().replace(' ', "");
        (text.starts_with("#[cfg(") && text != "#[cfg(test)]") || text.starts_with("#[ignore")
    })
}

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| quote::quote!(#attribute).to_string().replace(' ', "") == "#[test]")
}

impl Items {
    fn record(
        &mut self,
        name: String,
        attributes: &[Attribute],
        visit: impl FnOnce(&mut Collector),
    ) {
        if disabled(attributes) {
            return;
        }
        let mut collector = Collector::default();
        visit(&mut collector);
        let facts = self.facts.entry(name).or_default();
        facts.is_test |= is_test(attributes);
        facts.literals.extend(collector.literals);
        facts.idents.extend(collector.idents);
    }
}

impl<'ast> Visit<'ast> for Items {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        self.record(item.sig.ident.to_string(), &item.attrs, |collector| {
            collector.visit_item_fn(item);
        });
        if !disabled(&item.attrs) {
            syn::visit::visit_item_fn(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        self.record(item.sig.ident.to_string(), &item.attrs, |collector| {
            collector.visit_impl_item_fn(item);
        });
        if !disabled(&item.attrs) {
            syn::visit::visit_impl_item_fn(self, item);
        }
    }

    fn visit_item_const(&mut self, item: &'ast ItemConst) {
        self.record(item.ident.to_string(), &item.attrs, |collector| {
            collector.visit_item_const(item);
        });
    }

    fn visit_item_static(&mut self, item: &'ast ItemStatic) {
        self.record(item.ident.to_string(), &item.attrs, |collector| {
            collector.visit_item_static(item);
        });
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !disabled(&item.attrs) {
            syn::visit::visit_item_mod(self, item);
        }
    }
}

/// Collects live string literals and identifiers of one item, skipping
/// attributes (doc comments are attributes), `if false` bodies, and nested
/// items that are disabled.
#[derive(Default)]
struct Collector {
    literals: Vec<String>,
    idents: BTreeSet<String>,
}

impl Collector {
    fn walk_tokens(&mut self, stream: TokenStream) {
        for tree in stream {
            match tree {
                TokenTree::Group(group) => self.walk_tokens(group.stream()),
                TokenTree::Ident(ident) => {
                    self.idents.insert(ident.to_string());
                }
                TokenTree::Literal(literal) => {
                    if let Ok(Lit::Str(text)) = syn::parse_str::<Lit>(&literal.to_string()) {
                        self.literals.push(text.value());
                    }
                }
                TokenTree::Punct(_) => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_attribute(&mut self, _: &'ast Attribute) {}

    fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
        self.idents.insert(ident.to_string());
    }

    fn visit_lit_str(&mut self, text: &'ast syn::LitStr) {
        self.literals.push(text.value());
    }

    fn visit_macro(&mut self, node: &'ast Macro) {
        self.visit_path(&node.path);
        self.walk_tokens(node.tokens.clone());
    }

    fn visit_item_macro(&mut self, _: &'ast ItemMacro) {}

    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        let always_false = matches!(
            &*node.cond,
            Expr::Lit(literal) if matches!(&literal.lit, Lit::Bool(value) if !value.value)
        );
        if always_false {
            if let Some((_, otherwise)) = &node.else_branch {
                self.visit_expr(otherwise);
            }
        } else {
            syn::visit::visit_expr_if(self, node);
        }
    }
}

#[cfg(test)]
#[path = "exclusion_witness_tests.rs"]
mod tests;
