// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Fingerprint compaction over proc-macro token boundaries.

use proc_macro2::{Delimiter, TokenStream, TokenTree};

pub(super) fn compact_tokens(tokens: &TokenStream) -> String {
    let mut compacted = String::new();
    compact_into(tokens, &mut compacted);
    compacted
}

fn compact_into(tokens: &TokenStream, out: &mut String) {
    for tree in tokens.clone() {
        match tree {
            TokenTree::Group(group) => match group.delimiter() {
                Delimiter::None => compact_into(&group.stream(), out),
                Delimiter::Parenthesis => {
                    out.push('(');
                    compact_into(&group.stream(), out);
                    out.push(')');
                }
                Delimiter::Brace => {
                    out.push('{');
                    compact_into(&group.stream(), out);
                    out.push('}');
                }
                Delimiter::Bracket => {
                    out.push('[');
                    compact_into(&group.stream(), out);
                    out.push(']');
                }
            },
            TokenTree::Ident(ident) => out.push_str(&ident.to_string()),
            TokenTree::Punct(punct) => out.push(punct.as_char()),
            TokenTree::Literal(literal) => out.push_str(&compact_literal(&literal)),
        }
    }
}

fn compact_literal(literal: &proc_macro2::Literal) -> String {
    // A cooked string's line continuation is a backslash and a newline inside
    // that one literal. Token display keeps the backslash. Drop the pair so
    // it is not glued to the next character as a path separator. An escaped
    // backslash stays. Every other literal keeps its backslashes; whitespace
    // still drops, as it does between tokens.
    compact_spelling(&literal.to_string(), cooked_string(literal))
}

fn cooked_string(literal: &proc_macro2::Literal) -> bool {
    let stream = TokenStream::from(TokenTree::Literal(literal.clone()));
    let Ok(parsed) = syn::parse2::<syn::Lit>(stream) else {
        return false;
    };
    match parsed {
        syn::Lit::Str(_) | syn::Lit::ByteStr(_) | syn::Lit::CStr(_) => {
            !literal_is_raw(&literal.to_string())
        }
        _ => false,
    }
}

/// The literal token's own prefix. This is not a scan of the surrounding code.
fn literal_is_raw(spelling: &str) -> bool {
    let bytes = spelling.as_bytes();
    let rest = match bytes.first().copied() {
        Some(b'b' | b'c') if matches!(bytes.get(1).copied(), Some(b'r' | b'"')) => &spelling[1..],
        _ => spelling,
    };
    rest.starts_with('r')
}

fn compact_spelling(spelling: &str, drop_continuation: bool) -> String {
    let mut out = String::with_capacity(spelling.len());
    let mut chars = spelling.chars().peekable();
    while let Some(ch) = chars.next() {
        if drop_continuation && ch == '\\' {
            // A continuation newline is LF, CR, or CR LF. The Rust lexer
            // accepts all three, and literal display keeps that spelling.
            match chars.peek().copied() {
                Some('\n') => {
                    chars.next();
                }
                Some('\r') => {
                    chars.next();
                    if chars.peek().copied() == Some('\n') {
                        chars.next();
                    }
                }
                Some(next) => {
                    out.push('\\');
                    chars.next();
                    if !next.is_whitespace() {
                        out.push(next);
                    }
                }
                None => out.push('\\'),
            }
            continue;
        }
        if !ch.is_whitespace() {
            out.push(ch);
        }
    }
    out
}
