use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use sim_index_core::{
    CanonicalFeatureKey, DiscoveredSpecimen, DiscoveredSurface, FeatureId, FeatureRecord, IndexDoc,
    RouteId, RouteRecord, RouteStep, SpecimenId, SubjectId, SubjectRecord, SurfaceId, Visibility,
};

use super::*;

// conformance: sim index fixpoint checks self feature coverage.

#[test]
fn fixpoint_accepts_self_feature_with_required_coverage() {
    let temp = temp_dir("accepts-self-feature");
    let repo = temp.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("features.toml"),
        self_feature_toml(&["user", "code", "framework"]),
    )
    .unwrap();
    let fragment = temp.join("fragment.sx");
    let committed = temp.join("index.sx");
    let doc = self_doc();
    let encoded = encode_sx(&doc).unwrap();
    fs::write(&fragment, &encoded).unwrap();
    let merged = merge_fragment_paths(std::slice::from_ref(&fragment), true).unwrap();
    fs::write(&committed, encode_sx(&merged).unwrap()).unwrap();
    let mut strictness = Strictness::default();
    strictness.apply_strict_selectors("route").unwrap();

    let report = assert_fixpoint(&committed, &[fragment], &repo, &strictness).unwrap();

    assert_eq!(report.fragments, 1);
    assert_eq!(report.route_gaps, 0);
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn self_feature_missing_audience_fails() {
    let temp = temp_dir("missing-audience");
    let repo = temp.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("features.toml"),
        self_feature_toml(&["user", "code"]),
    )
    .unwrap();

    let err = assert_self_audiences(&repo).unwrap_err();

    assert!(err.contains("missing audience(s): framework"));
    fs::remove_dir_all(temp).unwrap();
}

fn temp_dir(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("sim-tooling-index-fixpoint-{label}-{nanos}"));
    path
}

fn self_feature_toml(audiences: &[&str]) -> String {
    format!(
        r#"schema = "sim.features"

[[feature]]
id = "{SELF_FEATURE_ID}"
title = "SIM Index core"
summary = "Generate, query, route, and check the SIM Index graph."
owner = "crate/xtask"
audiences = [{}]
claims_surfaces = ["cli/xtask", "docs/sim-tooling/generated"]
claims_specimens = ["spec-test/sim-tooling/src/index_fixpoint_tests"]
"#,
        audiences
            .iter()
            .map(|audience| format!("\"{audience}\""))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn self_doc() -> IndexDoc {
    let mut doc = IndexDoc {
        schema: "sim.index".to_owned(),
        generated_by: "test".to_owned(),
        visibility: Visibility::Public,
        subjects: vec![
            SubjectRecord {
                id: SubjectId::new("crate/xtask"),
                kind: "crate".to_owned(),
                title: "xtask".to_owned(),
            },
            SubjectRecord {
                id: SubjectId::new("doc-set/sim-tooling/generated"),
                kind: "doc-set".to_owned(),
                title: "sim-tooling generated docs".to_owned(),
            },
        ],
        anchors: Vec::new(),
        surfaces: vec![
            DiscoveredSurface {
                id: SurfaceId::new("cli/xtask"),
                subject: SubjectId::new("crate/xtask"),
                kind: "cli".to_owned(),
            },
            DiscoveredSurface {
                id: SurfaceId::new("docs/sim-tooling/generated"),
                subject: SubjectId::new("doc-set/sim-tooling/generated"),
                kind: "docs".to_owned(),
            },
        ],
        specimens: vec![DiscoveredSpecimen {
            id: SpecimenId::new("spec-test/sim-tooling/src/index_fixpoint_tests"),
            subject: SubjectId::new("crate/xtask"),
            kind: "spec-test".to_owned(),
            path: "src/index_fixpoint_tests.rs".to_owned(),
            language: None,
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
        id: FeatureId::new(SELF_FEATURE_ID),
        key: CanonicalFeatureKey::new("crate/xtask/feature-sim-index-core"),
        subject: SubjectId::new("crate/xtask"),
        title: "SIM Index core".to_owned(),
        summary: "Generate, query, route, and check the SIM Index graph.".to_owned(),
        anchors: Vec::new(),
        surfaces: vec![
            SurfaceId::new("cli/xtask"),
            SurfaceId::new("docs/sim-tooling/generated"),
        ],
        specimens: vec![SpecimenId::new(
            "spec-test/sim-tooling/src/index_fixpoint_tests",
        )],
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    });
    doc.routes.push(RouteRecord {
        id: RouteId::new("route/find-whether-feature-exists"),
        title: "Find whether a feature exists".to_owned(),
        audiences: vec!["user".to_owned(), "code".to_owned(), "framework".to_owned()],
        steps: vec![RouteStep::Feature {
            id: FeatureId::new(SELF_FEATURE_ID),
            why: "The self-index row owns generated index queries.".to_owned(),
        }],
        doc_anchor: None,
    });
    doc
}
