use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use sim_index_core::{
    AnchorId, DeclarationFact as IndexDeclarationFact, DeclarationRole, DiscoveredAnchor,
    ProtocolRelation, ProtocolResolution as IndexProtocolResolution, SourceLocation, SubjectId,
    SyntaxBound, UnresolvedReason,
};

use super::{
    SOURCE_SYNTAX_BOUND,
    declaration::{
        DeclarationLimits, ProtocolImplFact, PublicItemKind, declaration_facts_in_module,
        protocol::{ProtocolResolution, ProtocolUnresolvedReason},
    },
    is_test_source, rel_path, slug_path, subject_id,
};
use crate::repo_contract::PackageContract;

type SourceFacts = (
    Vec<DiscoveredAnchor>,
    Vec<IndexDeclarationFact>,
    Vec<ProtocolRelation>,
);

pub(crate) fn source_facts(
    repo: &Path,
    packages: &[PackageContract],
) -> Result<SourceFacts, String> {
    let mut anchors = BTreeMap::new();
    let mut declarations = BTreeMap::new();
    let mut relations = Vec::new();
    for package in packages {
        let subject = subject_id("crate", &package.name);
        for (path, module_path) in reachable_sources(repo, package)? {
            let rel = rel_path(repo, &path);
            if is_test_source(&rel) {
                continue;
            }
            let text = fs::read_to_string(path)
                .map_err(|error| format!("read reachable Rust source {rel}: {error}"))?;
            let scan = declaration_facts_in_module(
                &rel,
                &text,
                &module_path,
                DeclarationLimits::default(),
            );
            if let Some(evidence) = scan.evidence.iter().find(|evidence| {
                matches!(
                    evidence,
                    super::declaration::DeclarationEvidence::Malformed { .. }
                        | super::declaration::DeclarationEvidence::TruncatedItems { .. }
                        | super::declaration::DeclarationEvidence::UnsupportedPublicItem { .. }
                )
            }) {
                return Err(format!(
                    "incomplete reachable Rust source scan for {rel}: {evidence:?}"
                ));
            }
            for fact in scan.facts {
                let fact = index_fact(&package.name, fact);
                let identity = (fact.anchor.clone(), fact.role, fact.location.clone());
                declarations.insert(identity, fact);
            }
            relations.extend(
                scan.protocol_impls
                    .into_iter()
                    .map(|fact| protocol_relation(&mut anchors, &package.name, &subject, fact)),
            );
        }
    }
    let mut declarations = declarations.into_values().collect::<Vec<_>>();
    declarations.sort();
    let mut counts = BTreeMap::new();
    for fact in &declarations {
        *counts.entry(fact.anchor.clone()).or_insert(0usize) += 1;
    }
    for fact in &mut declarations {
        if counts[&fact.anchor] > 1 {
            fact.anchor = AnchorId::new(format!(
                "{}-declaration-{}",
                fact.anchor, fact.location.declaration
            ));
        }
    }
    declarations.sort();
    for fact in &declarations {
        anchors
            .entry(fact.anchor.to_string())
            .or_insert(DiscoveredAnchor {
                id: fact.anchor.clone(),
                subject: subject_id(
                    "crate",
                    fact.anchor.as_str().split('/').nth(2).unwrap_or("unknown"),
                ),
                kind: "rustdoc-item".to_owned(),
            });
    }
    relations.sort();
    Ok((anchors.into_values().collect(), declarations, relations))
}

pub(super) fn reachable_sources(
    repo: &Path,
    package: &PackageContract,
) -> Result<Vec<(PathBuf, String)>, String> {
    let mut roots = package_roots(repo, package);
    roots.sort();
    roots.dedup();
    let mut found = BTreeMap::new();
    let mut active = BTreeSet::new();
    for root in roots {
        walk_modules(repo, &root, "", &mut found, &mut active, false, false)?;
    }
    Ok(found.into_iter().collect())
}

fn package_roots(repo: &Path, package: &PackageContract) -> Vec<PathBuf> {
    package
        .targets
        .iter()
        .filter(|target| {
            target["kind"].as_array().is_some_and(|kinds| {
                !kinds.iter().any(|kind| {
                    matches!(
                        kind.as_str(),
                        Some("example" | "test" | "bench" | "custom-build")
                    )
                })
            })
        })
        .filter_map(|target| {
            target["src_path"]
                .as_str()
                .or_else(|| target["src"].as_str())
        })
        .map(|source| {
            let source = PathBuf::from(source);
            if source.is_absolute() {
                source
            } else {
                repo.join(source)
            }
        })
        .collect()
}

fn walk_modules(
    repo: &Path,
    source: &Path,
    prefix: &str,
    found: &mut BTreeMap<PathBuf, String>,
    active: &mut BTreeSet<PathBuf>,
    include_private: bool,
    api_paths: bool,
) -> Result<(), String> {
    let source = source.canonicalize().map_err(|error| {
        format!(
            "resolve reachable Rust module {}: {error}",
            source.display()
        )
    })?;
    if !source.starts_with(repo) {
        return Err(format!(
            "reachable Rust module escapes repository: {}",
            source.display()
        ));
    }
    if !active.insert(source.clone()) {
        return Err(format!("Rust module cycle at {}", source.display()));
    }
    found
        .entry(source.clone())
        .or_insert_with(|| prefix.to_owned());
    let text = fs::read_to_string(&source)
        .map_err(|error| format!("read reachable Rust module {}: {error}", source.display()))?;
    let parsed = syn::parse_file(&text)
        .map_err(|error| format!("parse reachable Rust module {}: {error}", source.display()))?;
    let reexported_modules = parsed
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                first_use_segment(&item.tree)
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let included_sources = parsed
        .items
        .iter()
        .filter_map(|item| {
            let syn::Item::Macro(item) = item else {
                return None;
            };
            if !item.mac.path.is_ident("include") {
                return None;
            }
            // Only literal includes name a source unit that can be resolved
            // from the repository. Generated OUT_DIR includes and other
            // expression forms belong to Cargo/build-script discovery.
            syn::parse2::<syn::LitStr>(item.mac.tokens.clone())
                .ok()
                .map(|literal| literal.value())
        })
        .collect::<Vec<_>>();
    let parent = source
        .parent()
        .ok_or_else(|| format!("module source has no parent: {}", source.display()))?;
    for included in included_sources {
        walk_modules(
            repo,
            &parent.join(included),
            prefix,
            found,
            active,
            include_private,
            api_paths,
        )?;
    }
    for item in parsed.items {
        let syn::Item::Mod(module) = item else {
            continue;
        };
        if (!include_private
            && !matches!(module.vis, syn::Visibility::Public(_))
            && !reexported_modules.contains(&module.ident.to_string()))
            || module.content.is_some()
        {
            continue;
        }
        let raw_name = module.ident.to_string();
        let name = raw_name.strip_prefix("r#").unwrap_or(&raw_name).to_owned();
        let privately_reexported = !matches!(module.vis, syn::Visibility::Public(_))
            && reexported_modules.contains(&module.ident.to_string());
        let child_prefix = if api_paths && privately_reexported {
            prefix.to_owned()
        } else if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let base = if source
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches!(name, "lib.rs" | "main.rs" | "mod.rs"))
        {
            parent.to_owned()
        } else {
            parent.join(source.file_stem().expect("Rust source has a stem"))
        };
        let explicit = module
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("path"))
            .map(|attr| match &attr.meta {
                syn::Meta::NameValue(value) => match &value.value {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(path),
                        ..
                    }) => Ok(path.value()),
                    _ => Err("path value is not a string"),
                },
                _ => Err("expected #[path = \"...\"]"),
            })
            .transpose()
            .map_err(|error| format!("invalid #[path] in {}: {error}", source.display()))?;
        let child = if let Some(explicit) = explicit {
            parent.join(explicit)
        } else {
            let flat = base.join(format!("{name}.rs"));
            let nested = base.join(&name).join("mod.rs");
            match (flat.is_file(), nested.is_file()) {
                (true, false) => flat,
                (false, true) => nested,
                (true, true) => return Err(format!("ambiguous reachable module {child_prefix}")),
                _ => {
                    return Err(format!(
                        "unresolved reachable module `{name}` ({child_prefix}) from {}",
                        source.display()
                    ));
                }
            }
        };
        walk_modules(
            repo,
            &child,
            &child_prefix,
            found,
            active,
            include_private,
            api_paths,
        )?;
    }
    active.remove(&source);
    Ok(())
}

pub(super) fn compilation_sources(
    repo: &Path,
    package: &PackageContract,
) -> Result<Vec<(PathBuf, String)>, String> {
    let mut roots = package_roots(repo, package);
    roots.sort();
    roots.dedup();
    let mut found = BTreeMap::new();
    let mut active = BTreeSet::new();
    for root in roots {
        walk_modules(repo, &root, "", &mut found, &mut active, true, false)?;
    }
    Ok(found.into_iter().collect())
}

pub(super) fn public_api_sources(
    repo: &Path,
    package: &PackageContract,
) -> Result<Vec<(PathBuf, String)>, String> {
    let mut found = BTreeMap::new();
    let mut active = BTreeSet::new();
    for root in package_roots(repo, package) {
        walk_modules(repo, &root, "", &mut found, &mut active, false, true)?;
    }
    Ok(found.into_iter().collect())
}

fn first_use_segment(tree: &syn::UseTree) -> Option<String> {
    match tree {
        syn::UseTree::Path(path) => Some(path.ident.to_string()),
        syn::UseTree::Name(name) => Some(name.ident.to_string()),
        syn::UseTree::Rename(rename) => Some(rename.ident.to_string()),
        syn::UseTree::Group(_) | syn::UseTree::Glob(_) => None,
    }
}

fn index_fact(package: &str, fact: super::declaration::DeclarationFact) -> IndexDeclarationFact {
    IndexDeclarationFact {
        anchor: AnchorId::new(format!(
            "anchor/rustdoc/{}/{}",
            slug_path(package),
            slug_path(&fact.module_path)
        )),
        role: declaration_role(fact.kind),
        module_path: fact.module_path,
        generics: fact.generics,
        members: fact.members,
        location: SourceLocation {
            file: fact.location.file,
            declaration: fact.location.declaration,
        },
        syntax_bound: SyntaxBound {
            max_bytes: SOURCE_SYNTAX_BOUND,
            truncated: fact.syntax_truncated,
        },
    }
}

fn declaration_role(kind: PublicItemKind) -> DeclarationRole {
    match kind {
        PublicItemKind::Const => DeclarationRole::Const,
        PublicItemKind::Enum => DeclarationRole::Enum,
        PublicItemKind::Function => DeclarationRole::Function,
        PublicItemKind::Module => DeclarationRole::Module,
        PublicItemKind::ReExport => DeclarationRole::ReExport,
        PublicItemKind::Static => DeclarationRole::Static,
        PublicItemKind::Struct => DeclarationRole::Struct,
        PublicItemKind::Trait => DeclarationRole::Trait,
        PublicItemKind::TypeAlias => DeclarationRole::TypeAlias,
    }
}

fn protocol_relation(
    anchors: &mut BTreeMap<String, DiscoveredAnchor>,
    package: &str,
    subject: &SubjectId,
    fact: ProtocolImplFact,
) -> ProtocolRelation {
    let anchor = AnchorId::new(format!(
        "anchor/rust-impl/{}/{}",
        slug_path(package),
        slug_path(&fact.source_anchor)
    ));
    anchors
        .entry(anchor.to_string())
        .or_insert(DiscoveredAnchor {
            id: anchor.clone(),
            subject: subject.clone(),
            kind: "rust-impl".to_owned(),
        });
    let (body_fingerprint, truncated) = if fact.body_fingerprint.len() > SOURCE_SYNTAX_BOUND {
        (String::new(), true)
    } else {
        (fact.body_fingerprint, false)
    };
    let resolution = match fact.protocol {
        ProtocolResolution::Resolved(protocol) => IndexProtocolResolution::Resolved { protocol },
        ProtocolResolution::Unresolved(reason) => {
            let (reason, candidates) = match reason {
                ProtocolUnresolvedReason::AmbiguousGlobImport => {
                    (UnresolvedReason::AmbiguousGlobImport, Vec::new())
                }
                ProtocolUnresolvedReason::AmbiguousName(candidates) => {
                    (UnresolvedReason::AmbiguousName, candidates)
                }
                ProtocolUnresolvedReason::ExternalMetadataAbsent => {
                    (UnresolvedReason::ExternalMetadataAbsent, Vec::new())
                }
            };
            IndexProtocolResolution::Unresolved { reason, candidates }
        }
    };
    ProtocolRelation {
        anchor,
        implementor: fact.implementor,
        source_spelling: fact.source_spelling,
        body_fingerprint,
        body_bound: SyntaxBound {
            max_bytes: SOURCE_SYNTAX_BOUND,
            truncated,
        },
        resolution,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sim-index-{name}-{nonce}"));
        fs::create_dir_all(root.join("src/public")).unwrap();
        root
    }

    #[test]
    fn module_walk_is_path_and_visibility_correct() {
        let root = fixture_root("modules");
        fs::write(
            root.join("src/lib.rs"),
            "mod private; pub mod public; #[path = \"renamed.rs\"] pub mod alias;\n",
        )
        .unwrap();
        fs::write(root.join("src/private.rs"), "pub struct Hidden;\n").unwrap();
        fs::write(root.join("src/public/mod.rs"), "pub mod nested;\n").unwrap();
        fs::write(root.join("src/public/nested.rs"), "pub struct Visible;\n").unwrap();
        fs::write(root.join("src/renamed.rs"), "pub struct ViaPath;\n").unwrap();
        let mut found = BTreeMap::new();
        walk_modules(
            &root,
            &root.join("src/lib.rs"),
            "",
            &mut found,
            &mut BTreeSet::new(),
            false,
            false,
        )
        .unwrap();
        let paths = found.into_values().collect::<BTreeSet<_>>();
        assert_eq!(
            paths,
            BTreeSet::from([
                "".into(),
                "alias".into(),
                "public".into(),
                "public/nested".into()
            ])
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn include_units_keep_the_containing_module_path() {
        let root = fixture_root("includes");
        fs::write(
            root.join("src/lib.rs"),
            "include!(\"surface.rs\"); include!(concat!(env!(\"OUT_DIR\"), \"/generated.rs\")); pub mod public;\n",
        )
        .unwrap();
        fs::write(root.join("src/surface.rs"), "pub struct Included;\n").unwrap();
        fs::write(root.join("src/public/mod.rs"), "include!(\"nested.rs\");\n").unwrap();
        fs::write(
            root.join("src/public/nested.rs"),
            "pub struct NestedIncluded;\n",
        )
        .unwrap();

        let mut found = BTreeMap::new();
        walk_modules(
            &root,
            &root.join("src/lib.rs"),
            "",
            &mut found,
            &mut BTreeSet::new(),
            false,
            false,
        )
        .unwrap();

        assert_eq!(
            found[&root.join("src/surface.rs").canonicalize().unwrap()],
            ""
        );
        assert_eq!(
            found[&root.join("src/public/nested.rs").canonicalize().unwrap()],
            "public"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unresolved_and_malformed_reachable_modules_are_fatal() {
        let root = fixture_root("failures");
        fs::write(root.join("src/lib.rs"), "pub mod missing;\n").unwrap();
        let error = walk_modules(
            &root,
            &root.join("src/lib.rs"),
            "",
            &mut BTreeMap::new(),
            &mut BTreeSet::new(),
            false,
            false,
        )
        .unwrap_err();
        assert!(error.contains("unresolved reachable module `missing`"));
        fs::write(root.join("src/lib.rs"), "pub fn broken(\n").unwrap();
        let error = walk_modules(
            &root,
            &root.join("src/lib.rs"),
            "",
            &mut BTreeMap::new(),
            &mut BTreeSet::new(),
            false,
            false,
        )
        .unwrap_err();
        assert!(error.contains("parse reachable Rust module"));
        fs::remove_dir_all(root).unwrap();
    }
}
