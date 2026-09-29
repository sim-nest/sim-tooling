//! Bounded, normalized declaration facts produced by the anchor scanner.

use std::collections::BTreeMap;

use quote::ToTokens;

use super::{is_public, join_path};

pub(super) mod protocol;
pub(super) use protocol::ProtocolImplFact;
use protocol::protocol_impl_facts;

#[derive(Clone, Copy, Debug)]
pub(super) struct DeclarationLimits {
    pub(super) max_items: usize,
    pub(super) max_syntax_bytes: usize,
}

impl Default for DeclarationLimits {
    fn default() -> Self {
        Self {
            max_items: 4_096,
            max_syntax_bytes: 16_384,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum PublicItemKind {
    Const,
    Enum,
    Function,
    Module,
    ReExport,
    Static,
    Struct,
    Trait,
    TypeAlias,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct SourceLocation {
    pub(super) file: String,
    pub(super) declaration: usize,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct DeclarationFact {
    pub(super) kind: PublicItemKind,
    pub(super) module_path: String,
    pub(super) generics: String,
    pub(super) members: Vec<String>,
    pub(super) location: SourceLocation,
    pub(super) syntax_truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DeclarationEvidence {
    Malformed {
        file: String,
        message: String,
    },
    TruncatedItems {
        file: String,
        limit: usize,
    },
    TruncatedSyntax {
        file: String,
        declaration: usize,
        limit: usize,
    },
    UnsupportedPublicItem {
        file: String,
        declaration: usize,
        kind: String,
    },
}

#[derive(Debug, Default)]
pub(super) struct DeclarationScan {
    pub(super) facts: Vec<DeclarationFact>,
    pub(super) protocol_impls: Vec<ProtocolImplFact>,
    pub(super) evidence: Vec<DeclarationEvidence>,
}

pub(super) fn declaration_facts(
    file: &str,
    text: &str,
    limits: DeclarationLimits,
) -> DeclarationScan {
    let parsed = match syn::parse_file(text) {
        Ok(parsed) => parsed,
        Err(error) => {
            return DeclarationScan {
                evidence: vec![DeclarationEvidence::Malformed {
                    file: file.to_owned(),
                    message: error.to_string(),
                }],
                ..DeclarationScan::default()
            };
        }
    };
    let mut scan = DeclarationScan::default();
    let mut declaration = 0;
    collect_public_items(&parsed.items, "", file, limits, &mut declaration, &mut scan);
    scan.protocol_impls = protocol_impl_facts(&parsed.items, file);
    scan.facts.sort();
    scan
}

pub(super) fn declaration_facts_in_module(
    file: &str,
    text: &str,
    module_path: &str,
    limits: DeclarationLimits,
) -> DeclarationScan {
    let mut scan = declaration_facts(file, text, limits);
    for fact in &mut scan.facts {
        fact.module_path = join_path(module_path, &fact.module_path);
    }
    scan
}

fn collect_public_items(
    items: &[syn::Item],
    prefix: &str,
    file: &str,
    limits: DeclarationLimits,
    declaration: &mut usize,
    scan: &mut DeclarationScan,
) {
    for item in items {
        let ordinal = *declaration;
        *declaration += 1;
        let fact = match item {
            syn::Item::Const(item) if is_public(&item.vis) => Some(simple_fact(
                PublicItemKind::Const,
                prefix,
                &item.ident,
                &item.generics,
                vec![],
                file,
                ordinal,
            )),
            syn::Item::Enum(item) if is_public(&item.vis) => Some(simple_fact(
                PublicItemKind::Enum,
                prefix,
                &item.ident,
                &item.generics,
                item.variants
                    .iter()
                    .map(|variant| normalize_variant(variant, &item.generics))
                    .collect(),
                file,
                ordinal,
            )),
            syn::Item::Fn(item) if is_public(&item.vis) => Some(simple_fact(
                PublicItemKind::Function,
                prefix,
                &item.sig.ident,
                &item.sig.generics,
                vec![],
                file,
                ordinal,
            )),
            syn::Item::Mod(item) if is_public(&item.vis) => {
                let name = join_path(prefix, &item.ident.to_string());
                let fact = DeclarationFact {
                    kind: PublicItemKind::Module,
                    module_path: name.clone(),
                    generics: String::new(),
                    members: vec![],
                    location: SourceLocation {
                        file: file.to_owned(),
                        declaration: ordinal,
                    },
                    syntax_truncated: false,
                };
                if !push_fact(fact, limits, scan) {
                    return;
                }
                if let Some((_, nested)) = &item.content {
                    collect_public_items(nested, &name, file, limits, declaration, scan);
                }
                None
            }
            syn::Item::Static(item) if is_public(&item.vis) => Some(simple_fact(
                PublicItemKind::Static,
                prefix,
                &item.ident,
                &syn::Generics::default(),
                vec![],
                file,
                ordinal,
            )),
            syn::Item::Struct(item) if is_public(&item.vis) => Some(simple_fact(
                PublicItemKind::Struct,
                prefix,
                &item.ident,
                &item.generics,
                normalize_fields(&item.fields, &item.generics),
                file,
                ordinal,
            )),
            syn::Item::Trait(item) if is_public(&item.vis) => Some(simple_fact(
                PublicItemKind::Trait,
                prefix,
                &item.ident,
                &item.generics,
                vec![],
                file,
                ordinal,
            )),
            syn::Item::Type(item) if is_public(&item.vis) => Some(simple_fact(
                PublicItemKind::TypeAlias,
                prefix,
                &item.ident,
                &item.generics,
                vec![normalize_tokens(&item.ty, &item.generics)],
                file,
                ordinal,
            )),
            syn::Item::Use(item) if is_public(&item.vis) => {
                for path in public_use_paths(&item.tree) {
                    let fact = DeclarationFact {
                        kind: PublicItemKind::ReExport,
                        module_path: join_path(prefix, &path),
                        generics: String::new(),
                        members: vec![normalize_tokens(&item.tree, &syn::Generics::default())],
                        location: SourceLocation {
                            file: file.to_owned(),
                            declaration: ordinal,
                        },
                        syntax_truncated: false,
                    };
                    if !push_fact(fact, limits, scan) {
                        return;
                    }
                }
                None
            }
            _ if public_visibility(item).is_some_and(is_public) => {
                scan.evidence
                    .push(DeclarationEvidence::UnsupportedPublicItem {
                        file: file.to_owned(),
                        declaration: ordinal,
                        kind: item_kind(item).to_owned(),
                    });
                None
            }
            _ => None,
        };
        if let Some(fact) = fact
            && !push_fact(fact, limits, scan)
        {
            return;
        }
    }
}

fn push_fact(
    mut fact: DeclarationFact,
    limits: DeclarationLimits,
    scan: &mut DeclarationScan,
) -> bool {
    if scan.facts.len() >= limits.max_items {
        if !scan
            .evidence
            .iter()
            .any(|evidence| matches!(evidence, DeclarationEvidence::TruncatedItems { .. }))
        {
            scan.evidence.push(DeclarationEvidence::TruncatedItems {
                file: fact.location.file.clone(),
                limit: limits.max_items,
            });
        }
        return false;
    }
    bound_fact(&mut fact, limits.max_syntax_bytes, scan);
    scan.facts.push(fact);
    true
}

fn simple_fact(
    kind: PublicItemKind,
    prefix: &str,
    ident: &syn::Ident,
    generics: &syn::Generics,
    members: Vec<String>,
    file: &str,
    declaration: usize,
) -> DeclarationFact {
    DeclarationFact {
        kind,
        module_path: join_path(prefix, &ident.to_string()),
        generics: normalize_tokens(generics, generics),
        members,
        location: SourceLocation {
            file: file.to_owned(),
            declaration,
        },
        syntax_truncated: false,
    }
}

fn normalize_fields(fields: &syn::Fields, generics: &syn::Generics) -> Vec<String> {
    fields
        .iter()
        .filter(|field| is_public(&field.vis))
        .map(|field| {
            let name = field
                .ident
                .as_ref()
                .map_or_else(|| "_".to_owned(), syn::Ident::to_string);
            format!("{name}:{}", normalize_tokens(&field.ty, generics))
        })
        .collect()
}

fn normalize_variant(variant: &syn::Variant, generics: &syn::Generics) -> String {
    let fields = variant
        .fields
        .iter()
        .map(|field| normalize_tokens(&field.ty, generics))
        .collect::<Vec<_>>()
        .join(",");
    format!("{}({fields})", variant.ident)
}

pub(super) fn normalize_tokens<T: ToTokens>(value: &T, generics: &syn::Generics) -> String {
    let names = generics
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let name = match param {
                syn::GenericParam::Type(param) => param.ident.to_string(),
                syn::GenericParam::Lifetime(param) => param.lifetime.ident.to_string(),
                syn::GenericParam::Const(param) => param.ident.to_string(),
            };
            (name, format!("__generic_{index}"))
        })
        .collect::<BTreeMap<_, _>>();

    fn rewrite(
        stream: proc_macro2::TokenStream,
        names: &BTreeMap<String, String>,
    ) -> proc_macro2::TokenStream {
        stream
            .into_iter()
            .map(|token| match token {
                proc_macro2::TokenTree::Ident(ident) => names
                    .get(&ident.to_string())
                    .map(|name| {
                        proc_macro2::TokenTree::Ident(proc_macro2::Ident::new(name, ident.span()))
                    })
                    .unwrap_or(proc_macro2::TokenTree::Ident(ident)),
                proc_macro2::TokenTree::Group(group) => {
                    let mut normalized =
                        proc_macro2::Group::new(group.delimiter(), rewrite(group.stream(), names));
                    normalized.set_span(group.span());
                    proc_macro2::TokenTree::Group(normalized)
                }
                other => other,
            })
            .collect()
    }

    compact_tokens(&rewrite(value.to_token_stream(), &names).to_string())
}

pub(super) fn compact_tokens(tokens: &str) -> String {
    tokens.split_whitespace().collect()
}

fn bound_fact(fact: &mut DeclarationFact, limit: usize, scan: &mut DeclarationScan) {
    let size = fact.generics.len() + fact.members.iter().map(String::len).sum::<usize>();
    if size <= limit {
        return;
    }
    fact.generics.clear();
    fact.members.clear();
    fact.syntax_truncated = true;
    scan.evidence.push(DeclarationEvidence::TruncatedSyntax {
        file: fact.location.file.clone(),
        declaration: fact.location.declaration,
        limit,
    });
}

fn public_use_paths(tree: &syn::UseTree) -> Vec<String> {
    fn walk(tree: &syn::UseTree, prefix: &str, paths: &mut Vec<String>) {
        match tree {
            syn::UseTree::Path(path) => walk(&path.tree, prefix, paths),
            syn::UseTree::Name(name) => paths.push(join_path(prefix, &name.ident.to_string())),
            syn::UseTree::Rename(rename) => {
                paths.push(join_path(prefix, &rename.rename.to_string()))
            }
            syn::UseTree::Group(group) => group
                .items
                .iter()
                .for_each(|item| walk(item, prefix, paths)),
            // A glob names no stable public declaration of its own. Its
            // exported items are discovered from their declarations; emitting
            // the punctuation-only "*" path would normalize to an empty
            // rustdoc anchor and leave the graph internally inconsistent.
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut paths = vec![];
    walk(tree, "", &mut paths);
    paths
}

fn public_visibility(item: &syn::Item) -> Option<&syn::Visibility> {
    match item {
        syn::Item::ExternCrate(item) => Some(&item.vis),
        syn::Item::Union(item) => Some(&item.vis),
        _ => None,
    }
}

fn item_kind(item: &syn::Item) -> &'static str {
    match item {
        syn::Item::ExternCrate(_) => "extern-crate",
        syn::Item::Macro(_) => "macro",
        syn::Item::Union(_) => "union",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_reexports_do_not_emit_empty_declarations() {
        let scan = declaration_facts(
            "src/lib.rs",
            "mod inner { pub struct Named; }\npub use inner::*;\npub use inner::Named as PublicNamed;\n",
            DeclarationLimits::default(),
        );

        assert!(
            scan.facts
                .iter()
                .all(|declaration| !declaration.module_path.is_empty())
        );
        assert!(
            scan.facts.iter().any(|declaration| {
                declaration.kind == PublicItemKind::ReExport
                    && declaration.module_path == "public-named"
            }),
            "unexpected declaration facts: {:?}",
            scan.facts
        );
    }
}
