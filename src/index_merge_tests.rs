use sim_index_core::{
    AnchorId, CanonicalFeatureKey, DeclarationFact, DeclarationRole, DiscoveredAnchor,
    DiscoveredSurface, FeatureId, FeatureRecord, IndexDoc, SourceLocation, SubjectId,
    SubjectRecord, SurfaceId, SyntaxBound,
};

use super::*;

#[test]
fn merge_namespaces_colliding_surfaces_and_rewrites_feature_claims() {
    let fragments = vec![
        Fragment {
            repo: "sim-left".to_owned(),
            doc: doc_with_surface("crate/left", "feature/left", "Left"),
        },
        Fragment {
            repo: "sim-right".to_owned(),
            doc: doc_with_surface("crate/right", "feature/right", "Right"),
        },
    ];

    let doc = merge_fragments(&fragments).unwrap();

    assert!(
        doc.surfaces
            .iter()
            .any(|surface| surface.id.as_str() == "local/sim-left/site-device/desktop")
    );
    assert!(
        doc.surfaces
            .iter()
            .any(|surface| surface.id.as_str() == "local/sim-right/site-device/desktop")
    );
    let left = doc
        .features
        .iter()
        .find(|feature| feature.id.as_str() == "feature/left")
        .unwrap();
    assert_eq!(
        left.surfaces[0].as_str(),
        "local/sim-left/site-device/desktop"
    );
}

#[test]
fn duplicate_inside_one_fragment_still_fails() {
    let mut doc = doc_with_surface("crate/demo", "feature/demo", "Demo");
    doc.surfaces.push(doc.surfaces[0].clone());
    let fragments = vec![Fragment {
        repo: "sim-demo".to_owned(),
        doc,
    }];

    let err = merge_fragments(&fragments).unwrap_err();

    assert!(err.contains("duplicate"));
}

#[test]
fn multi_repo_source_facts_merge_deterministically_and_reencode_identically() {
    let fragments = vec![
        Fragment {
            repo: "sim-zed".to_owned(),
            doc: doc_with_declaration("crate/zed", "anchor/rustdoc/zed/item", "zed"),
        },
        Fragment {
            repo: "sim-alpha".to_owned(),
            doc: doc_with_declaration("crate/alpha", "anchor/rustdoc/alpha/item", "alpha"),
        },
    ];

    let first = merge_fragments(&fragments).unwrap();
    let second = merge_fragments(&fragments).unwrap();

    assert_eq!(first.declarations.len(), 2);
    assert_eq!(encode_sx(&first).unwrap(), encode_sx(&second).unwrap());
}

#[test]
fn canonical_source_fact_copies_deduplicate_and_conflicts_fail() {
    let mut merged = IndexDoc::public("test");
    let fact = declaration("anchor/rustdoc/demo/item", "demo");
    merge_source_facts(&mut merged, vec![fact.clone(), fact.clone()], Vec::new()).unwrap();
    assert_eq!(merged.declarations, vec![fact.clone()]);

    let mut conflicting = fact;
    conflicting.members.push("field:u64".to_owned());
    let error = merge_source_facts(&mut merged, vec![conflicting], Vec::new()).unwrap_err();
    assert!(error.contains("conflicting declaration copies"));
}

fn doc_with_declaration(subject: &str, anchor: &str, module_path: &str) -> IndexDoc {
    let mut doc = IndexDoc::public("test");
    doc.subjects.push(SubjectRecord {
        id: SubjectId::new(subject),
        kind: "crate".to_owned(),
        title: subject.to_owned(),
    });
    doc.anchors.push(DiscoveredAnchor {
        id: AnchorId::new(anchor),
        subject: SubjectId::new(subject),
        kind: "rustdoc-item".to_owned(),
    });
    doc.declarations.push(declaration(anchor, module_path));
    doc
}

fn declaration(anchor: &str, module_path: &str) -> DeclarationFact {
    DeclarationFact {
        anchor: AnchorId::new(anchor),
        role: DeclarationRole::Struct,
        module_path: module_path.to_owned(),
        generics: String::new(),
        members: Vec::new(),
        location: SourceLocation {
            file: "src/lib.rs".to_owned(),
            declaration: 0,
        },
        syntax_bound: SyntaxBound {
            max_bytes: 16_384,
            truncated: false,
        },
    }
}

fn doc_with_surface(subject: &str, feature: &str, title: &str) -> IndexDoc {
    let mut doc = IndexDoc::public("test");
    doc.subjects.push(SubjectRecord {
        id: SubjectId::new(subject),
        kind: "crate".to_owned(),
        title: subject.to_owned(),
    });
    doc.surfaces.push(DiscoveredSurface {
        id: SurfaceId::new("site-device/desktop"),
        subject: SubjectId::new(subject),
        kind: "site-device".to_owned(),
    });
    doc.features.push(FeatureRecord {
        id: FeatureId::new(feature),
        key: CanonicalFeatureKey::new(format!("{subject}/feature")),
        subject: SubjectId::new(subject),
        title: title.to_owned(),
        summary: format!("{title} summary."),
        anchors: Vec::new(),
        surfaces: vec![SurfaceId::new("site-device/desktop")],
        specimens: Vec::new(),
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    });
    doc
}
