//! Declaration and protocol-fact selection for `index find`.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use sim_index_core::{AnchorId, IndexDoc, ProtocolRelation, ProtocolResolution, UnresolvedReason};

use super::{FindOptions, ResolutionFilter, matches_query};

pub(super) fn selected_fact_anchors(
    doc: &IndexDoc,
    options: &FindOptions,
    needle: &str,
) -> BTreeSet<AnchorId> {
    let declaration_filter = options.declaration_kind.is_some();
    let relation_filter = options.implements.is_some() || options.resolution.is_some();
    doc.anchors
        .iter()
        .filter(|anchor| {
            let declarations = doc
                .declarations
                .iter()
                .filter(|fact| fact.anchor == anchor.id);
            let mut relations = doc
                .protocol_relations
                .iter()
                .filter(|relation| relation.anchor == anchor.id);
            let declaration_match = !declaration_filter
                || declarations
                    .clone()
                    .any(|fact| options.declaration_kind == Some(fact.role));
            let relation_match = !relation_filter
                || relations
                    .clone()
                    .any(|relation| relation_matches(relation, options));
            let feature_match = options.feature.as_ref().is_none_or(|expected| {
                doc.features.iter().any(|feature| {
                    feature.anchors.iter().any(|id| id == &anchor.id)
                        && (feature.id.as_str() == expected || feature.key.as_str() == expected)
                })
            });
            let text_match = needle.is_empty()
                || matches_query(
                    needle,
                    &[anchor.id.as_str(), anchor.subject.as_str(), &anchor.kind],
                )
                || declarations.clone().any(|fact| {
                    matches_query(
                        needle,
                        &[fact.role.as_str(), &fact.module_path, &fact.location.file],
                    )
                })
                || relations.any(|relation| relation_text_matches(relation, needle));
            declaration_match && relation_match && feature_match && text_match
        })
        .map(|anchor| anchor.id.clone())
        .collect()
}

fn relation_matches(relation: &ProtocolRelation, options: &FindOptions) -> bool {
    let protocol_matches = options.implements.as_ref().is_none_or(|expected| {
        relation.source_spelling == *expected
            || match &relation.resolution {
                ProtocolResolution::Resolved { protocol } => protocol == expected,
                ProtocolResolution::Unresolved { candidates, .. } => {
                    candidates.iter().any(|candidate| candidate == expected)
                }
            }
    });
    let resolution_matches = options.resolution.is_none_or(|expected| {
        matches!(
            (expected, &relation.resolution),
            (
                ResolutionFilter::Resolved,
                ProtocolResolution::Resolved { .. }
            ) | (
                ResolutionFilter::Unresolved,
                ProtocolResolution::Unresolved { .. }
            )
        )
    });
    protocol_matches && resolution_matches
}

fn relation_text_matches(relation: &ProtocolRelation, needle: &str) -> bool {
    let mut fields = vec![
        relation.implementor.as_str(),
        relation.source_spelling.as_str(),
    ];
    match &relation.resolution {
        ProtocolResolution::Resolved { protocol } => fields.push(protocol),
        ProtocolResolution::Unresolved { candidates, .. } => {
            fields.extend(candidates.iter().map(String::as_str));
        }
    }
    matches_query(needle, &fields)
}

pub(super) fn protocol_relation_json(relation: &ProtocolRelation) -> Value {
    match &relation.resolution {
        ProtocolResolution::Resolved { protocol } => json!({
            "implementor": relation.implementor,
            "source_spelling": relation.source_spelling,
            "resolution": "resolved",
            "protocol": protocol,
        }),
        ProtocolResolution::Unresolved { reason, candidates } => json!({
            "implementor": relation.implementor,
            "source_spelling": relation.source_spelling,
            "resolution": "unresolved",
            "unresolved_reason": unresolved_reason(*reason),
            "candidates": candidates,
        }),
    }
}

pub(super) fn protocol_relation_summary(relation: &ProtocolRelation) -> String {
    match &relation.resolution {
        ProtocolResolution::Resolved { protocol } => format!(
            "resolved protocol {} implements {protocol}",
            relation.implementor
        ),
        ProtocolResolution::Unresolved { reason, .. } => format!(
            "unresolved protocol edge {} to {} ({})",
            relation.implementor,
            relation.source_spelling,
            unresolved_reason(*reason)
        ),
    }
}

fn unresolved_reason(reason: UnresolvedReason) -> &'static str {
    match reason {
        UnresolvedReason::AmbiguousGlobImport => "ambiguous-glob-import",
        UnresolvedReason::AmbiguousName => "ambiguous-name",
        UnresolvedReason::ExternalMetadataAbsent => "external-metadata-absent",
    }
}
