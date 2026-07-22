use sim_index_core::{
    CanonicalFeatureKey, DiscoveredSurface, FeatureId, FeatureRecord, IndexDoc, SubjectId,
    SubjectRecord, SurfaceId,
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
