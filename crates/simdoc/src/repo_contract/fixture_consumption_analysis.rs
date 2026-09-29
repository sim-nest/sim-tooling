// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pure syntactic predicates used while collecting one item's own facts:
//! recognizing a `consume_fixture("...")`-shaped call, folding a
//! statically decidable condition, and deciding whether a statement always
//! diverges (see `fixture_consumption`).

use syn::{Expr, ExprCall, Lit};

pub(super) fn mentions_cfg(expr: &Expr) -> bool {
    quote::quote!(#expr)
        .to_string()
        .replace(' ', "")
        .contains("cfg!(")
}

/// The leftmost call of a method chain (`f(x).a().b()` starts at `f(x)`).
pub(super) fn chain_root(mut expr: &Expr) -> &Expr {
    while let Expr::MethodCall(call) = expr {
        expr = &call.receiver;
    }
    expr
}

/// The callee path and literal argument of a `consume_fixture("...")`-shaped
/// call. The path is returned unresolved: whether it actually names a
/// reviewed helper is decided later, once the whole file's items are known
/// (see [`live_test_fixtures`]), never from this final segment alone.
pub(super) fn consumption(call: &ExprCall) -> Option<(Vec<String>, String)> {
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
pub(super) fn constant(expr: &Expr) -> Option<bool> {
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
pub(super) fn diverges(stmt: &syn::Stmt) -> bool {
    match stmt {
        syn::Stmt::Expr(expr, _) => expr_diverges(expr),
        // `let _: () = return;` diverges exactly as a bare `return;`
        // statement does; the let-else `diverge` block (`let Some(x) = y
        // else { return; };`) is not checked here, since a successful
        // match is the normal, non-diverging case for the statement itself.
        syn::Stmt::Local(local) => local
            .init
            .as_ref()
            .is_some_and(|init| expr_diverges(&init.expr)),
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

/// What `Collector::visit_condition` learned about a condition it just
/// visited: it always holds (`Decided(true)`), never holds
/// (`Decided(false)`), cannot be decided from ordinary control flow
/// (`Maybe`), or mentions a `cfg!` predicate this file cannot evaluate for
/// every host (`CfgUnknown`) -- in which case the code it guards must never
/// be assumed either reachable or unreachable, so neither branch is
/// visited at all, the same conservative stance this file has always taken
/// for `cfg!`.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Certainty {
    Decided(bool),
    Maybe,
    CfgUnknown,
}
