use sim_index_core::{
    DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface, SubjectRecord, Visibility,
};

use super::*;

#[test]
fn parses_and_materializes_prose_only_features() {
    let overlay = parse_overlay(
        r#"
schema = "sim.features"

[[feature]]
id = "feature/sim-run/repl"
title = "Interactive REPL"
summary = "Run a checked interactive session."
owner = "crate/sim-lib-repl"
audiences = ["user"]
guidance = "Start here when exploring the runtime."
claims_anchors = ["anchor/cli/repl"]
claims_surfaces = ["cli/repl"]
claims_specimens = ["recipe/sim-run/01-basics/version"]

[[route]]
id = "route/start-a-session"
task = "Start a SIM session"
audiences = ["user"]
steps = [
  { feature = "feature/sim-run/repl", why = "The REPL is interactive." },
  { specimen = "recipe/sim-run/01-basics/version", why = "The recipe is runnable." },
]
"#,
    )
    .expect("parse overlay");

    let merged = merge_authored(test_doc(), overlay).expect("merge overlay");

    assert_eq!(merged.features.len(), 1);
    assert_eq!(merged.routes.len(), 1);
    assert_eq!(merged.features[0].id.as_str(), "feature/sim-run/repl");
    assert_eq!(merged.features[0].anchors[0].as_str(), "anchor/cli/repl");
}

#[test]
fn literal_anchor_claim_is_rejected_at_parse_time() {
    let err = parse_overlay(
        r#"
schema = "sim.features"

[[feature]]
id = "feature/sim-run/bad"
title = "Bad"
summary = "Bad literal claim."
owner = "crate/sim-lib-repl"
anchors = ["anchor/cli/repl"]
"#,
    )
    .unwrap_err();

    assert!(err.contains("literal anchor claim: rejected"));
}

#[test]
fn literal_claims_are_rejected_by_index_check_too() {
    let mut doc = test_doc();
    doc.drafts.push(FeatureDraft {
        id: FeatureId::new("feature/sim-run/bad"),
        subject: SubjectId::new("crate/sim-lib-repl"),
        title: "Bad".to_owned(),
        summary: "Bad literal claim.".to_owned(),
        claims_anchors: Vec::new(),
        claims_surfaces: Vec::new(),
        claims_specimens: Vec::new(),
        literal_anchors: vec!["anchor/cli/repl".to_owned()],
        literal_surfaces: Vec::new(),
        literal_specimens: Vec::new(),
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    });

    let err = check_index_doc(&doc).unwrap_err().to_string();

    assert!(err.contains("literal anchor claim"));
}

#[test]
fn unresolved_discovered_ids_are_rejected_before_materialization() {
    let overlay = parse_overlay(
        r#"
schema = "sim.features"

[[feature]]
id = "feature/sim-run/missing"
title = "Missing"
summary = "Missing discovered row."
owner = "crate/sim-lib-repl"
claims_specimens = ["recipe/sim-run/missing"]
"#,
    )
    .expect("parse overlay");

    let err = merge_authored(test_doc(), overlay).unwrap_err();

    assert!(err.contains("unresolved discovered id: rejected"));
}

#[test]
fn authored_feature_suppresses_matching_draft() {
    let overlay = parse_overlay(
        r#"
schema = "sim.features"

[[feature]]
id = "feature/sim-run/repl"
title = "REPL"
summary = "Interactive command loop."
owner = "crate/sim-lib-repl"
claims_surfaces = ["cli/repl"]
"#,
    )
    .expect("parse overlay");
    let mut doc = test_doc();
    doc.drafts.push(FeatureDraft {
        id: FeatureId::new("feature/cli/repl"),
        subject: SubjectId::new("crate/sim-lib-repl"),
        title: "REPL draft".to_owned(),
        summary: "Discovered REPL draft.".to_owned(),
        claims_anchors: Vec::new(),
        claims_surfaces: vec![SurfaceId::new("cli/repl")],
        claims_specimens: Vec::new(),
        literal_anchors: Vec::new(),
        literal_surfaces: Vec::new(),
        literal_specimens: Vec::new(),
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    });

    let merged = merge_authored(doc, overlay).expect("merge overlay");

    assert!(merged.drafts.is_empty());
    assert_eq!(merged.features.len(), 1);
}

#[test]
fn authored_feature_relations_cover_supported_labels() {
    let overlay = parse_overlay(
        r#"
schema = "sim.features"
feature = [
  { id = "feature/demo/facade", title = "Facade", summary = "Presents a local implementation feature.", owner = "crate/sim-lib-repl", claims_surfaces = ["cli/repl"], presents = ["feature/demo/implementation"] },
  { id = "feature/demo/implementation", title = "Implementation", summary = "Implements the local facade behavior.", owner = "crate/sim-lib-repl", claims_specimens = ["recipe/sim-run/01-basics/version"] },
]
"#,
    )
    .expect("parse overlay");

    let merged = merge_authored(test_doc(), overlay).expect("merge overlay");
    let edge = merged
        .edges
        .iter()
        .find(|edge| edge.rel == "presents")
        .expect("presents edge");

    assert_eq!(edge.from, "feature/demo/facade");
    assert_eq!(edge.to, "feature/demo/implementation");
}

#[test]
fn authored_feature_preserves_generated_grammar_contract() {
    let overlay = parse_overlay(
        r#"
schema = "sim.features"

[[feature]]
id = "feature/sim-codecs/lisp-syntax"
title = "Lisp syntax"
summary = "Read and write Lisp syntax through the codec grammar."
owner = "crate/sim-lib-repl"
claims_surfaces = ["cli/repl"]
"#,
    )
    .expect("parse overlay");
    let mut doc = test_doc();
    doc.drafts.push(FeatureDraft {
        id: FeatureId::new("feature/syntax/lisp"),
        subject: SubjectId::new("crate/sim-lib-repl"),
        title: "Lisp syntax draft".to_owned(),
        summary: "Generated grammar draft.".to_owned(),
        claims_anchors: Vec::new(),
        claims_surfaces: vec![SurfaceId::new("cli/repl")],
        claims_specimens: Vec::new(),
        literal_anchors: Vec::new(),
        literal_surfaces: Vec::new(),
        literal_specimens: Vec::new(),
        grammar_contracts: vec![GrammarContract {
            id: "grammar/syntax/lisp".to_owned(),
            decoder: Some(AnchorId::new("anchor/cli/repl")),
            encoder: Some(AnchorId::new("anchor/cli/repl")),
            surface: Some(SurfaceId::new("cli/repl")),
            round_trip: true,
        }],
        doc_anchor: None,
    });

    let merged = merge_authored(doc, overlay).expect("merge overlay");

    assert!(merged.drafts.is_empty());
    assert_eq!(merged.features[0].grammar_contracts.len(), 1);
    assert_eq!(
        merged.features[0].grammar_contracts[0].id,
        "grammar/syntax/lisp"
    );
}

fn test_doc() -> IndexDoc {
    IndexDoc {
        schema: "sim.index".to_owned(),
        generated_by: "test".to_owned(),
        visibility: Visibility::Public,
        subjects: vec![SubjectRecord {
            id: SubjectId::new("crate/sim-lib-repl"),
            kind: "crate".to_owned(),
            title: "sim-lib-repl".to_owned(),
        }],
        anchors: vec![DiscoveredAnchor {
            id: AnchorId::new("anchor/cli/repl"),
            subject: SubjectId::new("crate/sim-lib-repl"),
            kind: "cli-verb".to_owned(),
        }],
        surfaces: vec![DiscoveredSurface {
            id: SurfaceId::new("cli/repl"),
            subject: SubjectId::new("crate/sim-lib-repl"),
            kind: "cli".to_owned(),
        }],
        specimens: vec![DiscoveredSpecimen {
            id: SpecimenId::new("recipe/sim-run/01-basics/version"),
            subject: SubjectId::new("crate/sim-lib-repl"),
            kind: "recipe".to_owned(),
            path: "recipes/01-basics/version/recipe.toml".to_owned(),
            language: None,
            runnable: true,
            checked: true,
            checked_by: Some("xtask check-recipes".to_owned()),
            doc_anchor: None,
        }],
        drafts: Vec::new(),
        features: Vec::new(),
        routes: Vec::new(),
        edges: Vec::new(),
    }
}
