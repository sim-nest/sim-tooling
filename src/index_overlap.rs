//! Advisory duplicate-implementation bridge over the merged SIM Index graph.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use sim_index_core::IndexDoc;

use crate::index_render::load_doc;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = OverlapOptions::parse(&args)?;
    let doc = load_doc(&options.input)?;
    let clusters = options
        .clusters
        .as_ref()
        .map(|path| read_clusters(path))
        .transpose()?
        .unwrap_or_default();
    let findings = overlap_findings(&doc, &clusters);
    if options.strict && !findings.is_empty() {
        return Err(strict_error(&findings));
    }
    if options.json {
        let text = serde_json::to_string_pretty(&json!({
            "advisory": !options.strict,
            "strict": options.strict,
            "cluster_count": clusters.len(),
            "finding_count": findings.len(),
            "findings": findings.iter().map(Finding::to_json).collect::<Vec<_>>(),
        }))
        .map_err(|err| format!("serialize overlap findings: {err}"))?;
        println!("{text}");
    } else if findings.is_empty() {
        println!(
            "index overlap: advisory ok ({} cluster(s), 0 missing feature relations)",
            clusters.len()
        );
    } else {
        for finding in &findings {
            println!(
                "index overlap: advisory {} {} <-> {} missing-relating-edge",
                finding.cluster, finding.left, finding.right
            );
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OverlapOptions {
    input: PathBuf,
    clusters: Option<PathBuf>,
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
            json,
            strict,
        })
    }
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} index overlap --input <index.sx> [--clusters <clusters.json>] [--json] [--strict]"
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CloneCluster {
    id: String,
    members: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Finding {
    cluster: String,
    left: String,
    right: String,
}

impl Finding {
    fn to_json(&self) -> Value {
        json!({
            "cluster": self.cluster,
            "features": [self.left, self.right],
            "reason": "missing-relating-edge",
        })
    }
}

fn read_clusters(path: &Path) -> Result<Vec<CloneCluster>, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|err| format!("parse {}: {err}", path.display()))?;
    let Some(clusters) = value.get("clusters").and_then(Value::as_array) else {
        return Err(format!("{} missing clusters array", path.display()));
    };
    clusters
        .iter()
        .enumerate()
        .map(|(index, cluster)| {
            let id = cluster
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("cluster/{index}"));
            let members = cluster
                .get("members")
                .or_else(|| cluster.get("items"))
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{id} missing members array"))?
                .iter()
                .map(member_id)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CloneCluster { id, members })
        })
        .collect()
}

fn member_id(value: &Value) -> Result<String, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_owned());
    }
    value
        .get("id")
        .or_else(|| value.get("path"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("cluster member must be a string or object with id/path: {value}"))
}

fn overlap_findings(doc: &IndexDoc, clusters: &[CloneCluster]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for cluster in clusters {
        let features = cluster_features(doc, cluster);
        for (left_index, left) in features.iter().enumerate() {
            for right in features.iter().skip(left_index + 1) {
                if !has_relating_edge(doc, left, right) {
                    findings.push(Finding {
                        cluster: cluster.id.clone(),
                        left: left.clone(),
                        right: right.clone(),
                    });
                }
            }
        }
    }
    findings
}

fn strict_error(findings: &[Finding]) -> String {
    findings
        .iter()
        .map(|finding| {
            format!(
                "unreconciled clone cluster {}: {} <-> {} missing-relating-edge",
                finding.cluster, finding.left, finding.right
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn cluster_features(doc: &IndexDoc, cluster: &CloneCluster) -> Vec<String> {
    let mut features = BTreeSet::new();
    for member in &cluster.members {
        for feature in &doc.features {
            if feature.id.as_str() == member || feature.subject.as_str() == member {
                features.insert(feature.id.to_string());
            }
            if feature.anchors.iter().any(|id| id.as_str() == member)
                || feature.surfaces.iter().any(|id| id.as_str() == member)
                || feature.specimens.iter().any(|id| id.as_str() == member)
            {
                features.insert(feature.id.to_string());
            }
        }
    }
    features.into_iter().collect()
}

fn has_relating_edge(doc: &IndexDoc, left: &str, right: &str) -> bool {
    doc.edges.iter().any(|edge| {
        matches!(edge.rel.as_str(), "supports" | "presents" | "replaces")
            && ((edge.from == left && edge.to == right) || (edge.from == right && edge.to == left))
    })
}

#[cfg(test)]
mod tests {
    use sim_index_core::{
        CanonicalFeatureKey, FeatureId, FeatureRecord, IndexDoc, IndexEdge, SubjectId,
        SubjectRecord,
    };

    use super::*;

    #[test]
    fn missing_feature_relation_is_reported() {
        let doc = doc_with_features(false);
        let clusters = vec![CloneCluster {
            id: "cluster/helpers".to_owned(),
            members: vec!["feature/one".to_owned(), "feature/two".to_owned()],
        }];

        let findings = overlap_findings(&doc, &clusters);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].cluster, "cluster/helpers");
    }

    #[test]
    fn existing_relation_satisfies_cluster() {
        let doc = doc_with_features(true);
        let clusters = vec![CloneCluster {
            id: "cluster/helpers".to_owned(),
            members: vec!["feature/one".to_owned(), "feature/two".to_owned()],
        }];

        assert!(overlap_findings(&doc, &clusters).is_empty());
    }

    #[test]
    fn strict_error_names_unreconciled_cluster() {
        let findings = vec![Finding {
            cluster: "cluster/helpers".to_owned(),
            left: "feature/one".to_owned(),
            right: "feature/two".to_owned(),
        }];

        let err = strict_error(&findings);

        assert!(err.contains("unreconciled clone cluster cluster/helpers"));
        assert!(err.contains("feature/one <-> feature/two"));
    }

    fn doc_with_features(with_edge: bool) -> IndexDoc {
        let mut doc = IndexDoc::public("test");
        for id in ["crate/one", "crate/two"] {
            doc.subjects.push(SubjectRecord {
                id: SubjectId::new(id),
                kind: "crate".to_owned(),
                title: id.to_owned(),
            });
        }
        for (feature, subject) in [("feature/one", "crate/one"), ("feature/two", "crate/two")] {
            doc.features.push(FeatureRecord {
                id: FeatureId::new(feature),
                key: CanonicalFeatureKey::new(format!("{subject}/feature")),
                subject: SubjectId::new(subject),
                title: feature.to_owned(),
                summary: "A feature.".to_owned(),
                anchors: Vec::new(),
                surfaces: Vec::new(),
                specimens: Vec::new(),
                grammar_contracts: Vec::new(),
                doc_anchor: None,
            });
        }
        if with_edge {
            doc.edges
                .push(IndexEdge::new("feature/one", "supports", "feature/two"));
        }
        doc
    }
}
