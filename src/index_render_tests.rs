use std::{
    collections::BTreeSet,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::content_digest::content_digest;
use sim_index_core::{
    CanonicalFeatureKey, DiscoveredSpecimen, DiscoveredSurface, FeatureId, FeatureRecord, IndexDoc,
    RouteId, RouteRecord, RouteStep, SpecimenId, SubjectId, SubjectRecord, SurfaceId, Visibility,
};

use super::*;

// conformance: generated docs render feature and specimen index pages.

#[test]
fn render_emits_named_pages_and_cards() {
    let doc = fixture_doc();
    let files = expected_files(&doc).unwrap();
    let names = files
        .iter()
        .map(|file| file.path_str().to_owned())
        .collect::<BTreeSet<_>>();

    for name in [
        "overview.md",
        "features.md",
        "features/feature--demo.md",
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
            .find(|file| file.path_str() == "index.cards.jsonl")
            .unwrap()
            .bytes
            .windows("\"kind\":\"feature\"".len())
            .any(|window| window == b"\"kind\":\"feature\"")
    );
    assert!(
        files
            .iter()
            .find(|file| file.path_str() == "index.cards.jsonl")
            .unwrap()
            .bytes
            .windows("\"kind\":\"surface\"".len())
            .any(|window| window == b"\"kind\":\"surface\"")
    );
}

#[test]
fn render_fixture_artifacts_keep_golden_bytes() {
    let files = expected_files(&fixture_doc()).unwrap();
    let actual = files
        .iter()
        .map(|file| (file.path_str(), content_digest(&file.bytes)))
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        [
            (
                "code.md",
                "e022109a6600a6e1c1d18a8243dfa88681f1176a329a9035049760ab76830cb6",
            ),
            (
                "features.md",
                "15d8234fb8773a48170e38f0d71d7128fba60fcc9d8f75fe9b1c63dc739f4e72",
            ),
            (
                "features/feature--demo.md",
                "dffef8ad177094ee9831dafb70c5caa8cbde22121659211f14290e982db5f75c",
            ),
            (
                "frameworks.md",
                "96c89d34cb6527371c1166c040337bca2a2afd6094ebe08b8552f00bcb9c228e",
            ),
            (
                "index.cards.jsonl",
                "0f5777ca1f453214913f96d8187b870328636d436bb73a4e320cd5c6af549602",
            ),
            (
                "index.json",
                "842243a848455faacf488ab22b8230232776f5ec154d8e7a8b2eb391a5f95c9e",
            ),
            (
                "languages.md",
                "a8c0fc8311004c40ebf6882176c7007c182af686c37b187caef58f2eb2213c7b",
            ),
            (
                "overview.md",
                "d10f83ae1bbfbbe15f7b6036315f5ed2a6338563e9270ccfecbaacb531679836",
            ),
            (
                "packages.md",
                "0d1c568ec3b31ee01b3b2c54ef31d484c3f985e2c4f6b7a349eaa8843b6bb4a8",
            ),
            (
                "routes.md",
                "9e832e87fcc6b2e02b523f9d552c33e6f023fa5b97dba3510c2bf83afb5a76d3",
            ),
            (
                "specimens.md",
                "9f0a03d319f8938f32a25dc6a1719a6b577963a030c83fa47bfb4a4d0e7d9e3d",
            ),
            (
                "surfaces.md",
                "ab9e9ca85786f0dca18909a934b9b9e444405744880daa02b27f13756736f00b",
            ),
            (
                "user.md",
                "87aaeeac4454c0e39f946f06611a27d22e36c4e8e68c07bd923e0b1ca77f1b2b",
            ),
        ]
        .map(|(path, digest)| (path, digest.to_owned()))
        .to_vec()
    );
}

#[test]
fn render_check_keeps_the_stale_message_contract() {
    let root = temp_root();
    let files = ArtifactSet::new(vec![
        GeneratedArtifact::new("code.md", b"current\n".to_vec()).unwrap(),
        GeneratedArtifact::new("user.md", b"current\n".to_vec()).unwrap(),
    ])
    .unwrap();

    assert_eq!(
        write_or_check_files(&root, &files, true).unwrap_err(),
        "stale generated index artifacts: user.md, code.md; run `sh bin/simctl index`"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn render_write_cleans_only_the_feature_page_scope() {
    let root = temp_root();
    fs::create_dir_all(root.join("features")).unwrap();
    fs::write(root.join("features/old.md"), "old\n").unwrap();
    fs::write(root.join("sibling.md"), "keep\n").unwrap();
    let files = ArtifactSet::new(vec![
        GeneratedArtifact::new("features/current.md", b"current\n".to_vec()).unwrap(),
    ])
    .unwrap();

    write_or_check_files(&root, &files, false).unwrap();

    assert!(!root.join("features/old.md").exists());
    assert_eq!(
        fs::read(root.join("features/current.md")).unwrap(),
        b"current\n"
    );
    assert_eq!(fs::read(root.join("sibling.md")).unwrap(), b"keep\n");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn route_page_keeps_single_trailing_newline_without_coverage_gaps() {
    let mut doc = fixture_doc();
    doc.routes.push(RouteRecord {
        id: RouteId::new("route/open-demo"),
        title: "Open demo".to_owned(),
        audiences: vec!["user".to_owned()],
        steps: vec![RouteStep::Feature {
            id: FeatureId::new("feature/demo"),
            why: "The demo feature opens the route.".to_owned(),
        }],
        doc_anchor: None,
    });

    let page = routes_page(&doc);

    assert!(page.ends_with('\n'));
    assert!(!page.ends_with("\n\n"));
}

fn temp_root() -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sim-tooling-index-render-{unique}"));
    fs::create_dir_all(&root).unwrap();
    root
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
