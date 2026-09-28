// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Whether a pattern or a `use` item locally binds the name
//! `consume_fixture`, shadowing an outer helper of that name for the rest
//! of the enclosing body (see `fixture_consumption`).

/// Whether `pattern` binds the identifier `consume_fixture` anywhere within
/// it: a bare identifier, or that identifier nested inside a tuple, tuple
/// struct, struct, slice, reference, parenthesized, typed, or or-pattern.
/// Every one of these compiles as a real local binding that shadows an
/// outer item of the same name for the rest of the enclosing body.
pub(super) fn pattern_binds_consume_fixture(pattern: &syn::Pat) -> bool {
    match pattern {
        syn::Pat::Ident(ident) => ident.ident == "consume_fixture",
        syn::Pat::Tuple(tuple) => tuple.elems.iter().any(pattern_binds_consume_fixture),
        syn::Pat::TupleStruct(tuple_struct) => {
            tuple_struct.elems.iter().any(pattern_binds_consume_fixture)
        }
        syn::Pat::Struct(structure) => structure
            .fields
            .iter()
            .any(|field| pattern_binds_consume_fixture(&field.pat)),
        syn::Pat::Slice(slice) => slice.elems.iter().any(pattern_binds_consume_fixture),
        syn::Pat::Reference(reference) => pattern_binds_consume_fixture(&reference.pat),
        syn::Pat::Paren(paren) => pattern_binds_consume_fixture(&paren.pat),
        syn::Pat::Type(typed) => pattern_binds_consume_fixture(&typed.pat),
        syn::Pat::Or(or) => or.cases.iter().any(pattern_binds_consume_fixture),
        _ => false,
    }
}

/// Whether `tree` (a `use` item's tree) brings something into scope under
/// the name `consume_fixture`, whether that is its own name (`use
/// a::consume_fixture;`) or a rename onto it (`use a::b as
/// consume_fixture;`). A glob (`use a::*;`) cannot be decided statically
/// without knowing `a`'s own exports, so it is not treated as a shadow;
/// analysis elsewhere in this file never trusts a bare, unqualified
/// reference across a module boundary regardless.
pub(super) fn use_tree_binds_consume_fixture(tree: &syn::UseTree) -> bool {
    match tree {
        syn::UseTree::Name(name) => name.ident == "consume_fixture",
        syn::UseTree::Rename(rename) => rename.rename == "consume_fixture",
        syn::UseTree::Path(path) => use_tree_binds_consume_fixture(&path.tree),
        syn::UseTree::Group(group) => group.items.iter().any(use_tree_binds_consume_fixture),
        syn::UseTree::Glob(_) => false,
    }
}
