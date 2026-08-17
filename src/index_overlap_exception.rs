//! Structural-candidate decisions from the one source-overlap classification ledger.

use std::{collections::BTreeMap, fs, path::PathBuf};

use crate::index_overlap_report::{CloneCluster, SourceClassification};

const CLUSTER_SENTINEL: &str = "@cluster";
const MEMBER_OWNED: &str = "member-owned";

#[derive(Clone, Debug, Eq, PartialEq)]
struct StructuralException {
    classification: SourceClassification,
    owner: String,
    reason: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StructuralExceptions {
    entries: BTreeMap<String, StructuralException>,
}

impl StructuralExceptions {
    pub(crate) fn read(path: Option<&PathBuf>, strict: bool) -> Result<Self, String> {
        let Some(path) = path else {
            if strict {
                return Err(
                    "index overlap --strict requires --exceptions <classifications.tsv>".to_owned(),
                );
            }
            return Ok(Self::default());
        };
        let text = fs::read_to_string(path)
            .map_err(|err| format!("read overlap exceptions {}: {err}", path.display()))?;
        let mut entries = BTreeMap::new();
        for (index, raw) in text.lines().enumerate() {
            if raw.trim().is_empty() || raw.starts_with('#') {
                continue;
            }
            let fields = raw.split('\t').collect::<Vec<_>>();
            if fields.len() != 7 {
                return Err(format!(
                    "{}:{}: expected 7 tab-separated overlap exception fields",
                    path.display(),
                    index + 1
                ));
            }
            let [
                cluster,
                repo,
                source_path,
                symbol,
                classification,
                owner,
                reason,
            ] = fields.as_slice()
            else {
                unreachable!("field count checked above")
            };
            if !is_structural_cluster(cluster) {
                continue;
            }
            if [repo, source_path, symbol]
                .iter()
                .any(|field| **field != CLUSTER_SENTINEL)
            {
                return Err(format!(
                    "{}:{}: structural exception {cluster} must use {CLUSTER_SENTINEL} in repo, path, and symbol fields",
                    path.display(),
                    index + 1
                ));
            }
            let classification = SourceClassification::parse(classification)?;
            if !matches!(
                classification,
                SourceClassification::Keep | SourceClassification::Delegated
            ) {
                return Err(format!(
                    "{}:{}: structural exception classification must be keep or delegated",
                    path.display(),
                    index + 1
                ));
            }
            if owner.trim().is_empty() || reason.trim().is_empty() {
                return Err(format!(
                    "{}:{}: structural exception requires an owner and reason",
                    path.display(),
                    index + 1
                ));
            }
            if entries
                .insert(
                    (*cluster).to_owned(),
                    StructuralException {
                        classification,
                        owner: (*owner).to_owned(),
                        reason: (*reason).to_owned(),
                    },
                )
                .is_some()
            {
                return Err(format!(
                    "{}:{}: duplicate structural exception for {cluster}",
                    path.display(),
                    index + 1
                ));
            }
        }
        Ok(Self { entries })
    }

    pub(crate) fn classify(&self, clusters: &mut [CloneCluster]) -> Result<(), String> {
        let current = clusters
            .iter()
            .filter(|cluster| is_structural_cluster(&cluster.id))
            .map(|cluster| cluster.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if let Some(stale) = self
            .entries
            .keys()
            .find(|cluster| !current.contains(cluster.as_str()))
        {
            return Err(format!("stale structural overlap exception: {stale}"));
        }
        for cluster in clusters {
            let Some(exception) = self.entries.get(&cluster.id) else {
                continue;
            };
            cluster.owner = exception.owner.clone();
            for member in &mut cluster.members {
                member.classification = exception.classification;
                member.reason = Some(exception.reason.clone());
                if exception.owner != MEMBER_OWNED {
                    member.owner = exception.owner.clone();
                }
            }
        }
        Ok(())
    }
}

fn is_structural_cluster(cluster: &str) -> bool {
    cluster.starts_with("record-shape/") || cluster.starts_with("implementation-shape/")
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::index_overlap_report::OverlapMember;

    use super::*;

    #[test]
    fn classifies_exact_clusters_and_rejects_stale_entries() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("sim-overlap-exceptions-{nonce}.tsv"));
        fs::write(
            &path,
            "record-shape/exact\t@cluster\t@cluster\t@cluster\tkeep\tmember-owned\tdistinct schema roles\n",
        )
        .unwrap();
        let decisions = StructuralExceptions::read(Some(&path), true).unwrap();
        let mut clusters = vec![cluster("record-shape/exact")];
        decisions.classify(&mut clusters).unwrap();
        assert_eq!(
            clusters[0].members[0].classification,
            SourceClassification::Keep
        );
        assert_eq!(clusters[0].members[0].owner, "crate/example");
        assert_eq!(
            clusters[0].members[0].reason.as_deref(),
            Some("distinct schema roles")
        );
        assert!(
            decisions
                .classify(&mut [cluster("record-shape/other")])
                .unwrap_err()
                .contains("stale structural overlap exception")
        );
        fs::remove_file(path).unwrap();
    }

    fn cluster(id: &str) -> CloneCluster {
        CloneCluster {
            id: id.to_owned(),
            owner: String::new(),
            replacement: String::new(),
            members: vec![OverlapMember {
                repo: "sim-example".to_owned(),
                path: "src/lib.rs".to_owned(),
                line: 1,
                symbol: "Example".to_owned(),
                anchor: None,
                fingerprint_reason: None,
                classification: SourceClassification::Candidate,
                reason: None,
                owner: "crate/example".to_owned(),
                replacement: String::new(),
            }],
        }
    }
}
