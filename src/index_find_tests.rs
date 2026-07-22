use sim_index_core::{
    CanonicalFeatureKey, DiscoveredSpecimen, DiscoveredSurface, FeatureId, FeatureRecord, IndexDoc,
    RouteId, RouteRecord, RouteStep, SpecimenId, SubjectId, SubjectRecord, SurfaceId, Visibility,
};

use super::*;

#[test]
fn find_matches_feature_summary() {
    let rows = find_rows_filtered(&fixture_doc(), "routing", None, None);

    assert_eq!(rows[0]["kind"], "feature");
    assert_eq!(rows[0]["id"], "feature/demo");
}

#[test]
fn find_matches_surface_rows() {
    let rows = find_rows_filtered(&fixture_doc(), "view-edit", None, None);

    assert_eq!(rows[0]["kind"], "surface");
    assert_eq!(rows[0]["id"], "view-edit/demo");
}

#[test]
fn audience_filter_keeps_features_reached_by_matching_routes() {
    let rows = find_rows_filtered(&fixture_doc(), "demo", Some("framework"), None);
    let ids = rows
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert!(ids.contains(&"feature/demo"));
    assert!(ids.contains(&"route/use-demo-framework"));
    assert!(!ids.contains(&"crate/demo"));
    assert!(!ids.contains(&"view-edit/demo"));
}

#[test]
fn surface_filter_keeps_language_grammar_and_claiming_features() {
    let rows = find_rows_filtered(&fixture_doc(), "lisp", None, Some("syntax"));
    let ids = rows
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect::<Vec<_>>();

    assert!(ids.contains(&"language/lisp"));
    assert!(ids.contains(&"grammar/lisp"));
    assert!(ids.contains(&"syntax/lisp"));
    assert!(ids.contains(&"feature/lisp-syntax"));
    assert!(!ids.contains(&"view-edit/demo"));
}

#[test]
fn surface_filter_includes_specimens_claimed_by_matching_feature() {
    let rows = find_rows_filtered(&fixture_doc(), "lisp", None, Some("syntax"));
    let specimen = rows
        .iter()
        .find(|row| row["id"] == "recipe/demo/open")
        .expect("claimed specimen row");

    assert_eq!(specimen["kind"], "specimen");
}

fn fixture_doc() -> IndexDoc {
    let mut doc = IndexDoc {
        schema: "sim.index".to_owned(),
        generated_by: "test".to_owned(),
        visibility: Visibility::Public,
        subjects: vec![
            SubjectRecord {
                id: SubjectId::new("crate/demo"),
                kind: "crate".to_owned(),
                title: "demo".to_owned(),
            },
            SubjectRecord {
                id: SubjectId::new("language/lisp"),
                kind: "language".to_owned(),
                title: "lisp".to_owned(),
            },
            SubjectRecord {
                id: SubjectId::new("grammar/lisp"),
                kind: "grammar".to_owned(),
                title: "lisp grammar".to_owned(),
            },
        ],
        anchors: Vec::new(),
        surfaces: vec![
            DiscoveredSurface {
                id: SurfaceId::new("view-edit/demo"),
                subject: SubjectId::new("crate/demo"),
                kind: "view-edit".to_owned(),
            },
            DiscoveredSurface {
                id: SurfaceId::new("syntax/lisp"),
                subject: SubjectId::new("language/lisp"),
                kind: "syntax".to_owned(),
            },
        ],
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
        routes: vec![RouteRecord {
            id: RouteId::new("route/use-demo-framework"),
            title: "Use the demo framework".to_owned(),
            audiences: vec!["framework".to_owned()],
            steps: vec![RouteStep::Feature {
                id: FeatureId::new("feature/demo"),
                why: "The demo feature is the framework entry point.".to_owned(),
            }],
            doc_anchor: None,
        }],
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
    doc.features.push(FeatureRecord {
        id: FeatureId::new("feature/lisp-syntax"),
        key: CanonicalFeatureKey::new("language/lisp/syntax"),
        subject: SubjectId::new("language/lisp"),
        title: "Lisp syntax".to_owned(),
        summary: "Read and write Lisp syntax.".to_owned(),
        anchors: Vec::new(),
        surfaces: vec![SurfaceId::new("syntax/lisp")],
        specimens: vec![SpecimenId::new("recipe/demo/open")],
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    });
    doc
}
