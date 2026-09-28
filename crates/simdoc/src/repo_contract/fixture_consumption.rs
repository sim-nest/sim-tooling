// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which fixtures a normal `cargo test` run consumes, read from a test
//! target's source (see [`super::exclusion_witness`]).

use std::collections::{BTreeMap, BTreeSet};

use syn::{
    Attribute, Expr, ExprCall, ExprIf, ItemConst, ItemFn, ItemStatic, Lit, Macro, visit::Visit,
};

/// The fixtures a normal `cargo test` run consumes: the first argument of
/// every `consume_fixture("...")` call in a live `#[test]` function or in a
/// live function, constant, or static such a function reaches by qualified
/// path (resolved through the module tree, `use` items, and glob imports;
/// never by bare identifier across modules).
pub(super) fn live_test_fixtures(source: &str) -> Result<Vec<String>, String> {
    let file = syn::parse_file(source).map_err(|err| err.to_string())?;
    // A file-level `#![cfg(...)]` (other than `test`) or `#![cfg_attr(...)]`
    // can switch the whole target off.
    if disabled(&file.attrs) {
        return Ok(Vec::new());
    }
    let mut items = Items::default();
    items.visit_file(&file);
    // The consumption API must exist in the file as the reviewed shape: a
    // function `consume_fixture` that joins its argument onto the package's
    // manifest directory. A no-op stand-in proves nothing.
    if items.reviewed_helpers.is_empty() {
        return Ok(Vec::new());
    }
    let mut reached = BTreeSet::new();
    let mut pending = items
        .facts
        .iter()
        .filter(|(_, facts)| facts.is_test)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    while let Some(key) = pending.pop() {
        if !reached.insert(key.clone()) {
            continue;
        }
        let Some(facts) = items.facts.get(&key) else {
            continue;
        };
        let module = &key[..key.len() - 1];
        for reference in &facts.references {
            pending.extend(items.resolve(module, reference));
        }
    }
    let mut consumed = Vec::new();
    for key in &reached {
        let Some(facts) = items.facts.get(key) else {
            continue;
        };
        let module = &key[..key.len() - 1];
        for (path, fixture) in &facts.fixture_calls {
            // A bare, unqualified reference inside a body that locally
            // shadows `consume_fixture` always binds to that shadow, per
            // Rust's own scoping rules: never credited, regardless of what
            // an outer, module-tree resolution would otherwise find.
            if facts.shadows_consume_fixture && path.len() == 1 {
                continue;
            }
            if items
                .resolve(module, path)
                .iter()
                .any(|target| items.reviewed_helpers.contains(target))
            {
                consumed.push(fixture.clone());
            }
        }
    }
    Ok(consumed)
}

#[derive(Default)]
struct Facts {
    is_test: bool,
    /// Every syntactically `consume_fixture("...")`-shaped call this item's
    /// body makes, as its own (unresolved) callee path and the literal
    /// argument. Resolved against [`Items::reviewed_helpers`] once the whole
    /// file is known (see [`live_test_fixtures`]), never credited from the
    /// name alone: a same-named decoy elsewhere in the file, reached only
    /// because its own final path segment matches, proves nothing.
    fixture_calls: Vec<(Vec<String>, String)>,
    /// Whether this item's own body (or its own signature) locally binds
    /// the name `consume_fixture`: a nested `fn`, a `let` binding, or a
    /// parameter. Rust's own scoping rules mean a bare, unqualified
    /// `consume_fixture(...)` call inside such a body always binds to that
    /// local shadow, never to any outer, reviewed helper of the same name
    /// -- so a bare reference is never credited when this is set,
    /// regardless of what [`Items::resolve`] would otherwise find.
    shadows_consume_fixture: bool,
    references: Vec<Vec<String>>,
}

#[derive(Default)]
struct Scope {
    aliases: BTreeMap<String, Vec<String>>,
    globs: Vec<Vec<String>>,
}

#[derive(Default)]
struct Items {
    /// Absolute paths of every function named `consume_fixture`, of the
    /// reviewed shape, defined anywhere in the file -- never a single
    /// file-wide flag: a call must resolve to one of these exact items, not
    /// merely share its final name with one.
    reviewed_helpers: BTreeSet<Vec<String>>,
    module: Vec<String>,
    facts: BTreeMap<Vec<String>, Facts>,
    scopes: BTreeMap<Vec<String>, Scope>,
}

/// Whether an attribute (as `#[...]` text without spaces) takes its item out
/// of a normal test build: a `cfg` other than `cfg(test)`, any `cfg_attr`
/// (which may add `ignore`), or `ignore`.
fn attribute_disabled(text: &str) -> bool {
    if let Some(predicate) = text
        .strip_prefix("#[cfg(")
        .and_then(|rest| rest.strip_suffix(")]"))
    {
        // Live only when the predicate is decidable and true on this host.
        return syn::parse_str::<syn::Meta>(predicate)
            .ok()
            .and_then(|meta| cfg_holds(&meta))
            != Some(true);
    }
    text.starts_with("#[cfg_attr(") || text.starts_with("#[ignore")
}

/// Whether a `cfg` predicate holds in a test build on this host, when it is
/// built from `all`, `any`, `not`, `test`, `unix`, and the `target_os`,
/// `target_arch`, `target_family`, and `target_env` values. Anything else
/// (features, custom cfgs) is undecidable.
fn cfg_holds(meta: &syn::Meta) -> Option<bool> {
    use syn::punctuated::Punctuated;
    match meta {
        syn::Meta::Path(path) if path.is_ident("test") => Some(true),
        syn::Meta::Path(path) if path.is_ident("unix") => Some(cfg!(unix)),
        syn::Meta::Path(path) if path.is_ident("windows") => Some(cfg!(windows)),
        syn::Meta::NameValue(pair) => {
            let syn::Expr::Lit(syn::ExprLit {
                lit: Lit::Str(value),
                ..
            }) = &pair.value
            else {
                return None;
            };
            let value = value.value();
            let key = pair.path.get_ident()?.to_string();
            match key.as_str() {
                "target_os" => Some(value == std::env::consts::OS),
                "target_arch" => Some(value == std::env::consts::ARCH),
                "target_family" => Some(value == std::env::consts::FAMILY),
                "target_env" => Some(
                    value
                        == if cfg!(target_env = "gnu") {
                            "gnu"
                        } else if cfg!(target_env = "musl") {
                            "musl"
                        } else {
                            ""
                        },
                ),
                _ => None,
            }
        }
        syn::Meta::List(list) => {
            let inner = list
                .parse_args_with(Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
                .ok()?;
            let values = inner.iter().map(cfg_holds).collect::<Vec<_>>();
            match list.path.get_ident()?.to_string().as_str() {
                "all" => {
                    if values.contains(&Some(false)) {
                        Some(false)
                    } else if values.iter().all(|value| *value == Some(true)) {
                        Some(true)
                    } else {
                        None
                    }
                }
                "any" => {
                    if values.contains(&Some(true)) {
                        Some(true)
                    } else if values.iter().all(|value| *value == Some(false)) {
                        Some(false)
                    } else {
                        None
                    }
                }
                "not" if values.len() == 1 => values[0].map(|value| !value),
                _ => None,
            }
        }
        syn::Meta::Path(_) => None,
    }
}

fn disabled(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute_disabled(
            &quote::quote!(#attribute)
                .to_string()
                .replace(' ', "")
                .replacen("#![", "#[", 1),
        )
    })
}

/// Whether the leading attributes of a statement or expression disable it.
fn leading_attributes_disable<T: quote::ToTokens>(node: &T) -> bool {
    use proc_macro2::TokenTree;
    let mut tokens = node.to_token_stream().into_iter();
    while let Some(TokenTree::Punct(punct)) = tokens.next() {
        if punct.as_char() != '#' {
            return false;
        }
        let Some(TokenTree::Group(group)) = tokens.next() else {
            return false;
        };
        if attribute_disabled(&format!("#{group}").replace(' ', "")) {
            return true;
        }
    }
    false
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
        let mut key = self.module.clone();
        key.push(name);
        let facts = self.facts.entry(key).or_default();
        facts.is_test |= is_test(attributes);
        facts.fixture_calls.extend(collector.fixture_calls);
        facts.shadows_consume_fixture |= collector.shadows_consume_fixture;
        facts.references.extend(collector.references);
    }

    /// The items a path written in `module` may name.
    fn resolve(&self, module: &[String], path: &[String]) -> Vec<Vec<String>> {
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

    fn absolute(&self, path: &[String]) -> Vec<String> {
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

    fn use_tree(&mut self, tree: &syn::UseTree, prefix: &mut Vec<String>) {
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

impl<'ast> Visit<'ast> for Items {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        if item.sig.ident == "consume_fixture" && !disabled(&item.attrs) {
            let body = quote::quote!(#item).to_string();
            if body.contains("CARGO_MANIFEST_DIR") && body.contains("join") && body.contains("->") {
                let mut key = self.module.clone();
                key.push("consume_fixture".to_owned());
                self.reviewed_helpers.insert(key);
            }
        }
        self.record(item.sig.ident.to_string(), &item.attrs, |collector| {
            collector.visit_item_fn(item);
        });
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

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if !disabled(&item.attrs) {
            self.use_tree(&item.tree, &mut Vec::new());
        }
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if disabled(&item.attrs) {
            return;
        }
        self.module.push(item.ident.to_string());
        self.scopes.entry(self.module.clone()).or_default();
        syn::visit::visit_item_mod(self, item);
        self.module.pop();
    }
}

/// Collects, from one item, the `consume_fixture("...")` calls whose value is
/// used and the paths it references. Skipped: attributes (doc comments are
/// attributes), nested items, closures, statements and expressions carrying a
/// disabling attribute, statically dead branches (`if false`, `if !true`,
/// `cfg!(any())`, `while false`), code after `return`, and every macro whose
/// arguments are not evaluated in place (only the `assert`, `format`, `print`,
/// and `vec` families are read).
#[derive(Default)]
struct Collector {
    fixture_calls: Vec<(Vec<String>, String)>,
    shadows_consume_fixture: bool,
    references: Vec<Vec<String>>,
}

/// Macros whose arguments are evaluated where they are written.
const EVALUATING_MACROS: [&str; 17] = [
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "print",
    "println",
    "eprint",
    "eprintln",
    "format",
    "write",
    "writeln",
    "vec",
    "panic",
    "unreachable",
    "todo",
];

/// Whether an expression asks about the build configuration (`cfg!(...)`).
fn mentions_cfg(expr: &Expr) -> bool {
    quote::quote!(#expr)
        .to_string()
        .replace(' ', "")
        .contains("cfg!(")
}

/// The leftmost call of a method chain (`f(x).a().b()` starts at `f(x)`).
fn chain_root(mut expr: &Expr) -> &Expr {
    while let Expr::MethodCall(call) = expr {
        expr = &call.receiver;
    }
    expr
}

/// The callee path and literal argument of a `consume_fixture("...")`-shaped
/// call. The path is returned unresolved: whether it actually names a
/// reviewed helper is decided later, once the whole file's items are known
/// (see [`live_test_fixtures`]), never from this final segment alone.
fn consumption(call: &ExprCall) -> Option<(Vec<String>, String)> {
    let Expr::Path(path) = &*call.func else {
        return None;
    };
    if path.path.segments.last()?.ident != "consume_fixture" {
        return None;
    }
    let segments = path
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    match call.args.first() {
        Some(Expr::Lit(literal)) => match &literal.lit {
            Lit::Str(text) => Some((segments, text.value())),
            _ => None,
        },
        _ => None,
    }
}

/// The value of a condition that is statically a boolean.
fn constant(expr: &Expr) -> Option<bool> {
    match expr {
        Expr::Lit(literal) => match &literal.lit {
            Lit::Bool(value) => Some(value.value),
            _ => None,
        },
        Expr::Paren(inner) => constant(&inner.expr),
        Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Not(_)) => {
            constant(&unary.expr).map(|value| !value)
        }
        Expr::Macro(mac) if mac.mac.path.is_ident("cfg") => {
            match mac.mac.tokens.to_string().replace(' ', "").as_str() {
                "any()" => Some(false),
                "all()" => Some(true),
                // `cfg!(test)` is true in a test build; any other predicate
                // depends on the build, so neither branch is relied on.
                "test" => Some(true),
                _ => None,
            }
        }
        _ => None,
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_attribute(&mut self, _: &'ast Attribute) {}

    // An item inside the one being collected is a separate item nothing has
    // called: skipped. A nested `fn consume_fixture` is the one exception
    // worth noticing without descending into it: Rust's own scoping rules
    // mean it shadows any outer same-named helper for every bare,
    // unqualified reference inside this body, reviewed or not.
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if let syn::Item::Fn(nested) = item
            && nested.sig.ident == "consume_fixture"
        {
            self.shadows_consume_fixture = true;
        }
    }

    // A parameter named `consume_fixture` shadows any outer helper of that
    // name for every bare reference in this function's own body, the same
    // as a local binding or a nested fn of that name.
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        for input in &item.sig.inputs {
            if let syn::FnArg::Typed(typed) = input
                && let syn::Pat::Ident(ident) = &*typed.pat
                && ident.ident == "consume_fixture"
            {
                self.shadows_consume_fixture = true;
            }
        }
        syn::visit::visit_item_fn(self, item);
    }

    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}

    fn visit_block(&mut self, block: &'ast syn::Block) {
        for statement in &block.stmts {
            self.visit_stmt(statement);
            if matches!(statement, syn::Stmt::Expr(Expr::Return(_), _)) {
                break;
            }
        }
    }

    fn visit_stmt(&mut self, statement: &'ast syn::Stmt) {
        if leading_attributes_disable(statement) {
            return;
        }
        // A local binding named `consume_fixture` shadows any outer helper
        // of that name for every bare reference in the rest of this body,
        // exactly as a nested fn of that name does.
        if let syn::Stmt::Local(local) = statement
            && let syn::Pat::Ident(ident) = &local.pat
            && ident.ident == "consume_fixture"
        {
            self.shadows_consume_fixture = true;
        }
        // A consumption whose value is dropped on the spot consumes nothing.
        // (an expression statement, a `let _`/`let _name`, or `drop(...)`).
        let dropped = match statement {
            syn::Stmt::Expr(expr, Some(_)) => Some(expr),
            syn::Stmt::Local(local) => {
                let discarded = match &local.pat {
                    syn::Pat::Wild(_) => true,
                    syn::Pat::Ident(ident) => ident.ident.to_string().starts_with('_'),
                    _ => false,
                };
                discarded
                    .then(|| local.init.as_ref().map(|init| &*init.expr))
                    .flatten()
            }
            _ => None,
        };
        if let Some(expr) = dropped {
            let root = match chain_root(expr) {
                Expr::Call(call) if quote::quote!(#call).to_string().starts_with("drop (") => {
                    match call.args.first() {
                        Some(Expr::Call(inner)) => Some(inner),
                        _ => None,
                    }
                }
                Expr::Call(call) => Some(call),
                _ => None,
            };
            if let Some(call) = root
                && consumption(call).is_some()
            {
                for argument in &call.args {
                    self.visit_expr(argument);
                }
                return;
            }
        }
        syn::visit::visit_stmt(self, statement);
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        if !leading_attributes_disable(expr) {
            syn::visit::visit_expr(self, expr);
        }
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        self.references.push(
            path.path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect(),
        );
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Some(fixture_call) = consumption(call) {
            self.fixture_calls.push(fixture_call);
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_macro(&mut self, node: &'ast Macro) {
        use syn::punctuated::Punctuated;
        let evaluating = node
            .path
            .get_ident()
            .is_some_and(|ident| EVALUATING_MACROS.contains(&ident.to_string().as_str()));
        if evaluating
            && let Ok(arguments) =
                node.parse_body_with(Punctuated::<Expr, syn::Token![,]>::parse_terminated)
        {
            for argument in &arguments {
                self.visit_expr(argument);
            }
        }
    }

    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        match constant(&node.cond) {
            Some(false) => {
                if let Some((_, otherwise)) = &node.else_branch {
                    self.visit_expr(otherwise);
                }
            }
            Some(true) => self.visit_block(&node.then_branch),
            None if mentions_cfg(&node.cond) => {}
            None => syn::visit::visit_expr_if(self, node),
        }
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        if constant(&node.cond) != Some(false) {
            syn::visit::visit_expr_while(self, node);
        }
    }
}
