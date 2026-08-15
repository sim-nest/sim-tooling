//! Conservative record-shape candidates derived from normalized Index declarations.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use sim_index_core::{DeclarationRole, IndexDoc};

use crate::{
    index_overlap_report::{CloneCluster, OverlapMember, SourceClassification},
    index_source::SourceResolver,
};

const MIN_PUBLIC_FIELDS: usize = 4;
const MIN_COMPATIBLE_RECORDS: usize = 2;

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
        AnchorId, DeclarationFact, DiscoveredAnchor, SourceLocation, SubjectId, SubjectRecord,
        SyntaxBound,
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
