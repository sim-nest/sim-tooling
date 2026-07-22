use std::collections::{BTreeMap, BTreeSet};

use sim_index_core::{
    AnchorId, DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface, FeatureId, FeatureRecord,
    SubjectRecord, SurfaceId, Visibility, key::CanonicalFeatureKey,
};

use super::*;

#[test]
fn strict_selectors_parse_category_values() {
    let mut strictness = Strictness::default();
    strictness
        .apply_strict_selectors("audience:user,surface:cli")
        .expect("parse selectors");

    assert!(strictness.strict_audiences.contains("user"));
    assert!(strictness.strict_surfaces.contains("cli"));
}

#[test]
fn enforcement_table_marks_only_strict_entries() {
    let strictness = Strictness::parse_features_toml(
        r#"
schema = "sim.features"

[enforcement.audience]
user = "strict"
code = "advisory"

[enforcement.surface]
cli = "strict"
view = "advisory"
"#,
    )
    .expect("parse enforcement");

    assert!(strictness.strict_audiences.contains("user"));
    assert!(strictness.strict_surfaces.contains("cli"));
    assert!(!strictness.strict_audiences.contains("code"));
}

#[test]
fn duplicate_claim_across_features_fails() {
    let mut doc = base_doc();
    doc.features
        .push(feature("feature/demo/one", &["cli/demo"], &[], &[]));
    doc.features
        .push(feature("feature/demo/two", &["cli/demo"], &[], &[]));

    let err = check_coverage_with_feature_audiences(&doc, &Strictness::default(), &BTreeMap::new())
        .unwrap_err();

    assert!(err.contains("duplicate claim: surface cli/demo"));
}

#[test]
fn missing_strict_cli_surface_fails() {
    let doc = base_doc();
    let mut strictness = Strictness::default();
    strictness.apply_strict_selectors("surface:cli").unwrap();

    let err =
        check_coverage_with_feature_audiences(&doc, &strictness, &BTreeMap::new()).unwrap_err();

    assert!(err.contains("unindexed: surface cli/demo"));
}

#[test]
fn strict_code_requires_reusable_code_anchors() {
    let mut doc = base_doc();
    doc.anchors.push(DiscoveredAnchor {
        id: AnchorId::new("anchor/crate/demo"),
        subject: SubjectId::new("crate/demo"),
        kind: "crate".to_owned(),
    });
    doc.anchors.push(DiscoveredAnchor {
        id: AnchorId::new("anchor/export/demo/runtime/install"),
        subject: SubjectId::new("crate/demo"),
        kind: "export".to_owned(),
    });
    doc.anchors.push(DiscoveredAnchor {
        id: AnchorId::new("anchor/rustdoc/demo/helper"),
        subject: SubjectId::new("crate/demo"),
        kind: "rustdoc-item".to_owned(),
    });
    doc.features.push(feature(
        "feature/demo/crate",
        &[],
        &[],
        &["anchor/crate/demo"],
    ));
    let mut strictness = Strictness::default();
    strictness.apply_strict_selectors("audience:code").unwrap();

    let err =
        check_coverage_with_feature_audiences(&doc, &strictness, &BTreeMap::new()).unwrap_err();

    assert!(err.contains("unindexed: anchor anchor/export/demo/runtime/install"));
    assert!(!err.contains("anchor/rustdoc/demo/helper"));
}

#[test]
fn advisory_specimen_gap_is_reported() {
    let report = check_coverage_with_feature_audiences(
        &base_doc(),
        &Strictness::default(),
        &BTreeMap::new(),
    )
    .expect("coverage report");

    assert!(
        report
            .advisory_missing
            .iter()
            .any(|item| { item.kind == ClaimKind::Specimen && item.id == "recipe/demo/hello" })
    );
}

#[test]
fn strict_user_feature_without_runnable_specimen_fails() {
    let mut doc = base_doc();
    doc.features
        .push(feature("feature/demo/user", &[], &[], &[]));
    let mut audiences = BTreeMap::new();
    audiences.insert(
        "feature/demo/user".to_owned(),
        BTreeSet::from(["user".to_owned()]),
    );
    let mut strictness = Strictness::default();
    strictness.apply_strict_selectors("specimen:user").unwrap();

    let err = check_coverage_with_feature_audiences(&doc, &strictness, &audiences).unwrap_err();

    assert!(err.contains("feature without runnable specimen: feature/demo/user"));
}

#[test]
fn strict_framework_feature_accepts_checked_runnable_specimen() {
    let mut doc = base_doc();
    doc.features.push(feature(
        "feature/demo/framework",
        &[],
        &["recipe/demo/hello"],
        &[],
    ));
    let mut audiences = BTreeMap::new();
    audiences.insert(
        "feature/demo/framework".to_owned(),
        BTreeSet::from(["framework".to_owned()]),
    );
    let mut strictness = Strictness::default();
    strictness
        .apply_strict_selectors("specimen:framework")
        .unwrap();

    check_coverage_with_feature_audiences(&doc, &strictness, &audiences)
        .expect("runnable specimen satisfies strict framework coverage");
}

fn base_doc() -> IndexDoc {
    IndexDoc {
        schema: "sim.index".to_owned(),
        generated_by: "test".to_owned(),
        visibility: Visibility::Public,
        subjects: vec![SubjectRecord {
            id: SubjectId::new("crate/demo"),
            kind: "crate".to_owned(),
            title: "demo".to_owned(),
        }],
        anchors: vec![DiscoveredAnchor {
            id: AnchorId::new("anchor/cli/demo"),
            subject: SubjectId::new("crate/demo"),
            kind: "cli-verb".to_owned(),
        }],
        surfaces: vec![DiscoveredSurface {
            id: SurfaceId::new("cli/demo"),
            subject: SubjectId::new("crate/demo"),
            kind: "cli".to_owned(),
        }],
        specimens: vec![DiscoveredSpecimen {
            id: sim_index_core::SpecimenId::new("recipe/demo/hello"),
            subject: SubjectId::new("crate/demo"),
            kind: "recipe".to_owned(),
            path: "recipes/hello/recipe.toml".to_owned(),
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

fn feature(id: &str, surfaces: &[&str], specimens: &[&str], anchors: &[&str]) -> FeatureRecord {
    FeatureRecord {
        id: FeatureId::new(id),
        key: CanonicalFeatureKey::new(format!("crate/demo/{}", id.replace('/', "-"))),
        subject: SubjectId::new("crate/demo"),
        title: id.to_owned(),
        summary: "Demo feature.".to_owned(),
        anchors: anchors.iter().map(|id| AnchorId::new(*id)).collect(),
        surfaces: surfaces.iter().map(|id| SurfaceId::new(*id)).collect(),
        specimens: specimens
            .iter()
            .map(|id| sim_index_core::SpecimenId::new(*id))
            .collect(),
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    }
}
