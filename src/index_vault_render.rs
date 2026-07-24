use std::collections::{BTreeMap, BTreeSet};

use crate::{
    generated_artifact::ArtifactSet,
    index_vault_graph::{VaultEndpoint, VaultGranularity, VaultGraph},
    index_vault_profile::{PROFILES, VaultProfile},
    index_vault_render_writer::{
        RenderState, granularity_label, note_index, validate_source_paths,
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultRender {
    pub(crate) artifacts: ArtifactSet,
    coverage: VaultRenderCoverage,
    unresolved_links: Vec<String>,
}

impl VaultRender {
    pub(crate) fn coverage(&self) -> &VaultRenderCoverage {
        &self.coverage
    }

    pub(crate) fn unresolved_links(&self) -> &[String] {
        &self.unresolved_links
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultRenderCoverage {
    pub(crate) rows: BTreeSet<VaultEndpoint>,
    pub(crate) represented: BTreeSet<VaultEndpoint>,
    pub(crate) relation_summaries: BTreeSet<String>,
    pub(crate) link_targets: BTreeSet<String>,
}

impl VaultRenderCoverage {
    pub(crate) fn unrepresented_rows(&self) -> usize {
        self.rows.difference(&self.represented).count()
    }

    pub(crate) fn represented_rows(&self) -> usize {
        self.represented.len()
    }

    pub(crate) fn represented_relations(&self) -> usize {
        self.relation_summaries.len()
    }

    pub(crate) fn link_targets(&self) -> usize {
        self.link_targets.len()
    }
}

pub(crate) fn check_all_vault_renders(graph: &VaultGraph) -> Result<(), String> {
    for profile in PROFILES {
        for granularity in [VaultGranularity::Compact, VaultGranularity::Full] {
            let rendered = render_vault(graph, profile, granularity)?;
            check_render_report(graph, profile, granularity, &rendered)?;
        }
    }
    Ok(())
}

pub(crate) fn render_vault(
    graph: &VaultGraph,
    profile: &VaultProfile,
    granularity: VaultGranularity,
) -> Result<VaultRender, String> {
    graph.check(granularity)?;
    validate_source_paths(graph)?;
    let notes = note_index(graph, granularity)?;
    let labels = label_index(graph);
    let mut state = RenderState::new(graph, profile, granularity, notes, labels);
    let mut artifacts = vec![state.readme()?];
    for node in &graph.nodes {
        if state.note_for(&node.endpoint()).is_some() {
            artifacts.push(state.note(node)?);
        }
    }
    let artifacts = ArtifactSet::new(artifacts)?;
    let artifact_paths = artifacts
        .iter()
        .map(|artifact| artifact.path_str().to_owned())
        .collect::<BTreeSet<_>>();
    let unresolved_links = state.unresolved_links(&artifact_paths);
    let coverage = state.coverage();
    let rendered = VaultRender {
        artifacts,
        coverage,
        unresolved_links,
    };
    check_render_report(graph, profile, granularity, &rendered)?;
    Ok(rendered)
}

fn check_render_report(
    graph: &VaultGraph,
    profile: &VaultProfile,
    granularity: VaultGranularity,
    rendered: &VaultRender,
) -> Result<(), String> {
    if rendered.coverage().unrepresented_rows() != 0 {
        return Err(format!(
            "{} {} leaves {} row(s) unrepresented",
            profile.id,
            granularity_label(granularity),
            rendered.coverage().unrepresented_rows()
        ));
    }
    if rendered.coverage().represented_rows() != graph.nodes.len() {
        return Err(format!(
            "{} {} represents {} row(s), expected {}",
            profile.id,
            granularity_label(granularity),
            rendered.coverage().represented_rows(),
            graph.nodes.len()
        ));
    }
    if rendered.coverage().represented_relations() != graph.relations.len() {
        return Err(format!(
            "{} {} represents {} relation(s), expected {}",
            profile.id,
            granularity_label(granularity),
            rendered.coverage().represented_relations(),
            graph.relations.len()
        ));
    }
    if !graph.nodes.is_empty() && rendered.coverage().link_targets() == 0 {
        return Err(format!(
            "{} {} produced no note links",
            profile.id,
            granularity_label(granularity)
        ));
    }
    if !rendered.unresolved_links().is_empty() {
        return Err(format!(
            "{} {} has unresolved link(s): {}",
            profile.id,
            granularity_label(granularity),
            rendered.unresolved_links().join(", ")
        ));
    }
    Ok(())
}

fn label_index(graph: &VaultGraph) -> BTreeMap<VaultEndpoint, String> {
    graph
        .nodes
        .iter()
        .map(|node| (node.endpoint(), node_title(node)))
        .collect()
}

fn node_title(node: &crate::index_vault_graph::VaultNode) -> String {
    match node {
        crate::index_vault_graph::VaultNode::Subject(node) => node.title.clone(),
        crate::index_vault_graph::VaultNode::Anchor(node) => node.id.clone(),
        crate::index_vault_graph::VaultNode::Surface(node) => node.id.clone(),
        crate::index_vault_graph::VaultNode::Specimen(node) => node.id.clone(),
        crate::index_vault_graph::VaultNode::Draft(node) => node.title.clone(),
        crate::index_vault_graph::VaultNode::Feature(node) => node.title.clone(),
        crate::index_vault_graph::VaultNode::Route(node) => node.title.clone(),
    }
}
