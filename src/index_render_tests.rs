use std::collections::BTreeSet;

use sim_index_core::{
    CanonicalFeatureKey, DiscoveredSpecimen, FeatureId, FeatureRecord, IndexDoc, SpecimenId,
    SubjectId, SubjectRecord, Visibility,
};

use super::*;

#[test]
fn render_emits_named_pages_and_cards() {
    let doc = fixture_doc();
    let files = expected_files(&doc).unwrap();
    let names = files.iter().map(|file| file.name).collect::<BTreeSet<_>>();

    for name in [
        "overview.md",
        "user.md",
        "code.md",
        "frameworks.md",
        "surfaces.md",
        "languages.md",
        "packages.md",
        "specimens.md",
        "routes.md",
        "index.json",
        "index.cards.jsonl",
    ] {
        assert!(names.contains(name), "missing {name}");
    }
    assert!(
        files
            .iter()
            .find(|file| file.name == "index.cards.jsonl")
            .unwrap()
            .contents
            .contains("\"kind\":\"feature\"")
    );
}

#[test]
fn find_matches_feature_summary() {
    let rows = find_rows(&fixture_doc(), "routing");

    assert_eq!(rows[0]["kind"], "feature");
    assert_eq!(rows[0]["id"], "feature/demo");
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
        surfaces: Vec::new(),
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
