// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Every name a pattern, a `use` item, or a local item locally binds,
//! shadowing an outer item of the same name for the rest of the enclosing
//! scope (see `fixture_consumption`).

/// Every identifier `pattern` binds: a bare identifier, or one nested inside
/// a tuple, tuple struct, struct, slice, reference, parenthesized, typed, or
/// or-pattern. Every one of these compiles as a real local binding that
/// shadows an outer item of the same name for the rest of the enclosing
/// scope.
pub(super) fn pattern_bound_names(pattern: &syn::Pat, names: &mut Vec<String>) {
    match pattern {
        syn::Pat::Ident(ident) => {
            names.push(ident.ident.to_string());
            // An `@`-pattern (`whole @ Some(consume_fixture)`) binds BOTH
            // the outer name and whatever its own subpattern binds.
            if let Some((_, subpat)) = &ident.subpat {
                pattern_bound_names(subpat, names);
            }
        }
        syn::Pat::Tuple(tuple) => {
            for element in &tuple.elems {
                pattern_bound_names(element, names);
            }
        }
        syn::Pat::TupleStruct(tuple_struct) => {
            for element in &tuple_struct.elems {
                pattern_bound_names(element, names);
            }
        }
        syn::Pat::Struct(structure) => {
            for field in &structure.fields {
                pattern_bound_names(&field.pat, names);
            }
        }
        syn::Pat::Slice(slice) => {
            for element in &slice.elems {
                pattern_bound_names(element, names);
            }
        }
        syn::Pat::Reference(reference) => pattern_bound_names(&reference.pat, names),
        syn::Pat::Paren(paren) => pattern_bound_names(&paren.pat, names),
        syn::Pat::Type(typed) => pattern_bound_names(&typed.pat, names),
        syn::Pat::Or(or) => {
            for case in &or.cases {
                pattern_bound_names(case, names);
            }
        }
        _ => {}
    }
}

/// Every name `tree` (a `use` item's tree) brings into scope: its own name
/// (`use a::b;` binds `b`) or a rename (`use a::b as c;` binds `c`). A glob
/// (`use a::*;`) cannot be decided statically without knowing `a`'s own
/// exports, so it binds nothing here; analysis elsewhere in this file never
/// trusts a bare, unqualified reference across a module boundary regardless.
pub(super) fn use_tree_bound_names(tree: &syn::UseTree, names: &mut Vec<String>) {
    match tree {
        syn::UseTree::Name(name) => names.push(name.ident.to_string()),
        syn::UseTree::Rename(rename) => names.push(rename.rename.to_string()),
        syn::UseTree::Path(path) => use_tree_bound_names(&path.tree, names),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                use_tree_bound_names(item, names);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// Every value-namespace name a local item (one written inside a function
/// body) binds: a nested `fn`, `const`, `static`, a tuple struct's own
/// callable constructor, or a `use`. Real Rust hoists a local item's name
/// across its whole enclosing block, unlike a `let` binding, which only
/// shadows code written after it.
pub(super) fn item_bound_names(item: &syn::Item, names: &mut Vec<String>) {
    match item {
        syn::Item::Fn(item) => names.push(item.sig.ident.to_string()),
        syn::Item::Const(item) => names.push(item.ident.to_string()),
        syn::Item::Static(item) => names.push(item.ident.to_string()),
        // A struct with named fields or no fields is not callable and binds
        // no value-namespace name a call-position reference could shadow.
        syn::Item::Struct(item) if matches!(item.fields, syn::Fields::Unnamed(_)) => {
            names.push(item.ident.to_string())
        }
        syn::Item::Use(item) => use_tree_bound_names(&item.tree, names),
        _ => {}
    }
}
