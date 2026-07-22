//! Advisory duplicate-implementation bridge over the merged SIM Index graph.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use serde_json::{Value, json};
use sim_index_core::{FeatureRecord, IndexDoc, SubjectId};

use crate::{
    index_overlap_report::{
        CloneCluster, OverlapMember, SourceClassification, read_overlap_report,
    },
    index_render::load_doc,
    index_source::SourceResolver,
};

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = OverlapOptions::parse(&args)?;
    let report = read_overlap_report(options.clusters.as_ref(), options.strict)?;
    let doc = load_doc(&options.input)?;
    let sources = SourceResolver::from_options(
        options.control_root.as_deref(),
        options.repos_manifest.as_deref(),
    )?;
    let findings = overlap_findings(&doc, &sources, &report.clusters);
    let strict_findings = findings
        .iter()
        .filter(|finding| finding.strict)
        .collect::<Vec<_>>();
    if options.strict && !strict_findings.is_empty() {
        return Err(strict_error(&strict_findings));
    }
    if options.json {
        let text = serde_json::to_string_pretty(&json!({
            "advisory": !options.strict,
            "strict": options.strict,
            "report_complete": report.complete,
            "roots_scanned": report.roots_scanned,
            "cluster_count": report.clusters.len(),
            "finding_count": findings.len(),
            "strict_finding_count": strict_findings.len(),
            "findings": findings.iter().map(Finding::to_json).collect::<Vec<_>>(),
        }))
        .map_err(|err| format!("serialize overlap findings: {err}"))?;
        println!("{text}");
    } else if findings.is_empty() {
        println!(
            "index overlap: advisory ok ({} cluster(s), 0 missing feature relations)",
            report.clusters.len()
        );
    } else {
        for finding in &findings {
            println!("{}", finding.to_text(options.strict));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OverlapOptions {
    input: PathBuf,
    clusters: Option<PathBuf>,
    control_root: Option<PathBuf>,
    repos_manifest: Option<PathBuf>,
    json: bool,
    strict: bool,
}

impl OverlapOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if !matches!(args.get(1).map(String::as_str), Some("index"))
            || !matches!(args.get(2).map(String::as_str), Some("overlap"))
        {
            return Err(usage(program));
        }
        let mut input = None;
        let mut clusters = None;
        let mut control_root = None;
        let mut repos_manifest = None;
        let mut json = false;
        let mut strict = false;
        let mut index = 3;
        while index < args.len() {
            match args[index].as_str() {
                "--input" => {
                    index += 1;
                    input = Some(PathBuf::from(
                        args.get(index).ok_or("--input requires a path")?,
                    ));
                }
                "--clusters" => {
                    index += 1;
                    clusters = Some(PathBuf::from(
                        args.get(index).ok_or("--clusters requires a path")?,
                    ));
                }
                "--control-root" => {
                    index += 1;
                    control_root = Some(PathBuf::from(
                        args.get(index).ok_or("--control-root requires a path")?,
                    ));
                }
                "--repos-manifest" => {
                    index += 1;
                    repos_manifest = Some(PathBuf::from(
                        args.get(index).ok_or("--repos-manifest requires a path")?,
                    ));
                }
                "--json" => json = true,
                "--strict" => strict = true,
                "-h" | "--help" => return Err(usage(program)),
                other => {
                    return Err(format!(
                        "unknown index overlap argument `{other}`; {}",
                        usage(program)
                    ));
                }
            }
            index += 1;
        }
        Ok(Self {
            input: input
                .ok_or_else(|| format!("index overlap requires --input; {}", usage(program)))?,
            clusters,
            control_root,
            repos_manifest,
            json,
            strict,
        })
    }
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} index overlap --input <index.sx> [--clusters <report.json>] [--control-root <path> --repos-manifest <path>] [--json] [--strict]"
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Finding {
    cluster: String,
    member: Option<MemberRef>,
    left: Option<String>,
    right: Option<String>,
    source_classification: Option<String>,
    graph_relation: Option<String>,
    reason: String,
    detail: String,
    strict: bool,
}

impl Finding {
    fn source_member(
        cluster: &CloneCluster,
        member: &OverlapMember,
        reason: &str,
        detail: String,
        strict: bool,
    ) -> Self {
        Self {
            cluster: cluster.id.clone(),
            member: Some(MemberRef::from_member(member)),
            left: None,
            right: None,
            source_classification: Some(member.classification.as_str().to_owned()),
            graph_relation: None,
            reason: reason.to_owned(),
            detail,
            strict,
        }
    }

    fn missing_relation(cluster: &CloneCluster, left: String, right: String) -> Self {
        Self {
            cluster: cluster.id.clone(),
            member: None,
            left: Some(left),
            right: Some(right),
            source_classification: Some("classified".to_owned()),
            graph_relation: Some("missing-relating-edge".to_owned()),
            reason: "missing-relating-edge".to_owned(),
            detail: format!(
                "classified members for {} should relate through {}",
                cluster.owner, cluster.replacement
            ),
            strict: true,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "cluster": self.cluster,
            "member": self.member.as_ref().map(MemberRef::to_json),
            "features": match (&self.left, &self.right) {
                (Some(left), Some(right)) => json!([left, right]),
                _ => Value::Null,
            },
            "source_classification": self.source_classification,
            "graph_relation": self.graph_relation,
            "reason": self.reason,
            "detail": self.detail,
            "strict": self.strict,
        })
    }

    fn to_text(&self, strict: bool) -> String {
        let mode = if strict && self.strict {
            "strict"
        } else {
            "advisory"
        };
        match (&self.member, &self.left, &self.right) {
            (Some(member), _, _) => format!(
                "index overlap: {mode} {} {}:{} {} {}: {}",
                self.cluster,
                member.repo_path(),
                member.line,
                member.classification,
                self.reason,
                self.detail
            ),
            (_, Some(left), Some(right)) => format!(
                "index overlap: {mode} {} {} <-> {} {}",
                self.cluster, left, right, self.reason
            ),
            _ => format!("index overlap: {mode} {} {}", self.cluster, self.reason),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MemberRef {
    repo: String,
    path: String,
    line: u64,
    symbol: String,
    classification: String,
    owner: String,
    replacement: String,
}

impl MemberRef {
    fn from_member(member: &OverlapMember) -> Self {
        Self {
            repo: member.repo.clone(),
            path: member.path.clone(),
            line: member.line,
            symbol: member.symbol.clone(),
            classification: member.classification.as_str().to_owned(),
            owner: member.owner.clone(),
            replacement: member.replacement.clone(),
        }
    }

    fn repo_path(&self) -> String {
        format!("{}/{}", self.repo, self.path)
    }

    fn to_json(&self) -> Value {
        json!({
            "repo": self.repo,
            "path": self.path,
            "line": self.line,
            "symbol": self.symbol,
            "classification": self.classification,
            "owner": self.owner,
            "replacement": self.replacement,
        })
    }
}

fn overlap_findings(
    doc: &IndexDoc,
    sources: &SourceResolver,
    clusters: &[CloneCluster],
) -> Vec<Finding> {
    let owners = OwnerIndex::from_doc(doc);
    let mut findings = Vec::new();
    for cluster in clusters {
        let mut accepted_features = BTreeSet::<String>::new();
        for member in &cluster.members {
            if member.classification == SourceClassification::Regression {
                findings.push(Finding::source_member(
                    cluster,
                    member,
                    "source-regression",
                    "owned source family still has a hard regression".to_owned(),
                    true,
                ));
                continue;
            }
            match member_features(doc, &owners, sources, member) {
                Ok(features) if features.is_empty() => {
                    if should_allow_unmapped_keep(member) {
                        continue;
                    }
                    let strict = member.classification != SourceClassification::Candidate;
                    let reason = if strict {
                        "unmapped-source-member"
                    } else {
                        "unindexed-source-member"
                    };
                    findings.push(Finding::source_member(
                        cluster,
                        member,
                        reason,
                        "member resolved to a subject with no owning or claimed feature".to_owned(),
                        strict,
                    ));
                }
                Ok(features) => {
                    if member.classification.requires_graph_relation() {
                        accepted_features.extend(features);
                    }
                }
                Err(err) => {
                    if should_allow_unmapped_keep(member) {
                        continue;
                    }
                    let reason = if err.contains("multiple") {
                        "ambiguous-source-member"
                    } else {
                        "unmapped-source-member"
                    };
                    findings.push(Finding::source_member(cluster, member, reason, err, true));
                }
            }
        }
        for (left_index, left) in accepted_features.iter().enumerate() {
            for right in accepted_features.iter().skip(left_index + 1) {
                if !has_relating_edge(doc, left, right) {
                    findings.push(Finding::missing_relation(
                        cluster,
                        left.clone(),
                        right.clone(),
                    ));
                }
            }
        }
    }
    findings
}

fn member_features(
    doc: &IndexDoc,
    owners: &OwnerIndex,
    sources: &SourceResolver,
    member: &OverlapMember,
) -> Result<BTreeSet<String>, String> {
    let subject = member_subject(doc, sources, member)?;
    Ok(features_for_subject(doc, owners, &subject))
}

fn member_subject(
    doc: &IndexDoc,
    sources: &SourceResolver,
    member: &OverlapMember,
) -> Result<SubjectId, String> {
    let package = sources.package_for(&member.repo, &member.path)?;
    resolve_merged_crate_subject(doc, &member.repo, &package)
}

fn resolve_merged_crate_subject(
    doc: &IndexDoc,
    repo: &str,
    package: &str,
) -> Result<SubjectId, String> {
    let candidates = [
        format!("crate/{package}"),
        format!("local/{repo}/crate/{package}"),
    ];
    let matches = candidates
        .iter()
        .filter(|candidate| {
            doc.subjects
                .iter()
                .any(|subject| subject.id.as_str() == candidate.as_str())
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [subject] => Ok(SubjectId::new((*subject).clone())),
        [] => Err(format!(
            "no merged crate subject for repo {repo} package {package}"
        )),
        subjects => Err(format!(
            "multiple merged crate subjects for repo {repo} package {package}: {}",
            subjects
                .iter()
                .map(|subject| subject.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnerIndex {
    anchors: BTreeMap<String, String>,
    surfaces: BTreeMap<String, String>,
    specimens: BTreeMap<String, String>,
}

impl OwnerIndex {
    fn from_doc(doc: &IndexDoc) -> Self {
        Self {
            anchors: doc
                .anchors
                .iter()
                .map(|record| (record.id.to_string(), record.subject.to_string()))
                .collect(),
            surfaces: doc
                .surfaces
                .iter()
                .map(|record| (record.id.to_string(), record.subject.to_string()))
                .collect(),
            specimens: doc
                .specimens
                .iter()
                .map(|record| (record.id.to_string(), record.subject.to_string()))
                .collect(),
        }
    }
}

fn features_for_subject(
    doc: &IndexDoc,
    owners: &OwnerIndex,
    subject: &SubjectId,
) -> BTreeSet<String> {
    doc.features
        .iter()
        .filter(|feature| feature_mentions_subject(feature, owners, subject.as_str()))
        .map(|feature| feature.id.to_string())
        .collect()
}

fn feature_mentions_subject(feature: &FeatureRecord, owners: &OwnerIndex, subject: &str) -> bool {
    feature.subject.as_str() == subject
        || feature.anchors.iter().any(|id| {
            owners
                .anchors
                .get(id.as_str())
                .is_some_and(|owner| owner == subject)
        })
        || feature.surfaces.iter().any(|id| {
            owners
                .surfaces
                .get(id.as_str())
                .is_some_and(|owner| owner == subject)
        })
        || feature.specimens.iter().any(|id| {
            owners
                .specimens
                .get(id.as_str())
                .is_some_and(|owner| owner == subject)
        })
}

fn should_allow_unmapped_keep(member: &OverlapMember) -> bool {
    matches!(
        member.classification,
        SourceClassification::Keep | SourceClassification::Delegated
    ) && matches!(member.repo.as_str(), "sim-kernel" | "sim-private")
        && !member.reason.as_deref().unwrap_or("").trim().is_empty()
}

fn has_relating_edge(doc: &IndexDoc, left: &str, right: &str) -> bool {
    doc.edges.iter().any(|edge| {
        matches!(edge.rel.as_str(), "supports" | "presents" | "replaces")
            && ((edge.from == left && edge.to == right) || (edge.from == right && edge.to == left))
    })
}

fn strict_error(findings: &[&Finding]) -> String {
    findings
        .iter()
        .map(
            |finding| match (&finding.member, &finding.left, &finding.right) {
                (Some(member), _, _) => format!(
                    "unreconciled clone cluster {}: {}:{} {} {} ({})",
                    finding.cluster,
                    member.repo_path(),
                    member.line,
                    member.classification,
                    finding.reason,
                    finding.detail
                ),
                (_, Some(left), Some(right)) => format!(
                    "unreconciled clone cluster {}: {} <-> {} {}",
                    finding.cluster, left, right, finding.reason
                ),
                _ => format!(
                    "unreconciled clone cluster {}: {}",
                    finding.cluster, finding.reason
                ),
            },
        )
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
#[path = "index_overlap_tests.rs"]
mod tests;
