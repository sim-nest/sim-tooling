use sim_index_core::{
    AnchorId, CanonicalFeatureKey, DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface,
    FeatureDraft, FeatureId, FeatureRecord, GrammarContract, IndexDoc, IndexEdge, RouteId,
    RouteRecord, RouteStep, SpecimenId, SubjectId, SubjectRecord, SurfaceId, Visibility,
};

use crate::{
    content_digest::content_digest,
    index_vault_graph::{VaultGranularity, VaultGraph},
    index_vault_profile::{PROFILES, resolve_profile},
    index_vault_render::{VaultRender, render_vault},
};

#[test]
fn compact_and_full_render_valid_artifact_sets_for_every_profile() {
    let graph = VaultGraph::from_index(&fixture_doc()).unwrap();

    for profile in PROFILES {
        for granularity in [VaultGranularity::Compact, VaultGranularity::Full] {
            let rendered = render_vault(&graph, profile, granularity).unwrap();
            assert_eq!(rendered.coverage().unrepresented_rows(), 0);
            assert!(rendered.unresolved_links().is_empty());
            assert_eq!(rendered.coverage().represented_rows(), graph.nodes.len());
            assert_eq!(
                rendered.coverage().represented_relations(),
                graph.relations.len()
            );
            assert!(rendered.coverage().link_targets() > 0);
        }
    }
}

#[test]
fn compact_embeds_anchors_and_full_adds_anchor_notes() {
    let graph = VaultGraph::from_index(&fixture_doc()).unwrap();
    let profile = resolve_profile("portable").unwrap();
    let compact = render_vault(&graph, profile, VaultGranularity::Compact).unwrap();
    let full = render_vault(&graph, profile, VaultGranularity::Full).unwrap();

    assert!(compact.artifacts.iter().all(|artifact| {
        artifact.path_str() != "anchors/anchor~demo~decoder.md"
            && artifact.path_str() != "anchors/anchor~demo~doc.md"
    }));
    assert!(full.artifacts.iter().any(|artifact| {
        artifact.path_str() == "anchors/anchor~demo~decoder.md"
            || artifact.path_str() == "anchors/anchor~demo~doc.md"
    }));

    let feature_note = artifact_text(&compact, "features/feature~demo.md");
    assert!(feature_note.contains("Anchors:"));
    assert!(feature_note.contains("anchor/demo/decoder"));
    assert!(feature_note.contains("index-edge:supports"));
}

#[test]
fn note_sets_include_navigation_relations_route_order_and_provenance() {
    let graph = VaultGraph::from_index(&fixture_doc()).unwrap();
    let profile = resolve_profile("portable").unwrap();
    let rendered = render_vault(&graph, profile, VaultGranularity::Compact).unwrap();

    let readme = artifact_text(&rendered, "README.md");
    assert!(readme.contains("portable-markdown-v1"));
    assert!(readme.contains("[Demo](features/feature~demo.md)"));
    assert!(readme.contains("[Open demo](routes/route~open-demo.md)"));

    let route = artifact_text(&rendered, "routes/route~open-demo.md");
    let first = route.find("1: [Demo]").unwrap();
    let second = route.find("2: [recipe/demo/open]").unwrap();
    assert!(first < second);
    assert!(route.contains("## Relations"));
    assert!(route.contains("### Outgoing"));
    assert!(route.contains("### Incoming"));
    assert!(route.contains("schema: \"sim.index\""));
    assert!(route.contains("generated-by: \"test\""));
}

#[test]
fn renders_are_byte_identical_after_input_permutation() {
    let graph = VaultGraph::from_index(&fixture_doc()).unwrap();
    let mut doc = fixture_doc();
    doc.subjects.reverse();
    doc.anchors.reverse();
    doc.surfaces.reverse();
    doc.specimens.reverse();
    doc.drafts.reverse();
    doc.features.reverse();
    doc.routes.reverse();
    doc.edges.reverse();
    for feature in &mut doc.features {
        feature.anchors.reverse();
        feature.surfaces.reverse();
        feature.specimens.reverse();
    }
    let permuted = VaultGraph::from_index(&doc).unwrap();
    let profile = resolve_profile("obsidian").unwrap();

    let first = render_vault(&graph, profile, VaultGranularity::Compact).unwrap();
    let second = render_vault(&permuted, profile, VaultGranularity::Compact).unwrap();

    assert_eq!(digests(&first), digests(&second));
}

#[test]
fn render_rejects_host_specific_source_paths_before_returning_bytes() {
    let mut doc = fixture_doc();
    doc.specimens[0].path = "C:\\tmp\\recipe.toml".to_owned();
    let graph = VaultGraph::from_index(&doc).unwrap();
    let profile = resolve_profile("portable").unwrap();

    let err = render_vault(&graph, profile, VaultGranularity::Compact).unwrap_err();

    assert!(err.contains("source path must be relative"));
}

fn artifact_text(rendered: &VaultRender, path: &str) -> String {
    String::from_utf8(
        rendered
            .artifacts
            .iter()
            .find(|artifact| artifact.path_str() == path)
            .unwrap_or_else(|| panic!("missing {path}"))
            .bytes
            .clone(),
    )
    .unwrap()
}

fn digests(rendered: &VaultRender) -> Vec<(&str, String)> {
    rendered
        .artifacts
        .iter()
        .map(|artifact| (artifact.path_str(), content_digest(&artifact.bytes)))
        .collect()
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
