use std::{collections::BTreeMap, fs, path::Path};

use sim_index_core::{
    AnchorId, DeclarationFact as IndexDeclarationFact, DeclarationRole, DiscoveredAnchor,
    ProtocolRelation, ProtocolResolution as IndexProtocolResolution, SourceLocation, SubjectId,
    SyntaxBound, UnresolvedReason,
};

use super::{
    SOURCE_SYNTAX_BOUND,
    declaration::{
        DeclarationLimits, ProtocolImplFact, PublicItemKind, declaration_facts,
        protocol::{ProtocolResolution, ProtocolUnresolvedReason},
    },
    is_test_source, package_rust_files, rel_path, slug_path, subject_id,
};
use crate::repo_contract::PackageContract;

pub(crate) fn source_facts(
    repo: &Path,
    packages: &[PackageContract],
) -> (
    Vec<DiscoveredAnchor>,
    Vec<IndexDeclarationFact>,
    Vec<ProtocolRelation>,
) {
    let mut anchors = BTreeMap::new();
    let mut declarations = BTreeMap::new();
    let mut relations = Vec::new();
    for package in packages {
        let subject = subject_id("crate", &package.name);
        for path in package_rust_files(repo, package) {
            let rel = rel_path(repo, &path);
            if is_test_source(&rel) {
                continue;
            }
            let Ok(text) = fs::read_to_string(path) else {
                continue;
            };
            let scan = declaration_facts(&rel, &text, DeclarationLimits::default());
            for fact in scan.facts {
                let fact = index_fact(&package.name, fact);
                let identity = (fact.anchor.clone(), fact.role, fact.module_path.clone());
                match declarations.entry(identity) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(fact);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        if source_precedes(&fact.location, &entry.get().location) {
                            entry.insert(fact);
                        }
                    }
                }
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
    relations.sort();
    (anchors.into_values().collect(), declarations, relations)
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
            truncated: false,
        },
    }
}

fn source_precedes(left: &SourceLocation, right: &SourceLocation) -> bool {
    (
        Path::new(&left.file).components().count(),
        &left.file,
        left.declaration,
    ) < (
        Path::new(&right.file).components().count(),
        &right.file,
        right.declaration,
    )
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
