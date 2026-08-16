//! Configured shared-protocol coverage for the overlap board.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use serde::Deserialize;
use sim_index_core::{IndexDoc, ProtocolResolution};

use super::{Finding, OwnerIndex, features_for_subject};

const SCHEMA: &str = "sim.index-overlap-policy/v1";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct CoveragePolicy {
    pub(super) protocols: BTreeSet<String>,
    pub(super) exemptions: BTreeMap<String, CoverageExemption>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    schema: String,
    #[serde(default)]
    protocol: Vec<Protocol>,
    #[serde(default)]
    exemption: Vec<CoverageExemption>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Protocol {
    path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct CoverageExemption {
    pub(super) anchor: String,
    pub(super) reason: String,
}

impl CoveragePolicy {
    pub(super) fn read(path: Option<&PathBuf>, strict: bool) -> Result<Self, String> {
        let Some(path) = path else {
            if strict {
                return Err("index overlap --strict requires --policy <policy.toml>".to_owned());
            }
            return Ok(Self::default());
        };
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("read overlap policy {}: {err}", path.display()))?;
        let file: PolicyFile = toml::from_str(&text)
            .map_err(|err| format!("parse overlap policy {}: {err}", path.display()))?;
        if file.schema != SCHEMA {
            return Err(format!(
                "{} has schema {}, expected {SCHEMA}",
                path.display(),
                file.schema
            ));
        }
        let mut policy = Self::default();
        for protocol in file.protocol {
            if protocol.path.trim().is_empty() || !policy.protocols.insert(protocol.path.clone()) {
                return Err(format!(
                    "{} has empty or duplicate protocol path {:?}",
                    path.display(),
                    protocol.path
                ));
            }
        }
        if policy.protocols.is_empty() {
            return Err(format!(
                "{} configures zero shared protocols",
                path.display()
            ));
        }
        for exemption in file.exemption {
            if exemption.anchor.trim().is_empty() || exemption.reason.trim().is_empty() {
                return Err(format!(
                    "{} has an exemption without an anchor and reason",
                    path.display()
                ));
            }
            if policy
                .exemptions
                .insert(exemption.anchor.clone(), exemption)
                .is_some()
            {
                return Err(format!(
                    "{} has a duplicate coverage exemption",
                    path.display()
                ));
            }
        }
        Ok(policy)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct CoverageClassification {
    pub(super) applicable: usize,
    pub(super) covered: usize,
    pub(super) exempt: usize,
    pub(super) uncovered: usize,
}

pub(super) fn protocol_coverage_findings(
    doc: &IndexDoc,
    policy: &CoveragePolicy,
) -> (Vec<Finding>, CoverageClassification) {
    let claimed = doc
        .features
        .iter()
        .flat_map(|feature| feature.anchors.iter().map(|anchor| anchor.as_str()))
        .collect::<BTreeSet<_>>();
    let owners = OwnerIndex::from_doc(doc);
    let anchor_subjects = doc
        .anchors
        .iter()
        .map(|anchor| (anchor.id.as_str(), &anchor.subject))
        .collect::<BTreeMap<_, _>>();
    let mut applicable = BTreeSet::new();
    let mut findings = Vec::new();
    let mut classification = CoverageClassification::default();
    for relation in &doc.protocol_relations {
        let ProtocolResolution::Resolved { protocol } = &relation.resolution else {
            continue;
        };
        if !policy.protocols.contains(protocol) {
            continue;
        }
        let anchor = relation.anchor.as_str();
        applicable.insert(anchor);
        classification.applicable += 1;
        let reachable = claimed.contains(anchor)
            || anchor_subjects
                .get(anchor)
                .is_some_and(|subject| !features_for_subject(doc, &owners, subject).is_empty());
        if reachable {
            classification.covered += 1;
        } else if policy.exemptions.contains_key(anchor) {
            classification.exempt += 1;
        } else {
            classification.uncovered += 1;
            findings.push(Finding::coverage(
                protocol,
                "uncovered-protocol",
                format!(
                    "{} ({anchor}) implements configured protocol {protocol} but is not claimed by an authored feature and has no reasoned exemption",
                    relation.implementor
                ),
                "implementation",
            ));
        }
    }
    for exemption in policy.exemptions.values() {
        if !applicable.contains(exemption.anchor.as_str()) {
            findings.push(Finding::coverage(
                "exemptions",
                "stale-coverage-exemption",
                format!(
                    "{} no longer names an implementation of a configured protocol: {}",
                    exemption.anchor, exemption.reason
                ),
                "exempt",
            ));
        }
    }
    (findings, classification)
}
