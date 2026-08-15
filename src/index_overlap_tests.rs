use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use sim_index_core::{
    AnchorId, CanonicalFeatureKey, DeclarationFact, DiscoveredAnchor, FeatureId, FeatureRecord,
    IndexDoc, IndexEdge, ProtocolRelation, SourceLocation, SubjectId, SubjectRecord, SyntaxBound,
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
fn classified_source_members_resolve_without_graph_findings() {
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

    assert!(
        findings.is_empty(),
        "classified source rows are consumed after the fail-closed source report"
    );
    fixture.cleanup();
}

#[test]
fn mapped_candidate_members_fail_the_complete_board() {
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

    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|finding| finding.strict));
    assert!(
        findings
            .iter()
            .all(|finding| finding.reason == "unresolved-candidate")
    );
    fixture.cleanup();
}

#[test]
fn mapped_candidate_without_feature_is_unresolved_finding() {
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
    assert_eq!(findings[0].reason, "unresolved-candidate");
    assert!(findings[0].strict);
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
            .all(|finding| finding.reason == "unresolved-candidate")
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

#[test]
fn protocol_roles_raise_real_gaps_and_reject_policy_categories() {
    let mut doc = IndexDoc::public("protocol-role-fixture");
    add_protocol_owner(&mut doc, "function", "sim_kernel::Function");
    add_protocol_owner(&mut doc, "class", "sim_kernel::Class");
    for (name, protocol, depends, implements) in [
        ("JavascriptFunction", "function", true, false),
        ("PythonClass", "class", true, false),
        ("LuaClosure", "function", true, true),
        ("TypeclassDictionary", "function", false, false),
        ("PrologPredicate", "function", false, false),
        ("JavascriptPrototype", "class", false, false),
        ("LuaMetatable", "class", false, false),
    ] {
        add_role_member(&mut doc, name, protocol, depends, implements);
    }

    let (findings, classification) = protocol_role_findings(&doc);

    assert_eq!(classification.configured_members, 7);
    assert_eq!(classification.gap_members, 2);
    assert_eq!(classification.satisfied_members, 1);
    assert_eq!(classification.policy_members, 4);
    assert_eq!(
        classification.policy_causes["no-protocol-owner-dependency"],
        4
    );
    let details = findings
        .iter()
        .map(|finding| finding.detail.as_str())
        .collect::<Vec<_>>();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("JavascriptFunction"))
    );
    assert!(details.iter().any(|detail| detail.contains("PythonClass")));
    for category_error in [
        "TypeclassDictionary",
        "PrologPredicate",
        "JavascriptPrototype",
        "LuaMetatable",
    ] {
        assert!(!details.iter().any(|detail| detail.contains(category_error)));
    }
}

#[test]
fn explicit_delegation_satisfies_a_protocol_role() {
    let mut doc = IndexDoc::public("protocol-role-delegation");
    add_protocol_owner(&mut doc, "function", "sim_kernel::Function");
    add_role_member(&mut doc, "IslispGeneric", "function", true, false);
    doc.edges.push(IndexEdge::new(
        "feature/member/IslispGeneric",
        "delegates-to",
        "feature/protocol/function",
    ));

    let (findings, classification) = protocol_role_findings(&doc);

    assert!(findings.is_empty());
    assert_eq!(classification.satisfied_members, 1);
}

#[test]
fn configured_protocol_coverage_is_finite_and_fail_closed() {
    let mut doc = IndexDoc::public("protocol-coverage");
    add_protocol_owner(&mut doc, "function", "sim_kernel::Function");
    add_role_member(&mut doc, "CoveredFunction", "function", true, true);
    add_role_member(&mut doc, "ExemptFunction", "function", true, true);
    add_role_member(&mut doc, "UncoveredFunction", "function", true, true);
    for name in ["ExemptFunction", "UncoveredFunction"] {
        let feature = doc
            .features
            .iter_mut()
            .find(|feature| feature.id.as_str() == format!("feature/member/{name}"))
            .unwrap();
        feature.anchors.clear();
    }
    let exempt_anchor = "anchor/member/ExemptFunction".to_owned();
    let mut policy = CoveragePolicy::default();
    policy.protocols.insert("sim_kernel::Function".to_owned());
    policy.exemptions.insert(
        exempt_anchor.clone(),
        policy::CoverageExemption {
            anchor: exempt_anchor,
            reason: "generated adapter is intentionally internal to its claimed facade".to_owned(),
        },
    );

    let (findings, classification) = protocol_coverage_findings(&doc, &policy);

    assert_eq!(classification.applicable, 3);
    assert_eq!(classification.covered, 1);
    assert_eq!(classification.exempt, 1);
    assert_eq!(classification.uncovered, 1);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].reason, "uncovered-protocol");
    assert!(findings[0].strict);
}

#[test]
fn stale_protocol_coverage_exemption_fails() {
    let mut policy = CoveragePolicy::default();
    policy.protocols.insert("sim_kernel::Function".to_owned());
    policy.exemptions.insert(
        "anchor/missing".to_owned(),
        policy::CoverageExemption {
            anchor: "anchor/missing".to_owned(),
            reason: "fixture proves exemptions cannot silently outlive source".to_owned(),
        },
    );

    let (findings, classification) =
        protocol_coverage_findings(&IndexDoc::public("empty"), &policy);

    assert_eq!(classification.applicable, 0);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].reason, "stale-coverage-exemption");
    assert!(findings[0].strict);
}

fn add_protocol_owner(doc: &mut IndexDoc, key: &str, protocol: &str) {
    let anchor = AnchorId::new(format!("anchor/protocol/{key}"));
    doc.declarations.push(declaration_fact(
        anchor.clone(),
        DeclarationRole::Trait,
        protocol,
    ));
    doc.features.push(feature_record(
        FeatureId::new(format!("feature/protocol/{key}")),
        anchor,
    ));
}

fn add_role_member(
    doc: &mut IndexDoc,
    name: &str,
    protocol: &str,
    depends: bool,
    implements: bool,
) {
    let anchor = AnchorId::new(format!("anchor/member/{name}"));
    let member = FeatureId::new(format!("feature/member/{name}"));
    let owner = format!("feature/protocol/{protocol}");
    doc.declarations.push(declaration_fact(
        anchor.clone(),
        DeclarationRole::Struct,
        name,
    ));
    doc.features
        .push(feature_record(member.clone(), anchor.clone()));
    doc.edges.push(IndexEdge::new(
        member.to_string(),
        "protocol-role",
        owner.clone(),
    ));
    if depends {
        doc.edges
            .push(IndexEdge::new(member.to_string(), "depends-on", owner));
    }
    if implements {
        let protocol_name = if protocol == "function" {
            "Function"
        } else {
            "Class"
        };
        doc.protocol_relations.push(ProtocolRelation {
            anchor,
            implementor: name.to_owned(),
            source_spelling: protocol_name.to_owned(),
            body_fingerprint: "fn invoke(&self) { shared_protocol(); }".to_owned(),
            body_bound: SyntaxBound {
                max_bytes: 16_384,
                truncated: false,
            },
            resolution: ProtocolResolution::Resolved {
                protocol: format!("sim_kernel::{protocol_name}"),
            },
        });
    }
}

fn declaration_fact(anchor: AnchorId, role: DeclarationRole, path: &str) -> DeclarationFact {
    DeclarationFact {
        anchor,
        role,
        module_path: path.to_owned(),
        generics: String::new(),
        members: Vec::new(),
        location: SourceLocation {
            file: "src/lib.rs".to_owned(),
            declaration: 0,
        },
        syntax_bound: SyntaxBound {
            max_bytes: 16_384,
            truncated: false,
        },
    }
}

fn feature_record(id: FeatureId, anchor: AnchorId) -> FeatureRecord {
    FeatureRecord {
        key: CanonicalFeatureKey::new(id.to_string()),
        subject: SubjectId::new(format!("subject/{}", id.as_str())),
        title: id.to_string(),
        summary: "protocol role fixture".to_owned(),
        id,
        anchors: vec![anchor],
        surfaces: Vec::new(),
        specimens: Vec::new(),
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    }
}

fn strict_options(clusters: Option<PathBuf>) -> OverlapOptions {
    OverlapOptions {
        input: PathBuf::from("index.sx"),
        clusters,
        policy: None,
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

    fn doc(&self, _with_edge: bool) -> IndexDoc {
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
