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
    // backslash stays. Every other literal keeps its backslashes. Whitespace
    // still drops, as it does between tokens, except one space in a run whose
    // deletion would add an unreviewed absolute path. That space stops a
    // fingerprint from inventing a path the literal did not write. A reviewed
    // public literal stays fully stripped, so an admitted spelling keeps its
    // fingerprint.
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
    let stripped = spell(spelling, drop_continuation, true);
    if !may_hold_path(&stripped) || !spelling.chars().any(char::is_whitespace) {
        return stripped;
    }
    let spaced = spell(spelling, drop_continuation, false);
    let already = private_paths(&spaced);
    if !adds_private_path(&stripped, &already) {
        return stripped;
    }
    drop_safe_whitespace(&spaced, &already)
}

fn spell(spelling: &str, drop_continuation: bool, drop_whitespace: bool) -> String {
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
        if !drop_whitespace || !ch.is_whitespace() {
            out.push(ch);
        }
    }
    out
}

fn may_hold_path(text: &str) -> bool {
    text.contains(['/', '\\', '~', '$'])
}

fn private_paths(text: &str) -> Vec<String> {
    crate::publication::path_tokens(text)
        .into_iter()
        .filter(|token| !crate::publication::is_public_path(token))
        .map(str::to_owned)
        .collect()
}

fn adds_private_path(text: &str, already: &[String]) -> bool {
    crate::publication::path_tokens(text)
        .into_iter()
        .any(|token| {
            !crate::publication::is_public_path(token) && !already.iter().any(|kept| kept == token)
        })
}

/// Drop each whitespace run that does not add a private path. A run that
/// would add one stays as a single space, which is enough to keep the slash
/// segments apart. A later deletion can turn a blocked join into a reviewed
/// literal, so the scan repeats until a pass drops nothing more.
fn drop_safe_whitespace(spaced: &str, already: &[String]) -> String {
    let mut current = spaced.to_owned();
    loop {
        let next = drop_one_pass(&current, already);
        if next == current {
            return current;
        }
        current = next;
    }
}

fn drop_one_pass(current: &str, already: &[String]) -> String {
    let mut out = current.to_owned();
    let mut from = 0;
    while let Some((start, end)) = next_whitespace_run(&out, from) {
        let mut trial = String::with_capacity(out.len() - (end - start));
        trial.push_str(&out[..start]);
        trial.push_str(&out[end..]);
        if adds_private_path(&trial, already) {
            if end - start != 1 || !out[start..end].starts_with(' ') {
                out.replace_range(start..end, " ");
            }
            from = start + 1;
        } else {
            out = trial;
            from = start;
        }
    }
    out
}

fn next_whitespace_run(text: &str, from: usize) -> Option<(usize, usize)> {
    let tail = text.get(from..)?;
    let start_rel = tail.find(char::is_whitespace)?;
    let start = from + start_rel;
    let rest = &text[start..];
    let end = start
        + rest
            .find(|ch: char| !ch.is_whitespace())
            .unwrap_or(rest.len());
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use quote::ToTokens;

    use super::compact_tokens;
    use crate::publication::{guard, path_tokens};

    fn compact_src(src: &str) -> String {
        let parsed = syn::parse_file(src).expect("sample");
        compact_tokens(&ToTokens::to_token_stream(&parsed))
    }

    #[test]
    fn prose_slash_words_do_not_become_an_absolute_path() {
        let src = r#"
            fn demo() {
                let _ = "auto parts request: catalog/get path | order/place supplier lines; live supplier requires auto/order and net/http";
            }
        "#;
        let compacted = compact_src(src);
        assert!(
            compacted.contains("autopartsrequest:catalog/getpath"),
            "{compacted}"
        );
        assert!(compacted.contains("orderand net/http"), "{compacted}");
        assert!(!compacted.contains("/orderandnet/http"), "{compacted}");
        assert!(
            path_tokens(&compacted).is_empty(),
            "{compacted} {:?}",
            path_tokens(&compacted)
        );
        guard(
            Path::new("/work/constellation/sim-fixture"),
            "sim-index-fragment.claims.sx",
            &compacted,
        )
        .unwrap_or_else(|err| panic!("{err}\n{compacted}"));
    }

    #[test]
    fn exact_paths_keep_their_publication_judgment() {
        let repo = Path::new("/work/constellation/sim-fixture");
        for src in [
            r#"fn demo() { let _ = "/tmp/db"; }"#,
            r#"fn demo() { let _ = "/unreviewed/route"; }"#,
            r#"fn demo() { let _ = "C:\\Users\\a"; }"#,
        ] {
            let compacted = compact_src(src);
            assert!(
                guard(repo, "README.md", &compacted).is_err(),
                "{src} => {compacted}"
            );
        }
        let admitted = compact_src(r#"fn demo() { let _ = "/proc/cpuinfo"; }"#);
        guard(repo, "README.md", &admitted).unwrap_or_else(|err| panic!("{err}\n{admitted}"));
    }

    #[test]
    fn joining_a_machine_path_across_a_space_stays_unpublished() {
        let compacted = compact_src(r#"fn demo() { let _ = "; /tmp /db"; }"#);
        assert!(
            !compacted.contains("/tmp/db"),
            "{compacted} {:?}",
            path_tokens(&compacted)
        );
        guard(
            Path::new("/work/constellation/sim-fixture"),
            "README.md",
            &compacted,
        )
        .unwrap_or_else(|err| panic!("{err}\n{compacted}"));
    }

    #[test]
    fn an_admitted_literal_joined_across_a_space_stays_fully_stripped() {
        let compacted = compact_src(r#"fn demo() { let _ = "; /proc /cpuinfo"; }"#);
        assert!(compacted.contains(";/proc/cpuinfo"), "{compacted}");
        guard(
            Path::new("/work/constellation/sim-fixture"),
            "README.md",
            &compacted,
        )
        .unwrap_or_else(|err| panic!("{err}\n{compacted}"));
    }

    #[test]
    fn a_later_join_strips_the_reviewed_literal_and_keeps_the_machine_path_apart() {
        let compacted = compact_src(r#"fn demo() { let _ = "; /proc /cpu info; /tmp /db"; }"#);
        assert!(compacted.contains(";/proc/cpuinfo;"), "{compacted}");
        assert!(!compacted.contains("/proc /"), "{compacted}");
        assert!(!compacted.contains("/tmp/db"), "{compacted}");
        assert!(
            !path_tokens(&compacted).contains(&"/tmp/db"),
            "{compacted} {:?}",
            path_tokens(&compacted)
        );
        guard(
            Path::new("/work/constellation/sim-fixture"),
            "README.md",
            &compacted,
        )
        .unwrap_or_else(|err| panic!("{err}\n{compacted}"));
    }
}
