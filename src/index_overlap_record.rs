//! Conservative record-shape candidates derived from normalized Index declarations.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use sim_index_core::{DeclarationRole, IndexDoc, ProtocolResolution};

use crate::{
    index_overlap_report::{CloneCluster, OverlapMember, SourceClassification},
    index_source::SourceResolver,
};

const MIN_PUBLIC_FIELDS: usize = 4;
const MIN_COMPATIBLE_RECORDS: usize = 2;
const MIN_IMPL_BODY_BYTES: usize = 24;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ImplementationShapeClassification {
    pub(crate) candidate_clusters: usize,
    pub(crate) candidate_members: usize,
    pub(crate) excluded_relation_count: usize,
    pub(crate) excluded_relation_causes: BTreeMap<String, usize>,
    pub(crate) false_positive_count: usize,
    pub(crate) false_positive_causes: BTreeMap<String, usize>,
}

pub(crate) fn implementation_shape_clusters(
    doc: &IndexDoc,
    sources: &SourceResolver,
) -> Result<(Vec<CloneCluster>, ImplementationShapeClassification), String> {
    let subjects = doc
        .anchors
        .iter()
        .map(|anchor| (anchor.id.as_str(), anchor.subject.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut eligible = BTreeMap::<String, Vec<_>>::new();
    let mut classification = ImplementationShapeClassification::default();
    for relation in &doc.protocol_relations {
        let protocol = match &relation.resolution {
            ProtocolResolution::Resolved { protocol } => protocol,
            ProtocolResolution::Unresolved { .. } => {
                classification.excluded_relation_count += 1;
                *classification
                    .excluded_relation_causes
                    .entry("unresolved-protocol".to_owned())
                    .or_default() += 1;
                continue;
            }
        };
        if relation.body_bound.truncated {
            classification.excluded_relation_count += 1;
            *classification
                .excluded_relation_causes
                .entry("truncated-body".to_owned())
                .or_default() += 1;
            continue;
        }
        if relation.body_fingerprint.len() < MIN_IMPL_BODY_BYTES {
            classification.excluded_relation_count += 1;
            *classification
                .excluded_relation_causes
                .entry("insufficient-body-evidence".to_owned())
                .or_default() += 1;
            continue;
        }
        eligible.entry(protocol.clone()).or_default().push(relation);
    }

    let mut groups = BTreeMap::<String, Vec<_>>::new();
    for (protocol, relations) in &eligible {
        for left in 0..relations.len() {
            for right in left + 1..relations.len() {
                if relations[left].body_fingerprint != relations[right].body_fingerprint {
                    classification.false_positive_count += 1;
                    *classification
                        .false_positive_causes
                        .entry("different-body-behavior".to_owned())
                        .or_default() += 1;
                }
            }
        }
        for relation in relations {
            groups
                .entry(format!("{protocol}|{}", relation.body_fingerprint))
                .or_default()
                .push(*relation);
        }
    }

    let mut clusters = Vec::new();
    for (fingerprint, relations) in groups {
        if relations.len() < MIN_COMPATIBLE_RECORDS {
            continue;
        }
        let mut members = Vec::new();
        for relation in relations {
            let subject = subjects
                .get(relation.anchor.as_str())
                .ok_or_else(|| format!("protocol relation {} has no anchor", relation.anchor))?;
            let package = subject
                .rsplit('/')
                .next()
                .ok_or_else(|| format!("cannot derive package from {subject}"))?;
            let (repo, path, line) = sources.implementation_source(
                package,
                &relation.source_spelling,
                &relation.implementor,
            )?;
            members.push(OverlapMember {
                repo,
                path,
                line,
                symbol: relation.implementor.clone(),
                anchor: Some(relation.anchor.to_string()),
                fingerprint_reason: Some(
                    "same resolved protocol and alpha-normalized implementation body".to_owned(),
                ),
                classification: SourceClassification::Candidate,
                reason: None,
                owner: subject.to_string(),
                replacement: String::new(),
            });
        }
        members.sort_by(|left, right| {
            (&left.repo, &left.path, left.line, &left.symbol).cmp(&(
                &right.repo,
                &right.path,
                right.line,
                &right.symbol,
            ))
        });
        let digest = format!("{:x}", Sha256::digest(fingerprint.as_bytes()));
        clusters.push(CloneCluster {
            id: format!("implementation-shape/{}", &digest[..16]),
            owner: String::new(),
            replacement: String::new(),
            members,
        });
    }
    clusters.sort_by(|left, right| left.id.cmp(&right.id));
    classification.candidate_clusters = clusters.len();
    classification.candidate_members = clusters.iter().map(|cluster| cluster.members.len()).sum();
    Ok((clusters, classification))
}

pub(crate) fn record_shape_clusters(
    doc: &IndexDoc,
    sources: &SourceResolver,
) -> Result<Vec<CloneCluster>, String> {
    let anchors = doc
        .anchors
        .iter()
        .map(|anchor| (anchor.id.as_str(), anchor.subject.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut groups = BTreeMap::<String, Vec<OverlapMember>>::new();
    for fact in &doc.declarations {
        if fact.role != DeclarationRole::Struct
            || fact.syntax_bound.truncated
            || fact.members.len() < MIN_PUBLIC_FIELDS
        {
            continue;
        }
        let subject = anchors
            .get(fact.anchor.as_str())
            .ok_or_else(|| format!("declaration {} has no anchor", fact.module_path))?;
        let package = subject
            .rsplit('/')
            .next()
            .ok_or_else(|| format!("cannot derive package from {subject}"))?;
        let (repo, line) =
            sources.declaration_source(package, &fact.location.file, &fact.module_path)?;
        let fingerprint = format!("struct|{}|{}", fact.generics, fact.members.join("|"));
        groups
            .entry(fingerprint.clone())
            .or_default()
            .push(OverlapMember {
                repo,
                path: fact.location.file.clone(),
                line,
                symbol: fact.module_path.clone(),
                anchor: Some(fact.anchor.to_string()),
                fingerprint_reason: Some(format!(
                    "same normalized public record signature: {fingerprint}"
                )),
                classification: SourceClassification::Candidate,
                reason: None,
                owner: subject.to_string(),
                replacement: String::new(),
            });
    }
    let mut clusters = groups
        .into_iter()
        .filter(|(_, members)| members.len() >= MIN_COMPATIBLE_RECORDS)
        .map(|(fingerprint, mut members)| {
            members.sort_by(|left, right| {
                (&left.repo, &left.path, left.line, &left.symbol).cmp(&(
                    &right.repo,
                    &right.path,
                    right.line,
                    &right.symbol,
                ))
            });
            let digest = format!("{:x}", Sha256::digest(fingerprint.as_bytes()));
            CloneCluster {
                id: format!("record-shape/{}", &digest[..16]),
                owner: String::new(),
                replacement: String::new(),
                members,
            }
        })
        .collect::<Vec<_>>();
    clusters.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(clusters)
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use sim_index_core::{
        AnchorId, DeclarationFact, DiscoveredAnchor, ProtocolRelation, SourceLocation, SubjectId,
        SubjectRecord, SyntaxBound,
    };

    use super::*;

    #[test]
    fn admits_matching_substantial_records_and_suppresses_small_noise() {
        let root = temp_root();
        for (repo, package, symbol) in [
            ("sim-one", "pkg-one", "FirstAdmission"),
            ("sim-two", "pkg-two", "SecondAdmission"),
        ] {
            fs::create_dir_all(root.join(repo).join("src")).unwrap();
            fs::create_dir_all(root.join(repo).join("docs/generated")).unwrap();
            fs::write(
                root.join(repo).join("src/lib.rs"),
                format!("pub struct {symbol} {{\n    pub a: A, pub b: B, pub c: C, pub d: D, pub e: E,\n}}\n"),
            ).unwrap();
            fs::write(
                root.join(repo).join("docs/generated/repo-contract.json"),
                format!(r#"{{"packages":[{{"name":"{package}","root":""}}]}}"#),
            )
            .unwrap();
        }
        fs::write(
            root.join("repos.toml"),
            "[[repo]]\nname=\"sim-one\"\ncontains_code=true\nlocal_path=\"sim-one\"\n\n[[repo]]\nname=\"sim-two\"\ncontains_code=true\nlocal_path=\"sim-two\"\n",
        ).unwrap();
        let sources = SourceResolver::from_manifest(&root, &root.join("repos.toml")).unwrap();
        let mut doc = IndexDoc::public("test");
        for (package, symbol) in [
            ("pkg-one", "FirstAdmission"),
            ("pkg-two", "SecondAdmission"),
        ] {
            let subject = SubjectId::new(format!("crate/{package}"));
            let anchor = AnchorId::new(format!("anchor/rustdoc/{package}/{symbol}"));
            doc.subjects.push(SubjectRecord {
                id: subject.clone(),
                kind: "crate".into(),
                title: package.into(),
            });
            doc.anchors.push(DiscoveredAnchor {
                id: anchor.clone(),
                subject,
                kind: "rustdoc-item".into(),
            });
            doc.declarations.push(declaration(
                anchor.clone(),
                symbol,
                vec!["a:A", "b:B", "c:C", "d:D", "e:E"],
            ));
            doc.declarations.push(declaration(
                anchor,
                &format!("{symbol}Noise"),
                vec!["left:A", "right:B"],
            ));
        }

        let clusters = record_shape_clusters(&doc, &sources).unwrap();

        assert_eq!(
            clusters.len(),
            1,
            "common two-field records stay below the noise budget"
        );
        assert_eq!(
            clusters[0].members.len(),
            2,
            "both substantial admissions are raised"
        );
        assert!(
            clusters[0]
                .members
                .iter()
                .all(|member| member.anchor.is_some())
        );
        assert!(clusters[0].members.iter().all(|member| {
            member
                .fingerprint_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("normalized public record signature"))
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clusters_managed_impls_and_classifies_different_clearing_as_false_positive() {
        let root = temp_root();
        let rows = [
            ("sim-js", "js-runtime", "JavaScriptManaged", "same-body"),
            ("sim-py", "py-runtime", "PythonManaged", "same-body"),
            (
                "sim-other",
                "other-runtime",
                "OtherManaged",
                "different-clear",
            ),
        ];
        let mut manifest = String::new();
        let mut doc = IndexDoc::public("test");
        for (repo, package, implementor, body) in rows {
            fs::create_dir_all(root.join(repo).join("src")).unwrap();
            fs::create_dir_all(root.join(repo).join("docs/generated")).unwrap();
            fs::write(
                root.join(repo).join("src/lib.rs"),
                format!("struct {implementor}; impl sim_kernel::Managed for {implementor} {{ fn run(&self) {{}} }}\n"),
            )
            .unwrap();
            fs::write(
                root.join(repo).join("docs/generated/repo-contract.json"),
                format!(r#"{{"packages":[{{"name":"{package}","root":""}}]}}"#),
            )
            .unwrap();
            manifest.push_str(&format!(
                "[[repo]]\nname=\"{repo}\"\ncontains_code=true\nlocal_path=\"{repo}\"\n\n"
            ));
            let subject = SubjectId::new(format!("crate/{package}"));
            let anchor = AnchorId::new(format!("anchor/rust-impl/{package}/{implementor}"));
            doc.subjects.push(SubjectRecord {
                id: subject.clone(),
                kind: "crate".into(),
                title: package.into(),
            });
            doc.anchors.push(DiscoveredAnchor {
                id: anchor.clone(),
                subject,
                kind: "rust-impl".into(),
            });
            doc.protocol_relations.push(ProtocolRelation {
                anchor,
                implementor: implementor.into(),
                source_spelling: "Managed".into(),
                body_fingerprint: format!("fn run(&self){{self.{body}();}}"),
                body_bound: SyntaxBound {
                    max_bytes: 16_384,
                    truncated: false,
                },
                resolution: ProtocolResolution::Resolved {
                    protocol: "sim_kernel::Managed".into(),
                },
            });
        }
        fs::write(root.join("repos.toml"), manifest).unwrap();
        let sources = SourceResolver::from_manifest(&root, &root.join("repos.toml")).unwrap();

        let (clusters, classification) = implementation_shape_clusters(&doc, &sources).unwrap();

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].members.len(), 2);
        assert_eq!(
            clusters[0]
                .members
                .iter()
                .map(|member| member.repo.as_str())
                .collect::<Vec<_>>(),
            vec!["sim-js", "sim-py"]
        );
        assert_eq!(classification.candidate_clusters, 1);
        assert_eq!(classification.candidate_members, 2);
        assert_eq!(classification.false_positive_count, 2);
        assert_eq!(
            classification.false_positive_causes["different-body-behavior"],
            2
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn declaration(anchor: AnchorId, symbol: &str, members: Vec<&str>) -> DeclarationFact {
        DeclarationFact {
            anchor,
            role: DeclarationRole::Struct,
            module_path: symbol.into(),
            generics: String::new(),
            members: members.into_iter().map(str::to_owned).collect(),
            location: SourceLocation {
                file: "src/lib.rs".into(),
                declaration: 0,
            },
            syntax_bound: SyntaxBound {
                max_bytes: 16_384,
                truncated: false,
            },
        }
    }

    fn temp_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "sim-tooling-record-shape-{}-{nonce}",
            std::process::id()
        ))
    }
}
