// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Recording one item's own facts, and resolving a path written in a module
//! to the absolute item(s) it may name -- through the module tree, `use`
//! items, and glob imports (see `fixture_consumption`).

use syn::Attribute;

use super::{Collector, Items, disabled, is_test};

impl Items {
    pub(super) fn record(
        &mut self,
        name: String,
        attributes: &[Attribute],
        is_async: bool,
        visit: impl FnOnce(&mut Collector),
    ) {
        if disabled(attributes) {
            return;
        }
        let mut collector = Collector::default();
        visit(&mut collector);
        let mut key = self.module.clone();
        key.push(name);
        let facts = self.facts.entry(key).or_default();
        facts.is_test |= is_test(attributes);
        facts.is_async |= is_async;
        facts.fixture_calls.extend(collector.fixture_calls);
        facts.references.extend(collector.references);
    }

    /// The items a path written in `module` may name.
    pub(super) fn resolve(&self, module: &[String], path: &[String]) -> Vec<Vec<String>> {
        let join =
            |base: &[String], rest: &[String]| base.iter().chain(rest).cloned().collect::<Vec<_>>();
        let parent = || module[..module.len().saturating_sub(1)].to_vec();
        let candidates = match path.first().map(String::as_str) {
            None => Vec::new(),
            Some("crate") => vec![path[1..].to_vec()],
            Some("self") => vec![join(module, &path[1..])],
            Some("super") => vec![join(&parent(), &path[1..])],
            Some(first) => {
                // Rust's precedence: what the module itself defines shadows
                // an import, which shadows a glob import.
                let local = join(module, path);
                let scope = self.scopes.get(module);
                let imported = scope
                    .and_then(|scope| scope.aliases.get(first))
                    .map(|alias| join(alias, &path[1..]));
                if self.facts.contains_key(&local) {
                    vec![local]
                } else if imported
                    .as_ref()
                    .is_some_and(|found| self.facts.contains_key(found))
                {
                    imported.into_iter().collect()
                } else {
                    scope
                        .map(|scope| scope.globs.as_slice())
                        .unwrap_or_default()
                        .iter()
                        .map(|glob| join(glob, path))
                        .collect()
                }
            }
        };
        candidates
            .into_iter()
            .filter(|candidate| self.facts.contains_key(candidate))
            .collect()
    }

    pub(super) fn absolute(&self, path: &[String]) -> Vec<String> {
        let mut module = self.module.clone();
        let mut index = 0;
        match path.first().map(String::as_str) {
            Some("crate") => {
                module.clear();
                index = 1;
            }
            Some("self") => index = 1,
            Some("super") => {
                while path.get(index).map(String::as_str) == Some("super") {
                    module.pop();
                    index += 1;
                }
            }
            _ => {}
        }
        module.extend(path[index..].iter().cloned());
        module
    }

    pub(super) fn use_tree(&mut self, tree: &syn::UseTree, prefix: &mut Vec<String>) {
        match tree {
            syn::UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                self.use_tree(&path.tree, prefix);
                prefix.pop();
            }
            syn::UseTree::Name(name) => {
                let mut full = prefix.clone();
                full.push(name.ident.to_string());
                let target = self.absolute(&full);
                let scope = self.scopes.entry(self.module.clone()).or_default();
                scope.aliases.insert(name.ident.to_string(), target);
            }
            syn::UseTree::Rename(rename) => {
                let mut full = prefix.clone();
                full.push(rename.ident.to_string());
                let target = self.absolute(&full);
                let scope = self.scopes.entry(self.module.clone()).or_default();
                scope.aliases.insert(rename.rename.to_string(), target);
            }
            syn::UseTree::Glob(_) => {
                let target = self.absolute(prefix);
                let scope = self.scopes.entry(self.module.clone()).or_default();
                scope.globs.push(target);
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.use_tree(item, prefix);
                }
            }
        }
    }
}
