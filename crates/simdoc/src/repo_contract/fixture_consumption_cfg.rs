// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Whether an attribute or a `cfg` predicate takes an item out of a normal
//! test build (see `fixture_consumption`).

use syn::Lit;

/// Whether an attribute (as `#[...]` text without spaces) takes its item out
/// of a normal test build: a `cfg` other than `cfg(test)`, any `cfg_attr`
/// (which may add `ignore`), or `ignore`.
pub(super) fn attribute_disabled(text: &str) -> bool {
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
pub(super) fn cfg_holds(meta: &syn::Meta) -> Option<bool> {
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
