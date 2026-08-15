use sim_index_core::{
    AnchorId, CanonicalFeatureKey, DeclarationFact, DeclarationRole, DiscoveredAnchor,
    DiscoveredSpecimen, DiscoveredSurface, FeatureId, FeatureRecord, IndexDoc, ProtocolRelation,
    ProtocolResolution, RouteId, RouteRecord, RouteStep, SourceLocation, SpecimenId, SubjectId,
    SubjectRecord, SurfaceId, SyntaxBound, UnresolvedReason, Visibility,
};

use super::*;

#[test]
fn filter_only_cli_accepts_implements_and_resolution_state() {
    let args = [
        "xtask",
        "index",
        "find",
        "--input",
        "index.sx",
        "--implements",
        "sim_kernel::Callable",
        "--resolved",
    ]
    .map(str::to_owned);

    let options = FindOptions::parse(&args).expect("filter-only query");
    assert_eq!(options.query, "");
    assert_eq!(options.implements.as_deref(), Some("sim_kernel::Callable"));
    assert_eq!(options.resolution, Some(ResolutionFilter::Resolved));
}

#[test]
fn cli_rejects_conflicting_resolution_states() {
    let args = [
        "xtask",
        "index",
        "find",
        "--input",
        "index.sx",
        "--resolved",
        "--unresolved",
    ]
    .map(str::to_owned);

    assert!(
        FindOptions::parse(&args)
            .expect_err("conflicting states")
            .contains("choose only one")
    );
}

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

#[test]
fn protocol_filters_return_provenance_rows_for_frozen_examples() {
    let mut doc = fixture_doc();
    for (index, implementor) in [
        "GuestCallable",
        "GuestClass",
        "ManagedObject",
        "AdmissionEnvelope",
    ]
    .into_iter()
    .enumerate()
    {
        add_protocol_example(
            &mut doc,
            index,
            implementor,
            ProtocolResolution::Resolved {
                protocol: "sim_kernel::Callable".to_owned(),
            },
        );
    }
    add_protocol_example(
        &mut doc,
        4,
        "ExternalCallable",
        ProtocolResolution::Unresolved {
            reason: UnresolvedReason::ExternalMetadataAbsent,
            candidates: Vec::new(),
        },
    );

    let options = FindOptions {
        input: PathBuf::new(),
        query: String::new(),
        json: true,
        audience: None,
        surface: None,
        declaration_kind: None,
        implements: Some("sim_kernel::Callable".to_owned()),
        resolution: Some(ResolutionFilter::Resolved),
        feature: None,
    };
    let rows = find_rows(&doc, &options);
    let anchors = rows
        .iter()
        .filter(|row| row["kind"] == "anchor")
        .collect::<Vec<_>>();

    assert_eq!(anchors.len(), 4);
    assert!(
        anchors
            .iter()
            .all(|row| row["protocol_relations"][0]["resolution"] == "resolved")
    );
    assert!(
        rows.iter()
            .any(|row| row["kind"] == "package" && row["id"] == "crate/demo")
    );
    assert!(
        rows.iter()
            .any(|row| row["kind"] == "feature" && row["id"] == "feature/demo")
    );
    assert!(
        rows.iter()
            .any(|row| row["kind"] == "specimen" && row["id"] == "recipe/demo/open")
    );

    let unresolved = find_rows(
        &doc,
        &FindOptions {
            implements: Some("Callable".to_owned()),
            resolution: Some(ResolutionFilter::Unresolved),
            ..options
        },
    );
    let edge = &unresolved
        .iter()
        .find(|row| row["kind"] == "anchor")
        .expect("unresolved anchor")["protocol_relations"][0];
    assert_eq!(edge["resolution"], "unresolved");
    assert_eq!(edge["unresolved_reason"], "external-metadata-absent");
    assert!(edge.get("protocol").is_none());
    assert!(
        unresolved
            .iter()
            .find(|row| row["kind"] == "anchor")
            .and_then(|row| row["title"].as_str())
            .is_some_and(|title| title.contains("unresolved protocol edge"))
    );
}

#[test]
fn declaration_and_owning_feature_filters_compose() {
    let mut doc = fixture_doc();
    add_protocol_example(
        &mut doc,
        0,
        "GuestCallable",
        ProtocolResolution::Resolved {
            protocol: "sim_kernel::Callable".to_owned(),
        },
    );
    doc.declarations.push(DeclarationFact {
        anchor: AnchorId::new("anchor/rustdoc/demo/example-0"),
        role: DeclarationRole::Struct,
        module_path: "guest::GuestCallable".to_owned(),
        generics: String::new(),
        members: Vec::new(),
        location: SourceLocation {
            file: "src/guest.rs".to_owned(),
            declaration: 0,
        },
        syntax_bound: SyntaxBound {
            max_bytes: 4096,
            truncated: false,
        },
    });

    let rows = find_rows(
        &doc,
        &FindOptions {
            input: PathBuf::new(),
            query: String::new(),
            json: true,
            audience: None,
            surface: None,
            declaration_kind: Some(DeclarationRole::Struct),
            implements: None,
            resolution: None,
            feature: Some("feature/demo".to_owned()),
        },
    );

    assert!(rows.iter().any(|row| row["kind"] == "anchor"));
    assert_eq!(
        rows.iter().filter(|row| row["kind"] == "feature").count(),
        1
    );
}

fn add_protocol_example(
    doc: &mut IndexDoc,
    index: usize,
    implementor: &str,
    resolution: ProtocolResolution,
) {
    let anchor = AnchorId::new(format!("anchor/rustdoc/demo/example-{index}"));
    doc.anchors.push(DiscoveredAnchor {
        id: anchor.clone(),
        subject: SubjectId::new("crate/demo"),
        kind: "rustdoc".to_owned(),
    });
    doc.protocol_relations.push(ProtocolRelation {
        anchor: anchor.clone(),
        implementor: implementor.to_owned(),
        source_spelling: "Callable".to_owned(),
        body_fingerprint: "fn call".to_owned(),
        body_bound: SyntaxBound {
            max_bytes: 4096,
            truncated: false,
        },
        resolution,
    });
    doc.features[0].anchors.push(anchor);
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
        declarations: Vec::new(),
        protocol_relations: Vec::new(),
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
