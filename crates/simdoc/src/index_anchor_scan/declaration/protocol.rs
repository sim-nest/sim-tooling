//! Conservative protocol-implementation resolution over one parsed Rust file.

use std::collections::{BTreeMap, BTreeSet};

use quote::ToTokens;
use syn::fold::Fold;

use super::{compact_tokens, normalize_tokens};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::index_anchor_scan) struct ProtocolImplFact {
    pub(in crate::index_anchor_scan) protocol: ProtocolResolution,
    pub(in crate::index_anchor_scan) source_spelling: String,
    pub(in crate::index_anchor_scan) implementor: String,
    pub(in crate::index_anchor_scan) body_fingerprint: String,
    pub(in crate::index_anchor_scan) source_anchor: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::index_anchor_scan) enum ProtocolResolution {
    Resolved(String),
    Unresolved(ProtocolUnresolvedReason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::index_anchor_scan) enum ProtocolUnresolvedReason {
    AmbiguousGlobImport,
    AmbiguousName(Vec<String>),
    ExternalMetadataAbsent,
}

#[derive(Clone, Debug)]
struct Binding {
    name: String,
    target: String,
}

#[derive(Default)]
struct Scope {
    bindings: Vec<Binding>,
    glob_imports: Vec<String>,
    traits: BTreeSet<String>,
}

pub(super) fn protocol_impl_facts(items: &[syn::Item], file: &str) -> Vec<ProtocolImplFact> {
    ProtocolResolver::new(items, file).impl_facts()
}

struct ProtocolResolver<'a> {
    file: &'a str,
    items: &'a [syn::Item],
    scopes: BTreeMap<String, Scope>,
}

impl<'a> ProtocolResolver<'a> {
    fn new(items: &'a [syn::Item], file: &'a str) -> Self {
        let mut resolver = Self {
            file,
            items,
            scopes: BTreeMap::new(),
        };
        resolver.collect_scopes(items, "");
        resolver
    }

    fn collect_scopes(&mut self, items: &[syn::Item], module: &str) {
        let mut scope = Scope::default();
        for item in items {
            match item {
                syn::Item::Trait(item) => {
                    scope.traits.insert(item.ident.to_string());
                }
                syn::Item::Use(item) => collect_bindings(&item.tree, "", &mut scope),
                _ => {}
            }
        }
        self.scopes.insert(module.to_owned(), scope);
        for item in items {
            if let syn::Item::Mod(item) = item
                && let Some((_, nested)) = &item.content
            {
                self.collect_scopes(nested, &canonical_local(module, &item.ident.to_string()));
            }
        }
    }

    fn impl_facts(&self) -> Vec<ProtocolImplFact> {
        let mut facts = Vec::new();
        let mut ordinal = 0;
        self.collect_impls(self.items, "", &mut ordinal, &mut facts);
        facts
    }

    fn collect_impls(
        &self,
        items: &[syn::Item],
        module: &str,
        ordinal: &mut usize,
        facts: &mut Vec<ProtocolImplFact>,
    ) {
        for item in items {
            let declaration = *ordinal;
            *ordinal += 1;
            match item {
                syn::Item::Impl(item) => {
                    let Some((_, protocol, _)) = &item.trait_ else {
                        continue;
                    };
                    facts.push(ProtocolImplFact {
                        protocol: self.resolve(module, protocol),
                        source_spelling: compact_tokens(&protocol.to_token_stream().to_string()),
                        implementor: compact_tokens(&item.self_ty.to_token_stream().to_string()),
                        body_fingerprint: implementation_fingerprint(item),
                        source_anchor: format!("{}#declaration-{declaration}", self.file),
                    });
                }
                syn::Item::Mod(item) => {
                    if let Some((_, nested)) = &item.content {
                        self.collect_impls(
                            nested,
                            &canonical_local(module, &item.ident.to_string()),
                            ordinal,
                            facts,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn resolve(&self, module: &str, path: &syn::Path) -> ProtocolResolution {
        let parts = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let Some(first) = parts.first() else {
            return ProtocolResolution::Unresolved(
                ProtocolUnresolvedReason::ExternalMetadataAbsent,
            );
        };
        if matches!(first.as_str(), "crate" | "self" | "super") {
            return ProtocolResolution::Resolved(resolve_local_prefix(module, &parts));
        }

        let scope = &self.scopes[module];
        let mut candidates = BTreeSet::new();
        if scope.traits.contains(first) {
            candidates.insert(canonical_local(module, &parts.join("::")));
        }
        for binding in scope
            .bindings
            .iter()
            .filter(|binding| binding.name == *first)
        {
            let suffix = parts.iter().skip(1).cloned().collect::<Vec<_>>().join("::");
            let target = if suffix.is_empty() {
                binding.target.clone()
            } else {
                format!("{}::{suffix}", binding.target)
            };
            candidates.extend(self.canonical_targets(module, &target, &mut BTreeSet::new()));
        }
        if !candidates.is_empty() {
            return resolution_from(candidates);
        }
        if parts.len() > 1 {
            return resolution_from(self.canonical_targets(
                module,
                &parts.join("::"),
                &mut BTreeSet::new(),
            ));
        }
        if scope.glob_imports.is_empty() {
            ProtocolResolution::Unresolved(ProtocolUnresolvedReason::ExternalMetadataAbsent)
        } else {
            ProtocolResolution::Unresolved(ProtocolUnresolvedReason::AmbiguousGlobImport)
        }
    }

    fn canonical_targets(
        &self,
        module: &str,
        target: &str,
        seen: &mut BTreeSet<String>,
    ) -> BTreeSet<String> {
        if !seen.insert(format!("{module}:{target}")) {
            return BTreeSet::from([target.to_owned()]);
        }
        let target = canonical_path_from(module, target, &self.scopes);
        let parts = target.split("::").collect::<Vec<_>>();
        let (scope_name, item_name) = if parts.len() == 1 {
            (module.to_owned(), parts[0])
        } else {
            let candidate = parts[..parts.len() - 1].join("::");
            if self.scopes.contains_key(&candidate) {
                (candidate, parts[parts.len() - 1])
            } else {
                return BTreeSet::from([target]);
            }
        };
        let Some(scope) = self.scopes.get(&scope_name) else {
            return BTreeSet::from([target]);
        };
        let matches = scope
            .bindings
            .iter()
            .filter(|binding| binding.name == item_name)
            .collect::<Vec<_>>();
        if !matches.is_empty() {
            matches
                .into_iter()
                .flat_map(|binding| self.canonical_targets(&scope_name, &binding.target, seen))
                .collect()
        } else if scope.traits.contains(item_name) {
            BTreeSet::from([canonical_local(&scope_name, item_name)])
        } else {
            BTreeSet::from([target])
        }
    }
}

fn implementation_fingerprint(item: &syn::ItemImpl) -> String {
    item.items
        .iter()
        .map(|member| match member {
            syn::ImplItem::Fn(method) => {
                let mut normalizer = LocalNormalizer::default();
                for input in &method.sig.inputs {
                    if let syn::FnArg::Typed(input) = input {
                        normalizer.collect_pattern(&input.pat);
                    }
                }
                collect_block_locals(&method.block, &mut normalizer);
                let normalized = normalizer.fold_impl_item_fn(method.clone());
                normalize_tokens(&normalized, &item.generics)
            }
            _ => normalize_tokens(member, &item.generics),
        })
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Default)]
struct LocalNormalizer {
    names: BTreeMap<String, String>,
}

impl LocalNormalizer {
    fn collect_pattern(&mut self, pattern: &syn::Pat) {
        match pattern {
            syn::Pat::Ident(ident) if ident.ident != "self" => {
                let next = format!("local{}", self.names.len());
                self.names.entry(ident.ident.to_string()).or_insert(next);
            }
            syn::Pat::Ident(ident) => {
                if let Some((_, pattern)) = &ident.subpat {
                    self.collect_pattern(pattern);
                }
            }
            syn::Pat::Or(pattern) => pattern
                .cases
                .iter()
                .for_each(|pat| self.collect_pattern(pat)),
            syn::Pat::Paren(pattern) => self.collect_pattern(&pattern.pat),
            syn::Pat::Reference(pattern) => self.collect_pattern(&pattern.pat),
            syn::Pat::Slice(pattern) => pattern
                .elems
                .iter()
                .for_each(|pat| self.collect_pattern(pat)),
            syn::Pat::Struct(pattern) => pattern
                .fields
                .iter()
                .for_each(|field| self.collect_pattern(&field.pat)),
            syn::Pat::Tuple(pattern) => pattern
                .elems
                .iter()
                .for_each(|pat| self.collect_pattern(pat)),
            syn::Pat::TupleStruct(pattern) => pattern
                .elems
                .iter()
                .for_each(|pat| self.collect_pattern(pat)),
            syn::Pat::Type(pattern) => self.collect_pattern(&pattern.pat),
            _ => {}
        }
    }
}

impl Fold for LocalNormalizer {
    fn fold_pat_ident(&mut self, mut pattern: syn::PatIdent) -> syn::PatIdent {
        if let Some(name) = self.names.get(&pattern.ident.to_string()) {
            pattern.ident = syn::Ident::new(name, pattern.ident.span());
        }
        syn::fold::fold_pat_ident(self, pattern)
    }

    fn fold_expr_path(&mut self, mut expression: syn::ExprPath) -> syn::ExprPath {
        if expression.qself.is_none()
            && expression.path.leading_colon.is_none()
            && expression.path.segments.len() == 1
            && let Some(segment) = expression.path.segments.first_mut()
            && let Some(name) = self.names.get(&segment.ident.to_string())
        {
            segment.ident = syn::Ident::new(name, segment.ident.span());
        }
        syn::fold::fold_expr_path(self, expression)
    }
}

fn collect_block_locals(block: &syn::Block, normalizer: &mut LocalNormalizer) {
    for statement in &block.stmts {
        if let syn::Stmt::Local(local) = statement {
            normalizer.collect_pattern(&local.pat);
        }
        // Nested locals are found by the fold's token normalization only after their
        // names have been collected, so walk expression blocks conservatively.
        if let syn::Stmt::Expr(expression, _) = statement {
            collect_expr_locals(expression, normalizer);
        }
    }
}

fn collect_expr_locals(expression: &syn::Expr, normalizer: &mut LocalNormalizer) {
    struct Collector<'a>(&'a mut LocalNormalizer);
    impl Fold for Collector<'_> {
        fn fold_local(&mut self, local: syn::Local) -> syn::Local {
            self.0.collect_pattern(&local.pat);
            syn::fold::fold_local(self, local)
        }
    }
    let _ = Collector(normalizer).fold_expr(expression.clone());
}

fn resolution_from(candidates: BTreeSet<String>) -> ProtocolResolution {
    if candidates.len() == 1 {
        ProtocolResolution::Resolved(candidates.into_iter().next().unwrap())
    } else {
        ProtocolResolution::Unresolved(ProtocolUnresolvedReason::AmbiguousName(
            candidates.into_iter().collect(),
        ))
    }
}

fn canonical_path_from(module: &str, target: &str, scopes: &BTreeMap<String, Scope>) -> String {
    let parts = target.split("::").map(str::to_owned).collect::<Vec<_>>();
    if parts
        .first()
        .is_some_and(|part| matches!(part.as_str(), "crate" | "self" | "super"))
    {
        return resolve_local_prefix(module, &parts);
    }
    let root_candidate = canonical_local("", target);
    let root_module = root_candidate
        .rsplit_once("::")
        .map_or(root_candidate.as_str(), |(parent, _)| parent);
    if scopes.contains_key(root_module) {
        root_candidate
    } else {
        target.to_owned()
    }
}

fn collect_bindings(tree: &syn::UseTree, prefix: &str, scope: &mut Scope) {
    match tree {
        syn::UseTree::Path(path) => {
            let next = if prefix.is_empty() {
                path.ident.to_string()
            } else {
                format!("{prefix}::{}", path.ident)
            };
            collect_bindings(&path.tree, &next, scope);
        }
        syn::UseTree::Name(name) => scope.bindings.push(Binding {
            name: name.ident.to_string(),
            target: if prefix.is_empty() {
                name.ident.to_string()
            } else {
                format!("{prefix}::{}", name.ident)
            },
        }),
        syn::UseTree::Rename(rename) => scope.bindings.push(Binding {
            name: rename.rename.to_string(),
            target: if prefix.is_empty() {
                rename.ident.to_string()
            } else {
                format!("{prefix}::{}", rename.ident)
            },
        }),
        syn::UseTree::Group(group) => group
            .items
            .iter()
            .for_each(|tree| collect_bindings(tree, prefix, scope)),
        syn::UseTree::Glob(_) => scope.glob_imports.push(prefix.to_owned()),
    }
}

fn canonical_local(module: &str, tail: &str) -> String {
    if module.is_empty() {
        format!("crate::{tail}")
    } else {
        format!("{module}::{tail}")
    }
}

fn resolve_local_prefix(module: &str, parts: &[String]) -> String {
    let mut base = module.split("::").map(str::to_owned).collect::<Vec<_>>();
    if base.first().is_some_and(|part| part == "crate") {
        base.remove(0);
    }
    let mut index = 0;
    match parts[0].as_str() {
        "crate" => {
            base.clear();
            index = 1;
        }
        "self" => index = 1,
        "super" => {
            while parts.get(index).is_some_and(|part| part == "super") {
                base.pop();
                index += 1;
            }
        }
        _ => {}
    }
    format!(
        "crate::{}",
        base.into_iter()
            .chain(parts[index..].iter().cloned())
            .collect::<Vec<_>>()
            .join("::")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_anchor_scan::declaration::{DeclarationLimits, declaration_facts};

    #[test]
    fn resolves_qualified_aliased_and_reexported_protocols_to_one_id() {
        let scan = declaration_facts(
            "src/lib.rs",
            r#"
                pub mod api { pub use sim_kernel::Function as KernelFunction; }
                trait LocalProtocol {}
                mod nested { use super::LocalProtocol as Local; struct D; impl Local for D {} }
                use api::KernelFunction as Callable;
                struct A; struct B; struct C;
                impl sim_kernel::Function for A {}
                impl Callable for B {}
                impl api::KernelFunction for C {}
            "#,
            DeclarationLimits::default(),
        );
        assert_eq!(
            scan.protocol_impls
                .iter()
                .map(|fact| &fact.protocol)
                .collect::<Vec<_>>(),
            vec![
                &ProtocolResolution::Resolved("crate::LocalProtocol".to_owned()),
                &ProtocolResolution::Resolved("sim_kernel::Function".to_owned()),
                &ProtocolResolution::Resolved("sim_kernel::Function".to_owned()),
                &ProtocolResolution::Resolved("sim_kernel::Function".to_owned()),
            ]
        );
    }

    #[test]
    fn ambiguous_globs_are_explicit_and_never_create_protocol_edges() {
        let scan = declaration_facts(
            "src/lib.rs",
            "use left::*; use right::*; struct Value; impl Protocol for Value {}",
            DeclarationLimits::default(),
        );
        let fact = &scan.protocol_impls[0];
        assert_eq!(fact.source_spelling, "Protocol");
        assert_eq!(
            fact.protocol,
            ProtocolResolution::Unresolved(ProtocolUnresolvedReason::AmbiguousGlobImport)
        );
        assert_eq!(fact.source_anchor, "src/lib.rs#declaration-3");

        let absent = declaration_facts(
            "src/lib.rs",
            "struct Value; impl UnknownProtocol for Value {}",
            DeclarationLimits::default(),
        );
        assert_eq!(
            absent.protocol_impls[0].protocol,
            ProtocolResolution::Unresolved(ProtocolUnresolvedReason::ExternalMetadataAbsent)
        );

        for source in [
            "use left::Protocol as P; use right::Protocol as P; struct V; impl P for V {}",
            "mod api { pub use left::Protocol as P; pub use right::Protocol as P; } use api::P as Shared; struct V; impl Shared for V {}",
        ] {
            let ambiguous = declaration_facts("src/lib.rs", source, DeclarationLimits::default());
            assert_eq!(
                ambiguous.protocol_impls[0].protocol,
                ProtocolResolution::Unresolved(ProtocolUnresolvedReason::AmbiguousName(vec![
                    "left::Protocol".to_owned(),
                    "right::Protocol".to_owned(),
                ]))
            );
        }
    }

    #[test]
    fn impl_fingerprint_is_format_stable_but_anchor_preserves_source_identity() {
        let left = declaration_facts(
            "src/left.rs",
            "use sim_kernel::Function as F; struct V; impl F for V { fn call(&self)->usize { 1 } }",
            DeclarationLimits::default(),
        );
        let right = declaration_facts(
            "src/right.rs",
            "use sim_kernel::Function as F; struct V; impl F for V { /* layout */ fn call( &self ) -> usize { 1 } }",
            DeclarationLimits::default(),
        );
        assert_eq!(
            left.protocol_impls[0].body_fingerprint,
            right.protocol_impls[0].body_fingerprint
        );
        assert_ne!(
            left.protocol_impls[0].source_anchor,
            right.protocol_impls[0].source_anchor
        );
        assert_eq!(left.protocol_impls[0].implementor, "V");
    }

    #[test]
    fn impl_fingerprint_alpha_normalizes_locals_but_preserves_behavior() {
        let scan = |body| {
            declaration_facts(
                "src/lib.rs",
                &format!("trait Managed {{ fn run(&self); }} struct V; impl Managed for V {{ fn run(&self) {{ {body} }} }}"),
                DeclarationLimits::default(),
            )
            .protocol_impls
            .remove(0)
            .body_fingerprint
        };
        let javascript = scan("let pending = self.queue.len(); self.clear(pending, 7);");
        let python = scan("let count = self.queue.len(); self.clear(count, 7);");
        let different_clearing = scan("let count = self.queue.len(); self.retain(count, 7);");

        assert_eq!(javascript, python);
        assert_ne!(javascript, different_clearing);
        assert!(javascript.contains("queue"));
        assert!(javascript.contains("clear"));
        assert!(javascript.contains('7'));
    }
}
