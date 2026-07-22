use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use sim_index_core::{
    AnchorId, CanonicalFeatureKey, DiscoveredAnchor, FeatureId, FeatureRecord, IndexDoc, IndexEdge,
    SubjectId, SubjectRecord,
};

use crate::index_overlap_report::read_overlap_report;

use super::*;

#[test]
fn strict_mode_requires_complete_cluster_report() {
    let root = temp_root("sim-tooling-overlap-required-report");
    let missing = root.join("missing.json");
    let wrong_schema = root.join("wrong.json");
    let incomplete = root.join("incomplete.json");
    let zero_roots = root.join("zero.json");
    fs::write(
        &wrong_schema,
        r#"{"schema":"other","complete":true,"roots_scanned":1,"clusters":[]}"#,
    )
    .unwrap();
    fs::write(
        &incomplete,
        r#"{"schema":"sim.overlap-report/v1","complete":false,"roots_scanned":1,"clusters":[]}"#,
    )
    .unwrap();
    fs::write(
        &zero_roots,
        r#"{"schema":"sim.overlap-report/v1","complete":true,"roots_scanned":0,"clusters":[]}"#,
    )
    .unwrap();

    let mut options = strict_options(None);
    assert!(
        read_overlap_report(options.clusters.as_ref(), options.strict)
            .unwrap_err()
            .contains("requires --clusters")
    );

    options.clusters = Some(missing);
    assert!(
        read_overlap_report(options.clusters.as_ref(), options.strict)
            .unwrap_err()
            .contains("read")
    );

    options.clusters = Some(wrong_schema);
    assert!(
        read_overlap_report(options.clusters.as_ref(), options.strict)
            .unwrap_err()
            .contains("expected sim.overlap-report/v1")
    );

    options.clusters = Some(incomplete);
    assert!(
        read_overlap_report(options.clusters.as_ref(), options.strict)
            .unwrap_err()
            .contains("not a complete overlap report")
    );

    options.clusters = Some(zero_roots);
    assert!(
        read_overlap_report(options.clusters.as_ref(), options.strict)
            .unwrap_err()
            .contains("scanned zero source roots")
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_members_resolve_through_repo_contracts_and_local_subjects() {
    let fixture = OverlapFixture::new("sim-tooling-overlap-source-members");
    let report_path = fixture.report(&json!({
        "schema": "sim.overlap-report/v1",
        "complete": true,
        "roots_scanned": 2,
        "clusters": [{
            "id": "sim-value/field-reader",
            "owner": "crate/sim-value",
            "replacement": "sim_value::access::field",
            "members": [
                delegated_member("sim-one", "crates/shared/src/lib.rs", 11),
                delegated_member("sim-two", "crates/shared/src/lib.rs", 13)
            ]
        }]
    }));
    let options = strict_options(Some(report_path));
    let report = read_overlap_report(options.clusters.as_ref(), options.strict).unwrap();
    let sources = SourceResolver::from_manifest(&fixture.root, &fixture.repos_manifest).unwrap();

    let findings = overlap_findings(&fixture.doc(false), &sources, &report.clusters);

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].reason, "missing-relating-edge");
    assert_eq!(findings[0].left.as_deref(), Some("feature/sim-one/shared"));
    assert_eq!(findings[0].right.as_deref(), Some("feature/sim-two/shared"));

    let reconciled = overlap_findings(&fixture.doc(true), &sources, &report.clusters);

    assert!(reconciled.is_empty());
    fixture.cleanup();
}

#[test]
fn mapped_candidate_members_remain_advisory() {
    let fixture = OverlapFixture::new("sim-tooling-overlap-candidate");
    let report_path = fixture.report(&json!({
        "schema": "sim.overlap-report/v1",
        "complete": true,
        "roots_scanned": 1,
        "clusters": [{
            "id": "sim-value/field-reader",
            "owner": "crate/sim-value",
            "replacement": "sim_value::access::field",
            "members": [
                candidate_member("sim-one", "crates/shared/src/lib.rs", 21),
                candidate_member("sim-two", "crates/shared/src/lib.rs", 23)
            ]
        }]
    }));
    let options = strict_options(Some(report_path));
    let report = read_overlap_report(options.clusters.as_ref(), options.strict).unwrap();
    let sources = SourceResolver::from_manifest(&fixture.root, &fixture.repos_manifest).unwrap();

    let findings = overlap_findings(&fixture.doc(false), &sources, &report.clusters);

    assert!(
        findings.is_empty(),
        "candidate source rows map but do not become strict graph findings"
    );
    fixture.cleanup();
}

#[test]
fn mapped_candidate_without_feature_is_advisory_finding() {
    let fixture = OverlapFixture::new("sim-tooling-overlap-unindexed-candidate");
    let report_path = fixture.report(&json!({
        "schema": "sim.overlap-report/v1",
        "complete": true,
        "roots_scanned": 1,
        "clusters": [{
            "id": "sim-kernel/test-cx",
            "owner": "crate/sim-kernel",
            "replacement": "sim_kernel::testing::bare_cx",
            "members": [candidate_member("sim-one", "crates/shared/src/lib.rs", 27)]
        }]
    }));
    let options = strict_options(Some(report_path));
    let report = read_overlap_report(options.clusters.as_ref(), options.strict).unwrap();
    let sources = SourceResolver::from_manifest(&fixture.root, &fixture.repos_manifest).unwrap();
    let mut doc = fixture.doc(false);
    doc.features.clear();

    let findings = overlap_findings(&doc, &sources, &report.clusters);

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].reason, "unindexed-source-member");
    assert!(!findings[0].strict);
    fixture.cleanup();
}

#[test]
fn unmapped_and_ambiguous_candidate_members_are_strict_findings() {
    let fixture = OverlapFixture::new("sim-tooling-overlap-unmapped");
    let report_path = fixture.report(&json!({
        "schema": "sim.overlap-report/v1",
        "complete": true,
        "roots_scanned": 1,
        "clusters": [{
            "id": "sim-value/field-reader",
            "owner": "crate/sim-value",
            "replacement": "sim_value::access::field",
            "members": [
                candidate_member("sim-one", "crates/missing/src/lib.rs", 31),
                candidate_member("sim-one", "crates/shared/src/lib.rs", 33)
            ]
        }]
    }));
    let options = strict_options(Some(report_path));
    let report = read_overlap_report(options.clusters.as_ref(), options.strict).unwrap();
    let sources = SourceResolver::from_manifest(&fixture.root, &fixture.repos_manifest).unwrap();
    let mut ambiguous = fixture.doc(false);
    ambiguous.subjects.push(SubjectRecord {
        id: SubjectId::new("crate/shared"),
        kind: "crate".to_owned(),
        title: "shared".to_owned(),
    });

    let findings = overlap_findings(&ambiguous, &sources, &report.clusters);

    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|finding| finding.strict));
    assert!(
        findings
            .iter()
            .any(|finding| finding.reason == "unmapped-source-member")
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.reason == "ambiguous-source-member")
    );
    fixture.cleanup();
}

#[test]
fn features_can_be_found_through_claimed_anchor_ownership() {
    let fixture = OverlapFixture::new("sim-tooling-overlap-anchor-owner");
    let report_path = fixture.report(&json!({
        "schema": "sim.overlap-report/v1",
        "complete": true,
        "roots_scanned": 1,
        "clusters": [{
            "id": "sim-value/field-reader",
            "owner": "crate/sim-value",
            "replacement": "sim_value::access::field",
            "members": [delegated_member("sim-one", "crates/shared/src/lib.rs", 41)]
        }]
    }));
    let options = strict_options(Some(report_path));
    let report = read_overlap_report(options.clusters.as_ref(), options.strict).unwrap();
    let sources = SourceResolver::from_manifest(&fixture.root, &fixture.repos_manifest).unwrap();
    let mut doc = IndexDoc::public("test");
    doc.subjects.push(SubjectRecord {
        id: SubjectId::new("local/sim-one/crate/shared"),
        kind: "crate".to_owned(),
        title: "shared".to_owned(),
    });
    doc.subjects.push(SubjectRecord {
        id: SubjectId::new("repo/sim-one"),
        kind: "repo".to_owned(),
        title: "sim-one".to_owned(),
    });
    doc.anchors.push(DiscoveredAnchor {
        id: AnchorId::new("anchor/sim-one/shared"),
        subject: SubjectId::new("local/sim-one/crate/shared"),
        kind: "rustdoc-item".to_owned(),
    });
    doc.features.push(FeatureRecord {
        id: FeatureId::new("feature/sim-one/claimed-anchor"),
        key: CanonicalFeatureKey::new("repo/sim-one/claimed-anchor"),
        subject: SubjectId::new("repo/sim-one"),
        title: "Claimed anchor".to_owned(),
        summary: "A feature owned through a claimed anchor.".to_owned(),
        anchors: vec![AnchorId::new("anchor/sim-one/shared")],
        surfaces: Vec::new(),
        specimens: Vec::new(),
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    });

    let features = member_features(
        &doc,
        &OwnerIndex::from_doc(&doc),
        &sources,
        &report.clusters[0].members[0],
    )
    .unwrap();

    assert!(features.contains("feature/sim-one/claimed-anchor"));
    fixture.cleanup();
}

fn strict_options(clusters: Option<PathBuf>) -> OverlapOptions {
    OverlapOptions {
        input: PathBuf::from("index.sx"),
        clusters,
        control_root: None,
        repos_manifest: None,
        json: false,
        strict: true,
    }
}

fn delegated_member(repo: &str, path: &str, line: u64) -> Value {
    member(
        repo,
        path,
        line,
        "delegated",
        Some("one-line delegation to the owner"),
    )
}

fn candidate_member(repo: &str, path: &str, line: u64) -> Value {
    member(repo, path, line, "candidate", None)
}

fn member(repo: &str, path: &str, line: u64, classification: &str, reason: Option<&str>) -> Value {
    json!({
        "repo": repo,
        "path": path,
        "line": line,
        "symbol": "fn field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr>",
        "classification": classification,
        "reason": reason,
        "owner": "crate/sim-value",
        "replacement": "sim_value::access::field"
    })
}

struct OverlapFixture {
    root: PathBuf,
    repos_manifest: PathBuf,
}

impl OverlapFixture {
    fn new(name: &str) -> Self {
        let root = temp_root(name);
        write_repo(&root, "sim-one");
        write_repo(&root, "sim-two");
        let repos_manifest = root.join("repos.toml");
        fs::write(
            &repos_manifest,
            "[[repo]]
name = \"sim-one\"
contains_code = true
local_path = \"sim-one\"

[[repo]]
name = \"sim-two\"
contains_code = true
local_path = \"sim-two\"
",
        )
        .unwrap();
        Self {
            root,
            repos_manifest,
        }
    }

    fn report(&self, value: &Value) -> PathBuf {
        let path = self.root.join("report.json");
        fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap();
        path
    }

    fn doc(&self, with_edge: bool) -> IndexDoc {
        let mut doc = IndexDoc::public("test");
        for repo in ["sim-one", "sim-two"] {
            let subject = format!("local/{repo}/crate/shared");
            let feature = format!("feature/{repo}/shared");
            doc.subjects.push(SubjectRecord {
                id: SubjectId::new(&subject),
                kind: "crate".to_owned(),
                title: "shared".to_owned(),
            });
            doc.features.push(FeatureRecord {
                id: FeatureId::new(&feature),
                key: CanonicalFeatureKey::new(format!("{subject}/shared")),
                subject: SubjectId::new(&subject),
                title: format!("{repo} shared"),
                summary: "A shared helper feature.".to_owned(),
                anchors: Vec::new(),
                surfaces: Vec::new(),
                specimens: Vec::new(),
                grammar_contracts: Vec::new(),
                doc_anchor: None,
            });
        }
        if with_edge {
            doc.edges.push(IndexEdge::new(
                "feature/sim-one/shared",
                "supports",
                "feature/sim-two/shared",
            ));
        }
        doc
    }

    fn cleanup(self) {
        fs::remove_dir_all(self.root).unwrap();
    }
}

fn write_repo(root: &Path, repo: &str) {
    let repo_root = root.join(repo);
    fs::create_dir_all(repo_root.join("docs/generated")).unwrap();
    fs::create_dir_all(repo_root.join("crates/shared/src")).unwrap();
    fs::write(repo_root.join("crates/shared/src/lib.rs"), "").unwrap();
    fs::write(
        repo_root.join("docs/generated/repo-contract.json"),
        r#"{
  "schema": "sim.repo-contract.v1",
  "packages": [
    { "name": "shared", "root": "crates/shared" }
  ]
}
"#,
    )
    .unwrap();
}

fn temp_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
