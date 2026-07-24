use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use sim_codec_index::{IndexCodec, IndexForm};
use sim_index_core::{
    AnchorId, CanonicalFeatureKey, DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface,
    FeatureDraft, FeatureId, FeatureRecord, GrammarContract, IndexDoc, IndexEdge, RouteId,
    RouteRecord, RouteStep, SpecimenId, SubjectId, SubjectRecord, SurfaceId, Visibility,
};
use sim_kernel::EncodePosition;

use crate::{
    index_vault::{ExportMode, IndexExportOptions, export},
    index_vault_graph::VaultGranularity,
    index_vault_manifest::{MANIFEST_FILE, VaultManifest},
};

// conformance: `xtask index export` composes the checked Index vault graph,
// profile renderer, and managed namespace transaction without owning a second graph.

static TEMP_ID: AtomicUsize = AtomicUsize::new(0);

#[test]
fn export_options_require_explicit_input_profile_and_vault_root() {
    assert_contains(
        IndexExportOptions::parse(&args(&["index", "export"])).unwrap_err(),
        "requires --input",
    );
    assert_contains(
        IndexExportOptions::parse(&args(&[
            "index",
            "export",
            "--input",
            "index.sx",
            "--profile",
            "portable",
        ]))
        .unwrap_err(),
        "requires --vault-root",
    );

    let options = IndexExportOptions::parse(&args(&[
        "index",
        "export",
        "--input",
        "index.sx",
        "--profile",
        "portable",
        "--vault-root",
        "vault",
    ]))
    .unwrap();
    assert_eq!(options.namespace, PathBuf::from("SIM-Index"));
    assert_eq!(options.granularity, VaultGranularity::Compact);
    assert_eq!(options.mode, ExportMode::Write);
}

#[test]
fn export_options_reject_duplicate_and_conflicting_flags() {
    assert_contains(
        IndexExportOptions::parse(&args(&[
            "index",
            "export",
            "--input",
            "a.sx",
            "--input",
            "b.sx",
            "--profile",
            "portable",
            "--vault-root",
            "vault",
        ]))
        .unwrap_err(),
        "duplicate",
    );
    assert_contains(
        IndexExportOptions::parse(&args(&[
            "index",
            "export",
            "--input",
            "index.sx",
            "--profile",
            "portable",
            "--vault-root",
            "vault",
            "--plan",
            "--check",
        ]))
        .unwrap_err(),
        "mutually exclusive",
    );
    assert_contains(
        IndexExportOptions::parse(&args(&[
            "index",
            "export",
            "--input",
            "index.sx",
            "--profile",
            "portable",
            "--vault-root",
            "vault",
            "--granularity",
            "wide",
        ]))
        .unwrap_err(),
        "granularity",
    );
}

#[test]
fn all_profiles_export_checked_fixture_and_preserve_sibling_notes() {
    let input = encoded_fixture(Visibility::Public);

    for profile in ["portable", "obsidian", "seqlog", "logseq"] {
        let root = TempRoot::new(profile);
        fs::write(root.path().join("User.md"), b"user note\n").unwrap();

        let plan = export(options(
            &input.path().join("index.sx"),
            root.path(),
            profile,
            ExportMode::Plan,
            VaultGranularity::Compact,
        ))
        .unwrap();
        assert_eq!(plan.mode, ExportMode::Plan);
        assert!(plan.summary().contains("profile="));
        assert!(!root.path().join("SIM-Index").exists());

        let write = export(options(
            &input.path().join("index.sx"),
            root.path(),
            profile,
            ExportMode::Write,
            VaultGranularity::Compact,
        ))
        .unwrap();
        assert_eq!(write.mode, ExportMode::Write);
        assert!(write.changed_artifacts > 0);
        assert_eq!(write.unchanged_artifacts, 0);
        assert_eq!(
            fs::read_to_string(root.path().join("User.md")).unwrap(),
            "user note\n"
        );
        assert!(root.path().join("SIM-Index").join(MANIFEST_FILE).exists());

        let check = export(options(
            &input.path().join("index.sx"),
            root.path(),
            profile,
            ExportMode::Check,
            VaultGranularity::Compact,
        ))
        .unwrap();
        assert_eq!(check.changed_artifacts, 0);
        assert_eq!(check.unchanged_artifacts, write.artifact_count);
        assert_eq!(check.profile_id, write.profile_id);
    }
}

#[test]
fn repeated_write_is_a_noop_and_managed_edits_block_check_and_write() {
    let input = encoded_fixture(Visibility::Public);
    let root = TempRoot::new("repeat");
    let first = export(options(
        &input.path().join("index.sx"),
        root.path(),
        "obsidian",
        ExportMode::Write,
        VaultGranularity::Compact,
    ))
    .unwrap();
    assert!(first.changed_artifacts > 0);

    let second = export(options(
        &input.path().join("index.sx"),
        root.path(),
        "obsidian",
        ExportMode::Write,
        VaultGranularity::Compact,
    ))
    .unwrap();
    assert_eq!(second.changed_artifacts, 0);
    assert_eq!(second.unchanged_artifacts, first.artifact_count);

    let manifest = read_manifest(root.path());
    assert_eq!(manifest.profile, "obsidian-markdown-v1");
    let readme = root.path().join("SIM-Index/README.md");
    fs::write(&readme, b"user edit\n").unwrap();

    assert_contains(
        export(options(
            &input.path().join("index.sx"),
            root.path(),
            "obsidian",
            ExportMode::Check,
            VaultGranularity::Compact,
        ))
        .unwrap_err(),
        "changed outside the exporter",
    );
    assert_contains(
        export(options(
            &input.path().join("index.sx"),
            root.path(),
            "obsidian",
            ExportMode::Write,
            VaultGranularity::Compact,
        ))
        .unwrap_err(),
        "changed outside the exporter",
    );
}

#[test]
fn full_granularity_and_private_local_visibility_are_checked() {
    let input = encoded_fixture(Visibility::Public);
    let root = TempRoot::new("full");
    let full = export(options(
        &input.path().join("index.sx"),
        root.path(),
        "portable",
        ExportMode::Write,
        VaultGranularity::Full,
    ))
    .unwrap();
    assert_eq!(full.granularity, "full");
    assert!(root.path().join("SIM-Index/anchors").exists());

    let private = encoded_fixture(Visibility::PrivateLocal);
    let err = export(options(
        &private.path().join("index.sx"),
        root.path(),
        "portable",
        ExportMode::Plan,
        VaultGranularity::Compact,
    ))
    .unwrap_err();
    assert_contains(err, "requires a public IndexDoc");
}

fn options(
    input: &Path,
    vault_root: &Path,
    profile: &str,
    mode: ExportMode,
    granularity: VaultGranularity,
) -> IndexExportOptions {
    IndexExportOptions {
        input: input.to_path_buf(),
        profile: profile.to_owned(),
        vault_root: vault_root.to_path_buf(),
        namespace: PathBuf::from("SIM-Index"),
        granularity,
        mode,
    }
}

fn encoded_fixture(visibility: Visibility) -> TempRoot {
    let root = TempRoot::new("index-input");
    let source = IndexCodec
        .encode(
            &fixture_doc(visibility),
            EncodePosition::Data,
            IndexForm::Sx,
        )
        .unwrap();
    fs::write(root.path().join("index.sx"), source).unwrap();
    root
}

fn fixture_doc(visibility: Visibility) -> IndexDoc {
    IndexDoc {
        schema: "sim.index".to_owned(),
        generated_by: "test".to_owned(),
        visibility,
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

fn args(parts: &[&str]) -> Vec<String> {
    std::iter::once("xtask".to_owned())
        .chain(parts.iter().map(|part| (*part).to_owned()))
        .collect()
}

fn read_manifest(root: &Path) -> VaultManifest {
    VaultManifest::from_bytes(&fs::read(root.join("SIM-Index").join(MANIFEST_FILE)).unwrap())
        .unwrap()
}

fn assert_contains(text: String, expected: &str) {
    assert!(
        text.contains(expected),
        "expected `{text}` to contain `{expected}`"
    );
}

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = TEMP_ID.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "sim-tooling-index-vault-cli-{label}-{}-{id}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).unwrap();
        }
        fs::create_dir(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
