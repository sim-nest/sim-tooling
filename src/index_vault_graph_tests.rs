use sim_index_core::{
    AnchorId, CanonicalFeatureKey, DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface,
    FeatureDraft, FeatureId, FeatureRecord, GrammarContract, IndexDoc, IndexEdge, RouteId,
    RouteRecord, RouteStep, SpecimenId, SubjectId, SubjectRecord, SurfaceId, Visibility,
};

use crate::index_vault_graph::{
    VaultEndpoint, VaultGranularity, VaultGraph, VaultNode, VaultNodeKind, VaultRelation,
};

// conformance: vault graphs derive app-neutral note rows from checked Index records.

#[test]
fn vault_graph_contract_matches_checked_fixture() {
    let graph = VaultGraph::from_index(&fixture_doc()).unwrap();

    graph.check(VaultGranularity::Compact).unwrap();
    assert_eq!(graph.coverage.unrepresented_rows(), 0);
    assert_eq!(graph.unresolved_relations().count(), 0);
    assert!(graph.nodes.iter().any(|node| {
        matches!(
            node,
            VaultNode::Feature(feature)
                if feature.id == "feature/demo"
                    && feature.grammar_contracts[0].id == "grammar/demo"
                    && feature.grammar_contracts[0].round_trip
        )
    }));

    let route_steps = graph
        .relations
        .iter()
        .filter(|relation| relation.rel == "route-step")
        .map(|relation| {
            (
                relation.order.unwrap(),
                relation.to.kind,
                relation.to.id.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        route_steps,
        [
            (0, VaultNodeKind::Feature, "feature/demo"),
            (1, VaultNodeKind::Specimen, "recipe/demo/open"),
        ]
    );
    let mut expected_reverse = graph
        .relations
        .iter()
        .map(VaultRelation::reversed)
        .collect::<Vec<_>>();
    expected_reverse.sort();
    assert_eq!(graph.reverse_relations, expected_reverse);
}

#[test]
fn permuted_input_rows_produce_identical_graphs() {
    let original = VaultGraph::from_index(&fixture_doc()).unwrap();
    let mut shuffled = fixture_doc();
    shuffled.subjects.reverse();
    shuffled.anchors.reverse();
    shuffled.surfaces.reverse();
    shuffled.specimens.reverse();
    shuffled.drafts.reverse();
    shuffled.features.reverse();
    shuffled.routes.reverse();
    shuffled.edges.reverse();
    shuffled.features[0].anchors.reverse();
    shuffled.features[0].surfaces.reverse();
    shuffled.features[0].specimens.reverse();

    let permuted = VaultGraph::from_index(&shuffled).unwrap();

    assert_eq!(permuted, original);
}

#[test]
fn invalid_unchecked_index_is_rejected_before_graphing() {
    let mut doc = fixture_doc();
    doc.subjects[0].id = SubjectId::new("bad subject");

    let err = VaultGraph::from_index(&doc).unwrap_err();

    assert!(err.contains("invalid index document"));
}

#[test]
fn duplicate_index_edges_are_rejected_as_duplicate_relations() {
    let mut doc = fixture_doc();
    doc.edges.push(IndexEdge::relates(
        FeatureId::new("feature/demo"),
        "supports",
        FeatureId::new("feature/other"),
    ));

    let err = VaultGraph::from_index(&doc).unwrap_err();

    assert!(err.contains("duplicate forward relation"));
}

#[test]
fn duplicate_raw_ids_across_node_kinds_are_rejected() {
    let mut doc = fixture_doc();
    doc.surfaces.push(DiscoveredSurface {
        id: SurfaceId::new("anchor/demo/doc"),
        subject: SubjectId::new("crate/demo"),
        kind: "syntax".to_owned(),
    });

    let err = VaultGraph::from_index(&doc).unwrap_err();

    assert!(err.contains("duplicate id anchor/demo/doc"));
}

#[test]
fn incomplete_granularity_coverage_is_rejected() {
    let mut graph = VaultGraph::from_index(&fixture_doc()).unwrap();
    let removed = VaultEndpoint {
        kind: VaultNodeKind::Feature,
        id: "feature/demo".to_owned(),
    };
    graph.coverage.full.remove(&removed);

    let err = graph.check(VaultGranularity::Full).unwrap_err();

    assert!(err.contains("unrepresented Full row"));
}

#[test]
fn unresolved_relation_iterator_reports_missing_endpoints() {
    let mut graph = VaultGraph::from_index(&fixture_doc()).unwrap();
    graph.relations.push(VaultRelation {
        from: VaultEndpoint {
            kind: VaultNodeKind::Feature,
            id: "feature/demo".to_owned(),
        },
        rel: "broken".to_owned(),
        to: VaultEndpoint {
            kind: VaultNodeKind::Anchor,
            id: "anchor/missing".to_owned(),
        },
        order: None,
    });

    assert_eq!(graph.unresolved_relations().count(), 1);
    assert!(graph.check(VaultGranularity::Compact).is_err());
}

fn fixture_doc() -> IndexDoc {
    IndexDoc {
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
                id: SubjectId::new("repo/demo"),
                kind: "repo".to_owned(),
                title: "Demo repo".to_owned(),
            },
        ],
        anchors: vec![
            DiscoveredAnchor {
                id: AnchorId::new("anchor/demo/doc"),
                subject: SubjectId::new("crate/demo"),
                kind: "doc-section".to_owned(),
            },
            DiscoveredAnchor {
                id: AnchorId::new("anchor/demo/decoder"),
                subject: SubjectId::new("crate/demo"),
                kind: "export".to_owned(),
            },
            DiscoveredAnchor {
                id: AnchorId::new("anchor/demo/encoder"),
                subject: SubjectId::new("crate/demo"),
                kind: "export".to_owned(),
            },
        ],
        surfaces: vec![DiscoveredSurface {
            id: SurfaceId::new("syntax/demo"),
            subject: SubjectId::new("crate/demo"),
            kind: "syntax".to_owned(),
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
            doc_anchor: Some(AnchorId::new("anchor/demo/doc")),
        }],
        drafts: vec![FeatureDraft {
            id: FeatureId::new("feature/demo-draft"),
            subject: SubjectId::new("crate/demo"),
            title: "Demo draft".to_owned(),
            summary: "Draft row.".to_owned(),
            claims_anchors: vec![AnchorId::new("anchor/demo/doc")],
            claims_surfaces: vec![SurfaceId::new("syntax/demo")],
            claims_specimens: vec![SpecimenId::new("recipe/demo/open")],
            literal_anchors: Vec::new(),
            literal_surfaces: Vec::new(),
            literal_specimens: Vec::new(),
            grammar_contracts: vec![grammar_contract()],
            doc_anchor: Some(AnchorId::new("anchor/demo/doc")),
        }],
        features: vec![
            FeatureRecord {
                id: FeatureId::new("feature/demo"),
                key: CanonicalFeatureKey::new("crate/demo/demo"),
                subject: SubjectId::new("crate/demo"),
                title: "Demo".to_owned(),
                summary: "Demo feature.".to_owned(),
                anchors: vec![
                    AnchorId::new("anchor/demo/encoder"),
                    AnchorId::new("anchor/demo/decoder"),
                ],
                surfaces: vec![SurfaceId::new("syntax/demo")],
                specimens: vec![SpecimenId::new("recipe/demo/open")],
                grammar_contracts: vec![grammar_contract()],
                doc_anchor: Some(AnchorId::new("anchor/demo/doc")),
            },
            FeatureRecord {
                id: FeatureId::new("feature/other"),
                key: CanonicalFeatureKey::new("crate/demo/other"),
                subject: SubjectId::new("crate/demo"),
                title: "Other".to_owned(),
                summary: "Other feature.".to_owned(),
                anchors: Vec::new(),
                surfaces: Vec::new(),
                specimens: Vec::new(),
                grammar_contracts: Vec::new(),
                doc_anchor: None,
            },
        ],
        routes: vec![RouteRecord {
            id: RouteId::new("route/open-demo"),
            title: "Open demo".to_owned(),
            audiences: vec!["framework".to_owned(), "code".to_owned()],
            steps: vec![
                RouteStep::Feature {
                    id: FeatureId::new("feature/demo"),
                    why: "Start with the reusable feature.".to_owned(),
                },
                RouteStep::Specimen {
                    id: SpecimenId::new("recipe/demo/open"),
                    why: "Then run the checked specimen.".to_owned(),
                },
            ],
            doc_anchor: Some(AnchorId::new("anchor/demo/doc")),
        }],
        edges: vec![
            IndexEdge::contains(SubjectId::new("repo/demo"), SubjectId::new("crate/demo")),
            IndexEdge::relates(
                FeatureId::new("feature/demo"),
                "supports",
                FeatureId::new("feature/other"),
            ),
        ],
    }
}

fn grammar_contract() -> GrammarContract {
    GrammarContract {
        id: "grammar/demo".to_owned(),
        decoder: Some(AnchorId::new("anchor/demo/decoder")),
        encoder: Some(AnchorId::new("anchor/demo/encoder")),
        surface: Some(SurfaceId::new("syntax/demo")),
        round_trip: true,
    }
}
