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

#[path = "fixture_consumption_shadow.rs"]
mod fixture_consumption_shadow;
use fixture_consumption_shadow::{item_bound_names, pattern_bound_names};

#[path = "fixture_consumption_cfg.rs"]
mod fixture_consumption_cfg;
use fixture_consumption_cfg::attribute_disabled;

/// The fixtures a normal `cargo test` run consumes: the first argument of
/// every `consume_fixture("...")` call in a live `#[test]` function or in a
/// live function, constant, or static such a function reaches by qualified
/// path (resolved through the module tree, `use` items, and glob imports;
/// never by bare identifier across modules) and actually invokes -- a
/// reference to an item as a value, an unawaited call to an `async fn`, or
/// an unpolled `async` block never counts.
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
        for (reference, awaited) in &facts.references {
            for target in items.resolve(module, reference) {
                // An `async fn`'s body never runs until its call is polled;
                // a call this collector did not see directly `.await`ed
                // never reaches it.
                let unpolled = !awaited
                    && items
                        .facts
                        .get(&target)
                        .is_some_and(|target_facts| target_facts.is_async);
                if !unpolled {
                    pending.push(target);
                }
            }
        }
    }
    let mut consumed = Vec::new();
    for key in &reached {
        let Some(facts) = items.facts.get(key) else {
            continue;
        };
        let module = &key[..key.len() - 1];
        for (path, fixture) in &facts.fixture_calls {
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
    /// Whether this item is an `async fn`: its body never runs until a call
    /// to it is polled, so a reference this collector did not see directly
    /// `.await`ed must never credit it as reached.
    is_async: bool,
    /// Every syntactically `consume_fixture("...")`-shaped call this item's
    /// body makes, as its own (unresolved) callee path and the literal
    /// argument. Resolved against [`Items::reviewed_helpers`] once the whole
    /// file is known (see [`live_test_fixtures`]), never credited from the
    /// name alone: a same-named decoy elsewhere in the file, reached only
    /// because its own final path segment matches, proves nothing. A call
    /// whose bare callee name resolves to a local shadow (a nested item, a
    /// `let`/parameter/pattern binding of that same name, in scope at the
    /// call site) is never recorded here at all, regardless of what that
    /// name would otherwise resolve to.
    fixture_calls: Vec<(Vec<String>, String)>,
    /// Every call this item's body actually makes (the callee's own
    /// unresolved path, and whether the call was directly `.await`ed),
    /// subject to the same local-shadow exclusion as `fixture_calls`.
    references: Vec<(Vec<String>, bool)>,
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

#[path = "fixture_consumption_resolve.rs"]
mod fixture_consumption_resolve;

impl<'ast> Visit<'ast> for Items {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        if item.sig.ident == "consume_fixture" && !disabled(&item.attrs) {
            // The signature and body only: `item.attrs` (doc comments and
            // other attributes are attributes) must never be able to forge
            // this shape, or `#[doc = "CARGO_MANIFEST_DIR join ->"] fn
            // consume_fixture(_: &str) -> u8 { 0 }` would pass as reviewed.
            let signature = &item.sig;
            let block = &item.block;
            let body = quote::quote!(#signature #block).to_string();
            if body.contains("CARGO_MANIFEST_DIR") && body.contains("join") && body.contains("->") {
                let mut key = self.module.clone();
                key.push("consume_fixture".to_owned());
                self.reviewed_helpers.insert(key);
            }
        }
        let is_async = item.sig.asyncness.is_some();
        self.record(
            item.sig.ident.to_string(),
            &item.attrs,
            is_async,
            |collector| {
                collector.visit_item_fn(item);
            },
        );
    }

    fn visit_item_const(&mut self, item: &'ast ItemConst) {
        self.record(item.ident.to_string(), &item.attrs, false, |collector| {
            collector.visit_item_const(item);
        });
    }

    fn visit_item_static(&mut self, item: &'ast ItemStatic) {
        self.record(item.ident.to_string(), &item.attrs, false, |collector| {
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
/// used and the calls it actually makes. Skipped: attributes (doc comments
/// are attributes), nested items (their own names are still hoisted into
/// scope, see [`Collector::scopes`]), closures, `async` blocks (never polled
/// here), statements and expressions carrying a disabling attribute,
/// statically dead branches (`if false`, `if !true`, `cfg!(any())`, `while
/// false`), any code that can never run because everything reachable before
/// it in the same block always diverges (a `return`, or nested blocks/`if`
/// that always return), and every macro whose arguments are not evaluated in
/// place (only the `assert`, `format`, `print`, and `vec` families are
/// read).
#[derive(Default)]
struct Collector {
    fixture_calls: Vec<(Vec<String>, String)>,
    references: Vec<(Vec<String>, bool)>,
    /// A stack of lexical scopes, innermost last, each holding the names it
    /// locally binds (an item hoisted across its whole enclosing block, a
    /// function parameter, or a `let`/`for`/`match`-arm/`if let`/`while
    /// let` pattern). A bare, single-segment call whose name appears in any
    /// active frame binds to that local shadow, per Rust's own scoping
    /// rules, never to any outer, same-named item -- regardless of what
    /// module-tree resolution would otherwise find.
    scopes: Vec<Vec<String>>,
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

/// Whether `stmt` always diverges: nothing textually after it in the same
/// block can ever run.
fn diverges(stmt: &syn::Stmt) -> bool {
    match stmt {
        syn::Stmt::Expr(expr, _) => expr_diverges(expr),
        _ => false,
    }
}

/// Whether evaluating `expr` always diverges: a bare `return`, a nested
/// block whose own contents always diverge, or an `if`/`else` where the
/// branch a statically decidable condition takes (or, when undecidable,
/// every branch) always diverges.
fn expr_diverges(expr: &Expr) -> bool {
    match expr {
        Expr::Return(_) => true,
        Expr::Block(block) => block_diverges(&block.block),
        Expr::If(if_expr) => match constant(&if_expr.cond) {
            Some(true) => block_diverges(&if_expr.then_branch),
            Some(false) => if_expr
                .else_branch
                .as_ref()
                .is_some_and(|(_, otherwise)| expr_diverges(otherwise)),
            None => {
                block_diverges(&if_expr.then_branch)
                    && if_expr
                        .else_branch
                        .as_ref()
                        .is_some_and(|(_, otherwise)| expr_diverges(otherwise))
            }
        },
        _ => false,
    }
}

fn block_diverges(block: &syn::Block) -> bool {
    block.stmts.iter().any(diverges)
}

impl Collector {
    fn is_shadowed(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .any(|frame| frame.iter().any(|bound| bound == name))
    }

    /// Records `call` as an actual invocation (`awaited` says whether this
    /// collector saw it directly `.await`ed): a bare, single-segment callee
    /// currently shadowed by a local binding of that name is never
    /// recorded at all, since it can bind only to that local decoy, never
    /// to any outer item -- reviewed helper or otherwise.
    fn record_call(&mut self, call: &ExprCall, awaited: bool) {
        let Expr::Path(path) = &*call.func else {
            return;
        };
        let segments = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if segments.len() == 1 && self.is_shadowed(&segments[0]) {
            return;
        }
        if let Some(fixture_call) = consumption(call) {
            self.fixture_calls.push(fixture_call);
        }
        self.references.push((segments, awaited));
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_attribute(&mut self, _: &'ast Attribute) {}

    // A local item's own name is hoisted into the enclosing block's scope
    // by `visit_block` below, before any of the block's statements are
    // visited (matching Rust's own item-hoisting rule); this collector
    // never descends into the item's own separate, unreached body.
    fn visit_item(&mut self, _: &'ast syn::Item) {}

    // A parameter (or a pattern destructuring one) locally binds for the
    // whole function body, exactly as a nested item or a `let` does.
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        let mut names = Vec::new();
        for input in &item.sig.inputs {
            if let syn::FnArg::Typed(typed) = input {
                pattern_bound_names(&typed.pat, &mut names);
            }
        }
        self.scopes.push(names);
        syn::visit::visit_item_fn(self, item);
        self.scopes.pop();
    }

    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}

    // An `async` block's contents never run until it is polled; nothing
    // here proves that ever happens (an immediately `.await`ed async block
    // is a legal but unusual pattern this analysis does not special-case,
    // the same conservative stance it already takes for a closure that is
    // called immediately at its own definition site).
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let mut hoisted = Vec::new();
        for statement in &block.stmts {
            if let syn::Stmt::Item(item) = statement {
                item_bound_names(item, &mut hoisted);
            }
        }
        self.scopes.push(hoisted);
        for statement in &block.stmts {
            self.visit_stmt(statement);
            if diverges(statement) {
                break;
            }
        }
        self.scopes.pop();
    }

    fn visit_stmt(&mut self, statement: &'ast syn::Stmt) {
        if leading_attributes_disable(statement) {
            return;
        }
        // A local binding shadows any outer item of the same name for the
        // rest of this block, including one bound through a destructuring
        // pattern (a tuple, a struct, a slice, an or-pattern, ...), not
        // only a bare identifier.
        if let syn::Stmt::Local(local) = statement {
            let mut names = Vec::new();
            pattern_bound_names(&local.pat, &mut names);
            if let Some(frame) = self.scopes.last_mut() {
                frame.extend(names);
            }
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

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        self.record_call(call, false);
        syn::visit::visit_expr_call(self, call);
    }

    // A call that is the direct base of `.await` is a real invocation of an
    // `async fn`'s body (or a poll of an `async` block, though that is
    // never traversed into regardless -- see `visit_expr_async`); anything
    // else awaited is not itself a call and is walked normally.
    fn visit_expr_await(&mut self, expr: &'ast syn::ExprAwait) {
        if leading_attributes_disable(expr) {
            return;
        }
        if let Expr::Call(call) = &*expr.base {
            self.record_call(call, true);
            for argument in &call.args {
                self.visit_expr(argument);
            }
        } else {
            self.visit_expr(&expr.base);
        }
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
                return;
            }
            Some(true) => {
                self.visit_block(&node.then_branch);
                return;
            }
            None if mentions_cfg(&node.cond) => return,
            None => {}
        }
        self.visit_expr(&node.cond);
        // An `if let PAT = ...` binds PAT for the then-branch only (an
        // `Expr::Let` reached here as `node.cond`, never visited generically
        // since this override does not delegate to the default `if`
        // traversal).
        let mut names = Vec::new();
        if let Expr::Let(let_expr) = &*node.cond {
            pattern_bound_names(&let_expr.pat, &mut names);
        }
        self.scopes.push(names);
        self.visit_block(&node.then_branch);
        self.scopes.pop();
        if let Some((_, otherwise)) = &node.else_branch {
            self.visit_expr(otherwise);
        }
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        if constant(&node.cond) == Some(false) {
            return;
        }
        self.visit_expr(&node.cond);
        // A `while let PAT = ...` binds PAT for the body only, the same as
        // `if let` above.
        let mut names = Vec::new();
        if let Expr::Let(let_expr) = &*node.cond {
            pattern_bound_names(&let_expr.pat, &mut names);
        }
        self.scopes.push(names);
        self.visit_block(&node.body);
        self.scopes.pop();
    }

    // A `for` loop's own pattern binds for its body only.
    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        let mut names = Vec::new();
        pattern_bound_names(&node.pat, &mut names);
        self.scopes.push(names);
        self.visit_expr(&node.expr);
        self.visit_block(&node.body);
        self.scopes.pop();
    }

    // A `match` arm's own pattern binds for its body only.
    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        let mut names = Vec::new();
        pattern_bound_names(&arm.pat, &mut names);
        self.scopes.push(names);
        syn::visit::visit_arm(self, arm);
        self.scopes.pop();
    }
}
