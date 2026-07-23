use std::collections::{BTreeMap, BTreeSet};

use sim_index_core::{
    AnchorId, DiscoveredSpecimen, DiscoveredSurface, FeatureDraft, FeatureRecord, GrammarContract,
    IndexDoc, IndexEdge, RouteRecord, RouteStep, SubjectRecord, Visibility, check_index_doc,
    shape::is_index_id,
};

pub(crate) use crate::index_vault_graph_model::{
    VaultAnchor, VaultCoverage, VaultEndpoint, VaultFeature, VaultFeatureDraft,
    VaultGrammarContract, VaultGranularity, VaultGraph, VaultNode, VaultNodeKind, VaultRelation,
    VaultRoute, VaultRouteStep, VaultSpecimen, VaultSubject, VaultSurface,
};

impl VaultGraph {
    pub(crate) fn from_index(doc: &IndexDoc) -> Result<Self, String> {
        if doc.visibility != Visibility::Public {
            return Err("vault graph requires a public IndexDoc".to_owned());
        }
        check_index_doc(doc).map_err(|err| format!("invalid index document: {err}"))?;

        let nodes = sorted_nodes(doc);
        let endpoints = endpoint_index(&nodes)?;
        let relations = sorted_relations(derive_relations(doc, &endpoints)?);
        let reverse_relations = sorted_relations(relations.iter().map(VaultRelation::reversed));
        let coverage = VaultCoverage::from_nodes(&nodes);
        let graph = Self {
            nodes,
            relations,
            reverse_relations,
            coverage,
        };
        graph.check(VaultGranularity::Full)?;
        Ok(graph)
    }

    pub(crate) fn check(&self, granularity: VaultGranularity) -> Result<(), String> {
        reject_duplicate_nodes(&self.nodes)?;
        reject_duplicate_relations("forward", &self.relations)?;
        reject_duplicate_relations("reverse", &self.reverse_relations)?;

        let unresolved = self
            .unresolved_relations()
            .map(VaultRelation::summary)
            .collect::<Vec<_>>();
        if !unresolved.is_empty() {
            return Err(format!(
                "vault graph has unresolved relation endpoint(s): {}",
                unresolved.join(", ")
            ));
        }

        let expected_reverse = sorted_relations(self.relations.iter().map(VaultRelation::reversed));
        if self.reverse_relations != expected_reverse {
            return Err("vault graph reverse relations do not mirror forward relations".to_owned());
        }

        let missing = self.coverage.unrepresented_for(granularity);
        if !missing.is_empty() {
            return Err(format!(
                "vault graph has unrepresented {:?} row(s): {}",
                granularity,
                missing
                    .iter()
                    .map(VaultEndpoint::summary)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        Ok(())
    }

    pub(crate) fn unresolved_relations(&self) -> impl Iterator<Item = &VaultRelation> {
        let endpoints = self
            .nodes
            .iter()
            .map(VaultNode::endpoint)
            .collect::<BTreeSet<_>>();
        self.relations.iter().filter(move |relation| {
            !endpoints.contains(&relation.from) || !endpoints.contains(&relation.to)
        })
    }
}

fn sorted_nodes(doc: &IndexDoc) -> Vec<VaultNode> {
    let mut nodes = Vec::new();
    nodes.extend(doc.subjects.iter().map(subject_node));
    nodes.extend(doc.anchors.iter().map(anchor_node));
    nodes.extend(doc.surfaces.iter().map(surface_node));
    nodes.extend(doc.specimens.iter().map(specimen_node));
    nodes.extend(doc.drafts.iter().map(draft_node));
    nodes.extend(doc.features.iter().map(feature_node));
    nodes.extend(doc.routes.iter().map(route_node));
    nodes.sort_by_key(|node| node.endpoint());
    nodes
}

fn subject_node(record: &SubjectRecord) -> VaultNode {
    VaultNode::Subject(VaultSubject {
        id: record.id.to_string(),
        kind: record.kind.clone(),
        title: record.title.clone(),
    })
}

fn anchor_node(record: &sim_index_core::DiscoveredAnchor) -> VaultNode {
    VaultNode::Anchor(VaultAnchor {
        id: record.id.to_string(),
        subject: record.subject.to_string(),
        kind: record.kind.clone(),
    })
}

fn surface_node(record: &DiscoveredSurface) -> VaultNode {
    VaultNode::Surface(VaultSurface {
        id: record.id.to_string(),
        subject: record.subject.to_string(),
        kind: record.kind.clone(),
    })
}

fn specimen_node(record: &DiscoveredSpecimen) -> VaultNode {
    VaultNode::Specimen(VaultSpecimen {
        id: record.id.to_string(),
        subject: record.subject.to_string(),
        kind: record.kind.clone(),
        path: record.path.clone(),
        language: record.language.clone(),
        runnable: record.runnable,
        checked: record.checked,
        checked_by: record.checked_by.clone(),
        doc_anchor: optional_anchor(&record.doc_anchor),
    })
}

fn draft_node(record: &FeatureDraft) -> VaultNode {
    VaultNode::Draft(VaultFeatureDraft {
        id: record.id.to_string(),
        subject: record.subject.to_string(),
        title: record.title.clone(),
        summary: record.summary.clone(),
        claims_anchors: sorted_ids(record.claims_anchors.iter()),
        claims_surfaces: sorted_ids(record.claims_surfaces.iter()),
        claims_specimens: sorted_ids(record.claims_specimens.iter()),
        grammar_contracts: grammar_contracts(&record.grammar_contracts),
        doc_anchor: optional_anchor(&record.doc_anchor),
    })
}

fn feature_node(record: &FeatureRecord) -> VaultNode {
    VaultNode::Feature(VaultFeature {
        id: record.id.to_string(),
        key: record.key.to_string(),
        subject: record.subject.to_string(),
        title: record.title.clone(),
        summary: record.summary.clone(),
        anchors: sorted_ids(record.anchors.iter()),
        surfaces: sorted_ids(record.surfaces.iter()),
        specimens: sorted_ids(record.specimens.iter()),
        grammar_contracts: grammar_contracts(&record.grammar_contracts),
        doc_anchor: optional_anchor(&record.doc_anchor),
    })
}

fn route_node(record: &RouteRecord) -> VaultNode {
    let mut audiences = record.audiences.clone();
    audiences.sort();
    audiences.dedup();
    VaultNode::Route(VaultRoute {
        id: record.id.to_string(),
        title: record.title.clone(),
        audiences,
        steps: record
            .steps
            .iter()
            .enumerate()
            .map(|(order, step)| VaultRouteStep {
                order,
                target: route_step_endpoint(step),
                why: step.why().to_owned(),
            })
            .collect(),
        doc_anchor: optional_anchor(&record.doc_anchor),
    })
}

fn route_step_endpoint(step: &RouteStep) -> VaultEndpoint {
    match step {
        RouteStep::Feature { id, .. } => VaultEndpoint::new(VaultNodeKind::Feature, id.as_str()),
        RouteStep::Specimen { id, .. } => VaultEndpoint::new(VaultNodeKind::Specimen, id.as_str()),
    }
}

fn optional_anchor(anchor: &Option<AnchorId>) -> Option<String> {
    anchor.as_ref().map(ToString::to_string)
}

fn sorted_ids<'a, T>(ids: impl Iterator<Item = &'a T>) -> Vec<String>
where
    T: ToString + 'a,
{
    let mut out = ids.map(ToString::to_string).collect::<Vec<_>>();
    out.sort();
    out
}

fn grammar_contracts(records: &[GrammarContract]) -> Vec<VaultGrammarContract> {
    let mut out = records
        .iter()
        .map(|record| VaultGrammarContract {
            id: record.id.clone(),
            decoder: record.decoder.as_ref().map(ToString::to_string),
            encoder: record.encoder.as_ref().map(ToString::to_string),
            surface: record.surface.as_ref().map(ToString::to_string),
            round_trip: record.round_trip,
        })
        .collect::<Vec<_>>();
    out.sort();
    out
}

fn endpoint_index(nodes: &[VaultNode]) -> Result<BTreeMap<String, VaultEndpoint>, String> {
    let mut endpoints = BTreeMap::new();
    for node in nodes {
        let endpoint = node.endpoint();
        if !is_index_id(&endpoint.id) {
            return Err(format!("vault graph invalid id {}", endpoint.summary()));
        }
        if endpoint.kind != VaultNodeKind::Draft {
            endpoints.insert(endpoint.id.clone(), endpoint);
        }
    }
    Ok(endpoints)
}

fn derive_relations(
    doc: &IndexDoc,
    endpoints: &BTreeMap<String, VaultEndpoint>,
) -> Result<Vec<VaultRelation>, String> {
    let mut relations = Vec::new();
    for anchor in &doc.anchors {
        relations.push(owns(
            &anchor.subject.to_string(),
            VaultEndpoint::new(VaultNodeKind::Anchor, anchor.id.as_str()),
        ));
    }
    for surface in &doc.surfaces {
        relations.push(owns(
            &surface.subject.to_string(),
            VaultEndpoint::new(VaultNodeKind::Surface, surface.id.as_str()),
        ));
    }
    for specimen in &doc.specimens {
        let specimen_endpoint = VaultEndpoint::new(VaultNodeKind::Specimen, specimen.id.as_str());
        relations.push(owns(
            &specimen.subject.to_string(),
            specimen_endpoint.clone(),
        ));
        push_doc_anchor(&mut relations, specimen_endpoint, &specimen.doc_anchor);
    }
    for draft in &doc.drafts {
        let draft_endpoint = VaultEndpoint::new(VaultNodeKind::Draft, draft.id.as_str());
        relations.push(owns(&draft.subject.to_string(), draft_endpoint.clone()));
        push_claims(
            &mut relations,
            &draft_endpoint,
            "claims-anchor",
            VaultNodeKind::Anchor,
            draft.claims_anchors.iter(),
        );
        push_claims(
            &mut relations,
            &draft_endpoint,
            "claims-surface",
            VaultNodeKind::Surface,
            draft.claims_surfaces.iter(),
        );
        push_claims(
            &mut relations,
            &draft_endpoint,
            "claims-specimen",
            VaultNodeKind::Specimen,
            draft.claims_specimens.iter(),
        );
        push_grammar_relations(&mut relations, &draft_endpoint, &draft.grammar_contracts);
        push_doc_anchor(&mut relations, draft_endpoint, &draft.doc_anchor);
    }
    for feature in &doc.features {
        let feature_endpoint = VaultEndpoint::new(VaultNodeKind::Feature, feature.id.as_str());
        relations.push(owns(&feature.subject.to_string(), feature_endpoint.clone()));
        push_claims(
            &mut relations,
            &feature_endpoint,
            "claims-anchor",
            VaultNodeKind::Anchor,
            feature.anchors.iter(),
        );
        push_claims(
            &mut relations,
            &feature_endpoint,
            "claims-surface",
            VaultNodeKind::Surface,
            feature.surfaces.iter(),
        );
        push_claims(
            &mut relations,
            &feature_endpoint,
            "claims-specimen",
            VaultNodeKind::Specimen,
            feature.specimens.iter(),
        );
        push_grammar_relations(
            &mut relations,
            &feature_endpoint,
            &feature.grammar_contracts,
        );
        push_doc_anchor(&mut relations, feature_endpoint, &feature.doc_anchor);
    }
    for route in &doc.routes {
        let route_endpoint = VaultEndpoint::new(VaultNodeKind::Route, route.id.as_str());
        for (order, step) in route.steps.iter().enumerate() {
            relations.push(VaultRelation::new(
                route_endpoint.clone(),
                "route-step",
                route_step_endpoint(step),
                Some(order),
            ));
        }
        push_doc_anchor(&mut relations, route_endpoint, &route.doc_anchor);
    }
    for edge in &doc.edges {
        relations.push(index_edge_relation(edge, endpoints)?);
    }
    Ok(relations)
}

fn owns(subject: &str, to: VaultEndpoint) -> VaultRelation {
    VaultRelation::new(
        VaultEndpoint::new(VaultNodeKind::Subject, subject),
        "owns",
        to,
        None,
    )
}

fn push_doc_anchor(
    relations: &mut Vec<VaultRelation>,
    from: VaultEndpoint,
    anchor: &Option<AnchorId>,
) {
    if let Some(anchor) = anchor {
        relations.push(VaultRelation::new(
            from,
            "documents",
            VaultEndpoint::new(VaultNodeKind::Anchor, anchor.as_str()),
            None,
        ));
    }
}

fn push_claims<'a, T>(
    relations: &mut Vec<VaultRelation>,
    from: &VaultEndpoint,
    rel: &'static str,
    kind: VaultNodeKind,
    claims: impl Iterator<Item = &'a T>,
) where
    T: ToString + 'a,
{
    let mut claims = claims.map(ToString::to_string).collect::<Vec<_>>();
    claims.sort();
    for claim in claims {
        relations.push(VaultRelation::new(
            from.clone(),
            rel,
            VaultEndpoint::new(kind, claim),
            None,
        ));
    }
}

fn push_grammar_relations(
    relations: &mut Vec<VaultRelation>,
    from: &VaultEndpoint,
    contracts: &[GrammarContract],
) {
    let contracts = grammar_contracts(contracts);
    for (order, contract) in contracts.iter().enumerate() {
        if let Some(decoder) = &contract.decoder {
            relations.push(VaultRelation::new(
                from.clone(),
                "grammar-decoder",
                VaultEndpoint::new(VaultNodeKind::Anchor, decoder),
                Some(order),
            ));
        }
        if let Some(encoder) = &contract.encoder {
            relations.push(VaultRelation::new(
                from.clone(),
                "grammar-encoder",
                VaultEndpoint::new(VaultNodeKind::Anchor, encoder),
                Some(order),
            ));
        }
        if let Some(surface) = &contract.surface {
            relations.push(VaultRelation::new(
                from.clone(),
                "grammar-surface",
                VaultEndpoint::new(VaultNodeKind::Surface, surface),
                Some(order),
            ));
        }
    }
}

fn index_edge_relation(
    edge: &IndexEdge,
    endpoints: &BTreeMap<String, VaultEndpoint>,
) -> Result<VaultRelation, String> {
    let from = endpoints
        .get(&edge.from)
        .ok_or_else(|| format!("vault graph missing edge endpoint {}", edge.from))?;
    let to = endpoints
        .get(&edge.to)
        .ok_or_else(|| format!("vault graph missing edge endpoint {}", edge.to))?;
    Ok(VaultRelation::new(
        from.clone(),
        format!("index-edge:{}", edge.rel),
        to.clone(),
        None,
    ))
}

fn sorted_relations(relations: impl IntoIterator<Item = VaultRelation>) -> Vec<VaultRelation> {
    let mut out = relations.into_iter().collect::<Vec<_>>();
    out.sort();
    out
}

fn reject_duplicate_nodes(nodes: &[VaultNode]) -> Result<(), String> {
    let mut seen_endpoints = BTreeSet::new();
    let mut seen_ids = BTreeSet::new();
    for endpoint in nodes.iter().map(VaultNode::endpoint) {
        if !seen_endpoints.insert(endpoint.clone()) {
            return Err(format!("vault graph duplicate node {}", endpoint.summary()));
        }
        if !seen_ids.insert(endpoint.id.clone()) {
            return Err(format!("vault graph duplicate id {}", endpoint.id));
        }
        if !is_index_id(&endpoint.id) {
            return Err(format!("vault graph invalid id {}", endpoint.summary()));
        }
    }
    Ok(())
}

fn reject_duplicate_relations(kind: &str, relations: &[VaultRelation]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for relation in relations {
        if !seen.insert(relation) {
            return Err(format!(
                "vault graph duplicate {kind} relation {}",
                relation.summary()
            ));
        }
    }
    Ok(())
}
