// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which fixtures a normal `cargo test` run consumes, read from a test
//! target's source (see [`super::exclusion_witness`]).

use std::collections::{BTreeMap, BTreeSet};

use syn::{Attribute, Expr, ExprCall, ExprIf, ItemConst, ItemFn, ItemStatic, Macro, visit::Visit};

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

#[path = "fixture_consumption_analysis.rs"]
mod fixture_consumption_analysis;
use fixture_consumption_analysis::{
    Certainty, chain_root, constant, consumption, diverges, mentions_cfg,
};

impl Collector {
    fn is_shadowed(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .any(|frame| frame.iter().any(|bound| bound == name))
    }

    /// Records `call` as an actual invocation (`awaited` says whether this
    /// collector saw it directly `.await`ed): a callee whose own FIRST path
    /// segment is currently shadowed by a local binding of that name is
    /// never recorded at all. Rust's own module resolution starts from that
    /// first segment too, so once it is a local value the rest of the path
    /// resolves inside whatever that local names -- never the outer, real
    /// item this (module-tree-only) analysis would otherwise walk to, which
    /// has no way to know about a local's own contents.
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
        if segments
            .first()
            .is_some_and(|first| self.is_shadowed(first))
        {
            return;
        }
        if let Some(fixture_call) = consumption(call) {
            self.fixture_calls.push(fixture_call);
        }
        self.references.push((segments, awaited));
    }

    /// Visits an `if`/`while` condition as a left-to-right chain of
    /// `&&`/`||`-joined conjuncts (a plain condition is a chain of one):
    /// a `let PAT = EXPR` conjunct visits `EXPR` in the scope active so
    /// far, then pushes `PAT`'s own bound names, so a LATER conjunct in the
    /// same chain -- and the body this condition guards -- sees them (real
    /// Rust let-chain scoping); a statically decided conjunct short-circuits
    /// the rest of its own `&&`/`||` exactly as the real operator would,
    /// so a conjunct that can never actually run is never visited. Returns
    /// how many scope frames were pushed (for the caller to pop once it is
    /// done with whatever this condition guards) and, when the WHOLE chain
    /// is statically decided, its value.
    fn visit_condition(&mut self, cond: &Expr) -> (usize, Certainty) {
        if let Expr::Binary(binary) = cond
            && let syn::BinOp::And(_) | syn::BinOp::Or(_) = binary.op
        {
            let is_and = matches!(binary.op, syn::BinOp::And(_));
            let (left_frames, left) = self.visit_condition(&binary.left);
            // `&&` short-circuits once the left side is false; `||` once
            // it is true -- the right side is never evaluated, so never
            // visited either.
            let short_circuit = Certainty::Decided(!is_and);
            if left == short_circuit {
                return (left_frames, left);
            }
            let (right_frames, right) = self.visit_condition(&binary.right);
            let decided = match (left, right) {
                (Certainty::Decided(left), Certainty::Decided(right)) => {
                    Certainty::Decided(if is_and { left && right } else { left || right })
                }
                (Certainty::CfgUnknown, _) | (_, Certainty::CfgUnknown) => Certainty::CfgUnknown,
                _ => Certainty::Maybe,
            };
            return (left_frames + right_frames, decided);
        }
        if let Expr::Let(let_expr) = cond {
            self.visit_expr(&let_expr.expr);
            let mut names = Vec::new();
            pattern_bound_names(&let_expr.pat, &mut names);
            self.scopes.push(names);
            return (1, Certainty::Maybe);
        }
        match constant(cond) {
            Some(value) => (0, Certainty::Decided(value)),
            None if mentions_cfg(cond) => (0, Certainty::CfgUnknown),
            None => {
                self.visit_expr(cond);
                (0, Certainty::Maybe)
            }
        }
    }

    /// Visits `expr`, a value known to be dropped as a whole (or nested
    /// inside one that is): a direct `consume_fixture("...")`-shaped call is
    /// visited for its own arguments only, never credited (its own return
    /// value is thrown away); a tuple, array, or parenthesized expression is
    /// unwrapped one layer and each part treated the same way in turn (the
    /// whole aggregate being dropped drops every element with it); anything
    /// else is visited normally -- it may still execute for real and reach
    /// further code, even though this particular value ends up discarded.
    fn visit_dropped(&mut self, expr: &Expr) {
        match expr {
            Expr::Tuple(tuple) => {
                for element in &tuple.elems {
                    self.visit_dropped(element);
                }
            }
            Expr::Array(array) => {
                for element in &array.elems {
                    self.visit_dropped(element);
                }
            }
            Expr::Paren(paren) => self.visit_dropped(&paren.expr),
            _ => {
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
                match root {
                    Some(call) if consumption(call).is_some() => {
                        for argument in &call.args {
                            self.visit_expr(argument);
                        }
                    }
                    _ => self.visit_expr(expr),
                }
            }
        }
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
            self.visit_dropped(expr);
        } else {
            syn::visit::visit_stmt(self, statement);
        }
        // A local binding shadows any outer item of the same name for the
        // rest of this block, including one bound through a destructuring
        // pattern (a tuple, a struct, a slice, an or-pattern, ...), not
        // only a bare identifier -- pushed only now, after the statement's
        // own initializer has already been visited: Rust resolves a `let`
        // binding's own right-hand side in the OUTER scope, so `let
        // consume_fixture = consume_fixture("x");` must not shadow the call
        // on its own right-hand side.
        if let syn::Stmt::Local(local) = statement {
            let mut names = Vec::new();
            pattern_bound_names(&local.pat, &mut names);
            if let Some(frame) = self.scopes.last_mut() {
                frame.extend(names);
            }
        }
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
        let Some(name) = node.path.get_ident().map(ToString::to_string) else {
            return;
        };
        if !EVALUATING_MACROS.contains(&name.as_str()) {
            return;
        }
        let Ok(arguments) =
            node.parse_body_with(Punctuated::<Expr, syn::Token![,]>::parse_terminated)
        else {
            return;
        };
        // `assert!`/`debug_assert!` only ever evaluate their condition;
        // `assert_eq!`/`assert_ne!`/`debug_assert_eq!`/`debug_assert_ne!`
        // only their two compared values -- every later argument is the
        // panic message, evaluated only if the assertion actually fails.
        let eager = match name.as_str() {
            "assert" | "debug_assert" => 1,
            "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne" => 2,
            _ => arguments.len(),
        };
        for argument in arguments.iter().take(eager) {
            self.visit_expr(argument);
        }
    }

    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        let (frames, certainty) = self.visit_condition(&node.cond);
        match certainty {
            Certainty::CfgUnknown => {}
            Certainty::Decided(true) => self.visit_block(&node.then_branch),
            Certainty::Decided(false) => {
                if let Some((_, otherwise)) = &node.else_branch {
                    self.visit_expr(otherwise);
                }
            }
            Certainty::Maybe => {
                self.visit_block(&node.then_branch);
                if let Some((_, otherwise)) = &node.else_branch {
                    self.visit_expr(otherwise);
                }
            }
        }
        for _ in 0..frames {
            self.scopes.pop();
        }
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        let (frames, certainty) = self.visit_condition(&node.cond);
        if !matches!(certainty, Certainty::Decided(false) | Certainty::CfgUnknown) {
            self.visit_block(&node.body);
        }
        for _ in 0..frames {
            self.scopes.pop();
        }
    }

    // A `for` loop's own pattern binds for its body only, and never for its
    // own iterator expression: `for consume_fixture in real_iterator() {}`
    // must evaluate `real_iterator()` in the OUTER scope first, exactly as
    // a `let` binding's own initializer is.
    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.visit_expr(&node.expr);
        let mut names = Vec::new();
        pattern_bound_names(&node.pat, &mut names);
        self.scopes.push(names);
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
