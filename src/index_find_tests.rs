use sim_index_core::{
    CanonicalFeatureKey, DiscoveredSpecimen, DiscoveredSurface, FeatureId, FeatureRecord, IndexDoc,
    SpecimenId, SubjectId, SubjectRecord, SurfaceId, Visibility,
};

use super::*;

#[test]
fn find_matches_feature_summary() {
    let rows = find_rows(&fixture_doc(), "routing");

    assert_eq!(rows[0]["kind"], "feature");
    assert_eq!(rows[0]["id"], "feature/demo");
}

#[test]
fn find_matches_surface_rows() {
    let rows = find_rows(&fixture_doc(), "view-edit");

    assert_eq!(rows[0]["kind"], "surface");
    assert_eq!(rows[0]["id"], "view-edit/demo");
}

fn fixture_doc() -> IndexDoc {
    let mut doc = IndexDoc {
        schema: "sim.index".to_owned(),
        generated_by: "test".to_owned(),
        visibility: Visibility::Public,
        subjects: vec![SubjectRecord {
            id: SubjectId::new("crate/demo"),
            kind: "crate".to_owned(),
            title: "demo".to_owned(),
        }],
        anchors: Vec::new(),
        surfaces: vec![DiscoveredSurface {
            id: SurfaceId::new("view-edit/demo"),
            subject: SubjectId::new("crate/demo"),
            kind: "view-edit".to_owned(),
        }],
        specimens: vec![DiscoveredSpecimen {
            id: SpecimenId::new("recipe/demo/open"),
            subject: SubjectId::new("crate/demo"),
            kind: "recipe".to_owned(),
            path: "recipes/open/recipe.toml".to_owned(),
            language: Some("lisp".to_owned()),
            runnable: true,
            checked: true,
            checked_by: Some("cargo test".to_owned()),
            doc_anchor: None,
        }],
        drafts: Vec::new(),
        features: Vec::new(),
        routes: Vec::new(),
        edges: Vec::new(),
    };
    doc.features.push(FeatureRecord {
        id: FeatureId::new("feature/demo"),
        key: CanonicalFeatureKey::new("crate/demo/demo"),
        subject: SubjectId::new("crate/demo"),
        title: "Demo".to_owned(),
        summary: "Routing demo feature.".to_owned(),
        anchors: Vec::new(),
        surfaces: Vec::new(),
        specimens: vec![SpecimenId::new("recipe/demo/open")],
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    });
    doc
}
