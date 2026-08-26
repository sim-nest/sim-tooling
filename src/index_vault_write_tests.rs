use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    generated_artifact::{ArtifactSet, GeneratedArtifact},
    generated_namespace::{ManagedNamespace, MigrationFault, MigrationRequest},
    index_vault_manifest::{MANIFEST_FILE, VaultManifest, VaultManifestSeed, sha256_digest},
};
use sim_codec_index_vault::{VaultEncoder, legacy_projection_v1, resolve_profile};
use sim_index_core::IndexDoc;
use sim_index_vault_core::{VaultGranularity, VaultProjection};

// conformance: managed vault namespaces reject unsafe state before replacing generated notes.

static TEMP_ID: AtomicUsize = AtomicUsize::new(0);

#[test]
fn plan_rejects_traversal_and_does_not_write() {
    let root = TempRoot::new("plan");
    fs::write(root.path().join("User.md"), b"user\n").unwrap();
    assert!(ManagedNamespace::open(root.path(), "../SIM-Index").is_err());
    assert!(ManagedNamespace::open(root.path(), "C:/SIM-Index").is_err());
    assert!(GeneratedArtifact::new("../escape.md", b"x".to_vec()).is_err());

    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    let before = root_entries(root.path());
    let plan = namespace.plan(
        &seed("portable-markdown-v2"),
        &artifacts(&[("README.md", "hi\n")]),
    );

    assert_eq!(plan.namespace, "SIM-Index");
    assert_eq!(plan.artifact_count, 1);
    assert_eq!(plan.byte_count, 3);
    assert_eq!(root_entries(root.path()), before);
    assert!(!root.path().join("SIM-Index").exists());
}

#[test]
fn commit_writes_owned_namespace_and_preserves_siblings() {
    let root = TempRoot::new("commit");
    fs::write(root.path().join("User.md"), b"user\n").unwrap();
    fs::create_dir(root.path().join("Notes")).unwrap();
    fs::write(root.path().join("Notes/own.md"), b"mine\n").unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    let set = artifacts(&[
        ("README.md", "readme\n"),
        ("Features/feature~demo.md", "feature\n"),
    ]);

    namespace
        .preflight(&seed("portable-markdown-v2"), &set)
        .unwrap()
        .commit()
        .unwrap();

    assert_eq!(
        fs::read_to_string(root.path().join("User.md")).unwrap(),
        "user\n"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("Notes/own.md")).unwrap(),
        "mine\n"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("SIM-Index/README.md")).unwrap(),
        "readme\n"
    );
    let manifest = read_manifest(root.path());
    assert_eq!(manifest.namespace, "SIM-Index");
    assert_eq!(manifest.profile, "portable-markdown-v2");
    assert_eq!(manifest.artifacts.len(), 2);
    namespace
        .check(&seed("portable-markdown-v2"), &set)
        .unwrap();
}

#[test]
fn preflight_refuses_unowned_wrong_profile_changed_missing_and_foreign_state() {
    let root = TempRoot::new("conflicts");
    fs::create_dir(root.path().join("SIM-Index")).unwrap();
    fs::write(root.path().join("SIM-Index/manual.md"), b"manual\n").unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    assert_contains(
        namespace
            .preflight(
                &seed("portable-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "no ownership manifest",
    );

    let root = committed_root("wrong-profile", &[("README.md", "old\n")]);
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    assert_contains(
        namespace
            .preflight(
                &seed("obsidian-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "profile",
    );

    let root = committed_root("edited", &[("README.md", "old\n")]);
    fs::write(root.path().join("SIM-Index/README.md"), b"user edit\n").unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    assert_contains(
        namespace
            .preflight(
                &seed("portable-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "changed",
    );

    let root = committed_root("missing", &[("README.md", "old\n")]);
    fs::remove_file(root.path().join("SIM-Index/README.md")).unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    assert_contains(
        namespace
            .preflight(
                &seed("portable-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "missing",
    );

    let root = committed_root("foreign", &[("README.md", "old\n")]);
    fs::write(root.path().join("SIM-Index/foreign.md"), b"mine\n").unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    assert_contains(
        namespace
            .preflight(
                &seed("portable-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "foreign",
    );
}

#[test]
fn check_reports_stale_without_writing() {
    let root = committed_root("stale", &[("README.md", "old\n")]);
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    let replacement = artifacts(&[("README.md", "new\n")]);

    assert_contains(
        namespace
            .check(&seed("portable-markdown-v2"), &replacement)
            .unwrap_err(),
        "stale",
    );
    assert_eq!(
        fs::read_to_string(root.path().join("SIM-Index/README.md")).unwrap(),
        "old\n"
    );
}

#[test]
fn concurrent_manifest_change_blocks_commit_before_writes() {
    let root = committed_root("concurrent", &[("README.md", "old\n")]);
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    let transaction = namespace
        .preflight(
            &seed("portable-markdown-v2"),
            &artifacts(&[("README.md", "new\n")]),
        )
        .unwrap();
    fs::write(
        root.path().join("SIM-Index").join(MANIFEST_FILE),
        b"{\"schema\":\"other\"}\n",
    )
    .unwrap();

    assert!(transaction.commit().is_err());
    assert!(!root.path().join(".SIM-Index.sim-stage").exists());
    assert_eq!(
        fs::read_to_string(root.path().join("SIM-Index/README.md")).unwrap(),
        "old\n"
    );
}

#[test]
fn interrupted_stage_and_recovery_are_reported_without_cleanup() {
    let root = TempRoot::new("interrupted");
    fs::create_dir(root.path().join(".SIM-Index.sim-stage")).unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    assert_contains(
        namespace
            .preflight(
                &seed("portable-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "interrupted",
    );
    assert!(root.path().join(".SIM-Index.sim-stage").exists());

    fs::remove_dir(root.path().join(".SIM-Index.sim-stage")).unwrap();
    fs::create_dir(root.path().join(".SIM-Index.sim-recovery")).unwrap();
    assert_contains(
        namespace
            .preflight(
                &seed("portable-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "interrupted",
    );
    assert!(root.path().join(".SIM-Index.sim-recovery").exists());
}

#[test]
fn injected_rename_failure_keeps_stage_and_recovery() {
    let root = committed_root("rename-failure", &[("README.md", "old\n")]);
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    let transaction = namespace
        .preflight(
            &seed("portable-markdown-v2"),
            &artifacts(&[("README.md", "new\n")]),
        )
        .unwrap();

    assert_contains(
        transaction
            .commit_with_injected_recovery_failure()
            .unwrap_err(),
        "injected rename failure",
    );
    assert!(!root.path().join("SIM-Index").exists());
    assert_eq!(
        fs::read_to_string(root.path().join(".SIM-Index.sim-recovery/README.md")).unwrap(),
        "old\n"
    );
    assert_eq!(
        fs::read_to_string(root.path().join(".SIM-Index.sim-stage/README.md")).unwrap(),
        "new\n"
    );
}

#[test]
fn case_fold_collisions_are_rejected() {
    assert!(
        ArtifactSet::new(vec![
            GeneratedArtifact::new("Notes/A.md", b"a".to_vec()).unwrap(),
            GeneratedArtifact::new("notes/a.md", b"b".to_vec()).unwrap(),
        ])
        .is_err()
    );

    let root = TempRoot::new("case-fold");
    fs::create_dir(root.path().join("SIM-Index")).unwrap();
    fs::write(root.path().join("SIM-Index/A.md"), b"a\n").unwrap();
    fs::write(root.path().join("SIM-Index/a.md"), b"b\n").unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    assert_contains(
        namespace
            .preflight(
                &seed("portable-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "case-fold collision",
    );
}

#[test]
fn v1_requires_explicit_migration_and_migration_is_idempotent() {
    let root = TempRoot::new("v1-migration");
    let target = root.path().join("SIM-Index");
    fs::create_dir(&target).unwrap();
    let legacy_readme = b"---\nsim_profile: \"portable-markdown-v1\"\ngranularity: \"compact\"\nschema: \"sim.index/v1\"\ngenerated-by: \"fixture\"\n---\n\n# SIM Index Vault\n\n## Navigation\n";
    fs::write(target.join("README.md"), legacy_readme).unwrap();
    let digest = sha256_digest(legacy_readme);
    let manifest = serde_json::json!({
        "schema": "sim.index-vault-manifest.v1",
        "namespace": "SIM-Index",
        "profile": "portable-markdown-v1",
        "granularity": "compact",
        "index_digest": "sha256:7040c16de1e23dddf77df8ff8043c2bee23b42b47a0f326e5e124ae9bc2178e0",
        "coverage": {"features": 1},
        "artifacts": {"README.md": digest},
    });
    fs::write(
        target.join(MANIFEST_FILE),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    let doc = IndexDoc::public("migration-fixture");
    let projection = VaultProjection::from_complete(&doc, VaultGranularity::Compact).unwrap();
    let legacy_projection = legacy_projection_v1(&doc, VaultGranularity::Compact).unwrap();
    let bundle = VaultEncoder::new(resolve_profile("portable-markdown-v2").unwrap())
        .encode(&projection)
        .unwrap();
    let next = ArtifactSet::new(
        bundle
            .entries
            .iter()
            .map(|entry| GeneratedArtifact::new(&entry.path, entry.bytes.clone()).unwrap())
            .collect(),
    )
    .unwrap();
    assert_contains(
        namespace
            .diff(&seed("portable-markdown-v2"), &next)
            .unwrap_err(),
        "--migrate-profile",
    );
    let changed = namespace
        .migrate_v1(
            "portable-markdown-v1",
            &seed("portable-markdown-v2"),
            &next,
            &legacy_projection,
            &bundle,
            &projection,
        )
        .unwrap();
    assert_eq!(changed.changed_artifacts, 1);
    assert_eq!(
        fs::read(target.join("README.md")).unwrap(),
        bundle
            .entries
            .iter()
            .find(|entry| entry.path == "README.md")
            .unwrap()
            .bytes
    );
    assert!(
        fs::read_to_string(target.join(MANIFEST_FILE))
            .unwrap()
            .contains("manifest.v2")
    );
    let again = namespace
        .migrate_v1(
            "portable-markdown-v1",
            &seed("portable-markdown-v2"),
            &next,
            &legacy_projection,
            &bundle,
            &projection,
        )
        .unwrap();
    assert_eq!(again.changed_artifacts, 0);
    assert_eq!(root_entries(root.path()), vec!["SIM-Index"]);
}

#[test]
fn every_migration_failpoint_reopens_to_a_classified_state() {
    let points = [
        MigrationFault::BeforeManifestRead,
        MigrationFault::AfterManifestRead,
        MigrationFault::AfterSourceValidation,
        MigrationFault::AfterLegacyVerify,
        MigrationFault::BeforeStageWrite,
        MigrationFault::AfterStageWrite,
        MigrationFault::AfterStageVerify,
        MigrationFault::BeforeRecoveryRename,
        MigrationFault::AfterRecoveryRename,
        MigrationFault::BeforeLiveRename,
        MigrationFault::AfterLiveRename,
        MigrationFault::BeforeManifestReadback,
        MigrationFault::AfterManifestReadback,
        MigrationFault::BeforeRecoveryCleanup,
        MigrationFault::AfterRecoveryCleanup,
    ];
    for point in points {
        let (root, next, legacy_projection, bundle, projection) = legacy_migration_fixture();
        fs::write(root.path().join("User.md"), b"sibling\n").unwrap();
        let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
        let error = namespace
            .migrate_v1_with_fault(
                MigrationRequest::new(
                    "portable-markdown-v1",
                    &seed("portable-markdown-v2"),
                    &next,
                    &legacy_projection,
                    &bundle,
                    &projection,
                ),
                point,
            )
            .unwrap_err();
        assert_contains(error, "failpoint");
        assert_eq!(fs::read(root.path().join("User.md")).unwrap(), b"sibling\n");
        let live = root.path().join("SIM-Index");
        let stage = root.path().join(".SIM-Index.sim-stage");
        let recovery = root.path().join(".SIM-Index.sim-recovery");
        assert!(
            (live.exists() && !recovery.exists())
                || (!live.exists() && stage.exists() && recovery.exists())
                || (live.exists() && recovery.exists()),
            "unclassified migration state at {point:?}"
        );
    }
}

fn legacy_migration_fixture() -> (
    TempRoot,
    ArtifactSet,
    VaultProjection,
    sim_codec_index_vault::VaultBundle,
    VaultProjection,
) {
    let root = TempRoot::new("migration-failpoint");
    let target = root.path().join("SIM-Index");
    fs::create_dir(&target).unwrap();
    let legacy_readme = b"---\nsim_profile: \"portable-markdown-v1\"\ngranularity: \"compact\"\nschema: \"sim.index/v1\"\ngenerated-by: \"fixture\"\n---\n\n# SIM Index Vault\n\n## Navigation\n";
    fs::write(target.join("README.md"), legacy_readme).unwrap();
    let manifest = serde_json::json!({
        "schema": "sim.index-vault-manifest.v1",
        "namespace": "SIM-Index",
        "profile": "portable-markdown-v1",
        "granularity": "compact",
        "index_digest": "sha256:7040c16de1e23dddf77df8ff8043c2bee23b42b47a0f326e5e124ae9bc2178e0",
        "coverage": {},
        "artifacts": {"README.md": sha256_digest(legacy_readme)},
    });
    fs::write(
        target.join(MANIFEST_FILE),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
    let doc = IndexDoc::public("migration-fixture");
    let projection = VaultProjection::from_complete(&doc, VaultGranularity::Compact).unwrap();
    let legacy_projection = legacy_projection_v1(&doc, VaultGranularity::Compact).unwrap();
    let bundle = VaultEncoder::new(resolve_profile("portable-markdown-v2").unwrap())
        .encode(&projection)
        .unwrap();
    let next = ArtifactSet::new(
        bundle
            .entries
            .iter()
            .map(|entry| GeneratedArtifact::new(&entry.path, entry.bytes.clone()).unwrap())
            .collect(),
    )
    .unwrap();
    (root, next, legacy_projection, bundle, projection)
}

#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected() {
    use std::os::unix::fs::symlink;

    let root = TempRoot::new("symlink");
    fs::write(root.path().join("outside.md"), b"outside\n").unwrap();
    fs::create_dir(root.path().join("SIM-Index")).unwrap();
    symlink(
        root.path().join("outside.md"),
        root.path().join("SIM-Index/link.md"),
    )
    .unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();

    assert_contains(
        namespace
            .preflight(
                &seed("portable-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "symlink",
    );
}

#[cfg(unix)]
#[test]
fn interrupted_broken_stage_symlink_is_reported_without_cleanup() {
    use std::os::unix::fs::symlink;

    let root = TempRoot::new("broken-stage-symlink");
    symlink(
        root.path().join("missing-stage-target"),
        root.path().join(".SIM-Index.sim-stage"),
    )
    .unwrap();
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();

    assert_contains(
        namespace
            .preflight(
                &seed("portable-markdown-v2"),
                &artifacts(&[("README.md", "new\n")]),
            )
            .unwrap_err(),
        "interrupted",
    );
    assert!(fs::symlink_metadata(root.path().join(".SIM-Index.sim-stage")).is_ok());
}

fn committed_root(name: &str, files: &[(&str, &str)]) -> TempRoot {
    let root = TempRoot::new(name);
    let namespace = ManagedNamespace::open(root.path(), "SIM-Index").unwrap();
    let set = artifacts(files);
    namespace
        .preflight(&seed("portable-markdown-v2"), &set)
        .unwrap()
        .commit()
        .unwrap();
    root
}

fn artifacts(files: &[(&str, &str)]) -> ArtifactSet {
    ArtifactSet::new(
        files
            .iter()
            .map(|(path, text)| GeneratedArtifact::new(*path, text.as_bytes().to_vec()).unwrap())
            .collect(),
    )
    .unwrap()
}

fn seed(profile: &str) -> VaultManifestSeed {
    VaultManifestSeed::new(
        profile,
        "compact",
        "sha256:7040c16de1e23dddf77df8ff8043c2bee23b42b47a0f326e5e124ae9bc2178e0",
        BTreeMap::from([
            ("subjects".to_owned(), 1),
            ("anchors".to_owned(), 2),
            ("features".to_owned(), 3),
        ]),
        "sha256:7040c16de1e23dddf77df8ff8043c2bee23b42b47a0f326e5e124ae9bc2178e0",
        "sha256:7040c16de1e23dddf77df8ff8043c2bee23b42b47a0f326e5e124ae9bc2178e0",
    )
    .unwrap()
}

fn read_manifest(root: &Path) -> VaultManifest {
    VaultManifest::from_bytes(&fs::read(root.join("SIM-Index").join(MANIFEST_FILE)).unwrap())
        .unwrap()
}

fn root_entries(root: &Path) -> Vec<String> {
    let mut entries = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    entries.sort();
    entries
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
            "sim-tooling-index-vault-{label}-{}-{id}",
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
