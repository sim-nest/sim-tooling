use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use crate::{
    generated_artifact::GeneratedArtifact,
    index_vault_graph::{
        VaultEndpoint, VaultFeature, VaultFeatureDraft, VaultGrammarContract, VaultGranularity,
        VaultGraph, VaultNode, VaultNodeKind, VaultRelation, VaultRoute, VaultSpecimen,
    },
    index_vault_link::{
        LinkTargetStyle, MetadataStyle, NoteRef, OutlineStyle, escape_markdown_label,
    },
    index_vault_profile::VaultProfile,
    index_vault_render::VaultRenderCoverage,
};

pub(crate) struct RenderState<'a> {
    graph: &'a VaultGraph,
    profile: &'a VaultProfile,
    granularity: VaultGranularity,
    notes: BTreeMap<VaultEndpoint, NoteRef>,
    labels: BTreeMap<VaultEndpoint, String>,
    represented: BTreeSet<VaultEndpoint>,
    relation_summaries: BTreeSet<String>,
    link_targets: BTreeSet<String>,
}

impl<'a> RenderState<'a> {
    pub(crate) fn new(
        graph: &'a VaultGraph,
        profile: &'a VaultProfile,
        granularity: VaultGranularity,
        notes: BTreeMap<VaultEndpoint, NoteRef>,
        labels: BTreeMap<VaultEndpoint, String>,
    ) -> Self {
        Self {
            graph,
            profile,
            granularity,
            notes,
            labels,
            represented: BTreeSet::new(),
            relation_summaries: BTreeSet::new(),
            link_targets: BTreeSet::new(),
        }
    }

    pub(crate) fn readme(&mut self) -> Result<GeneratedArtifact, String> {
        let mut out = self.header(
            "SIM Index Vault",
            &[
                ("profile", self.profile.id.to_owned()),
                (
                    "granularity",
                    granularity_label(self.granularity).to_owned(),
                ),
                ("schema", self.graph.schema.clone()),
                ("generated-by", self.graph.generated_by.clone()),
            ],
        );
        self.section(&mut out, "Navigation");
        for kind in note_kinds(self.granularity) {
            let endpoints = self
                .graph
                .nodes
                .iter()
                .map(VaultNode::endpoint)
                .filter(|endpoint| endpoint.kind == kind)
                .collect::<Vec<_>>();
            if endpoints.is_empty() {
                continue;
            }
            self.subsection(&mut out, plural_kind(kind));
            for endpoint in endpoints {
                let link = self.root_link(&endpoint)?;
                self.list_line(&mut out, 0, &link);
            }
        }
        self.artifact("README.md", out)
    }

    pub(crate) fn note(&mut self, node: &VaultNode) -> Result<GeneratedArtifact, String> {
        let endpoint = node.endpoint();
        self.represent(&endpoint);
        let note = self
            .note_for(&endpoint)
            .ok_or_else(|| format!("missing note for {}", endpoint.summary()))?
            .clone();
        let mut out = self.header(&node_title(node), &self.node_properties(node));
        self.node_body(&mut out, node, &note)?;
        self.relations(&mut out, &endpoint, &note)?;
        self.artifact(note.path_str(), out)
    }

    pub(crate) fn note_for(&self, endpoint: &VaultEndpoint) -> Option<&NoteRef> {
        self.notes.get(endpoint)
    }

    pub(crate) fn coverage(&self) -> VaultRenderCoverage {
        VaultRenderCoverage {
            rows: self.graph.coverage.rows.clone(),
            represented: self.represented.clone(),
            relation_summaries: self.relation_summaries.clone(),
            link_targets: self.link_targets.clone(),
        }
    }

    pub(crate) fn unresolved_links(&self, artifact_paths: &BTreeSet<String>) -> Vec<String> {
        self.link_targets
            .difference(artifact_paths)
            .cloned()
            .collect()
    }

    fn node_body(
        &mut self,
        out: &mut String,
        node: &VaultNode,
        source: &NoteRef,
    ) -> Result<(), String> {
        match node {
            VaultNode::Subject(subject) => {
                self.section(out, "Subject");
                self.field(out, "Kind", &code(&subject.kind));
                self.field(out, "Title", &subject.title);
            }
            VaultNode::Anchor(anchor) => {
                self.section(out, "Anchor");
                self.field(out, "Kind", &code(&anchor.kind));
                let subject = subject_endpoint(&anchor.subject);
                let text = self.endpoint_text(source, subject)?;
                self.field(out, "Subject", &text);
            }
            VaultNode::Surface(surface) => {
                self.section(out, "Surface");
                self.field(out, "Kind", &code(&surface.kind));
                let subject = subject_endpoint(&surface.subject);
                let text = self.endpoint_text(source, subject)?;
                self.field(out, "Subject", &text);
            }
            VaultNode::Specimen(specimen) => self.specimen_body(out, specimen, source)?,
            VaultNode::Draft(draft) => self.draft_body(out, draft, source)?,
            VaultNode::Feature(feature) => self.feature_body(out, feature, source)?,
            VaultNode::Route(route) => self.route_body(out, route, source)?,
        }
        Ok(())
    }

    fn specimen_body(
        &mut self,
        out: &mut String,
        specimen: &VaultSpecimen,
        source: &NoteRef,
    ) -> Result<(), String> {
        self.section(out, "Specimen");
        let subject = self.endpoint_text(source, subject_endpoint(&specimen.subject))?;
        self.field(out, "Subject", &subject);
        self.field(out, "Kind", &code(&specimen.kind));
        self.field(out, "Source path", &code(&specimen.path));
        self.field(
            out,
            "Language",
            specimen.language.as_deref().unwrap_or_default(),
        );
        self.field(out, "Runnable", bool_text(specimen.runnable));
        self.field(out, "Checked", bool_text(specimen.checked));
        self.field(
            out,
            "Checked by",
            specimen.checked_by.as_deref().unwrap_or(""),
        );
        if let Some(anchor) = &specimen.doc_anchor {
            let text = self.endpoint_text(source, anchor_endpoint(anchor))?;
            self.field(out, "Doc anchor", &text);
        }
        Ok(())
    }

    fn draft_body(
        &mut self,
        out: &mut String,
        draft: &VaultFeatureDraft,
        source: &NoteRef,
    ) -> Result<(), String> {
        self.section(out, "Draft");
        let subject = self.endpoint_text(source, subject_endpoint(&draft.subject))?;
        self.field(out, "Subject", &subject);
        self.field(out, "Summary", &draft.summary);
        self.claims(
            out,
            source,
            &draft.claims_anchors,
            &draft.claims_surfaces,
            &draft.claims_specimens,
        )?;
        self.grammar_contracts(out, source, &draft.grammar_contracts)?;
        Ok(())
    }

    fn feature_body(
        &mut self,
        out: &mut String,
        feature: &VaultFeature,
        source: &NoteRef,
    ) -> Result<(), String> {
        self.section(out, "Feature");
        let subject = self.endpoint_text(source, subject_endpoint(&feature.subject))?;
        self.field(out, "Subject", &subject);
        self.field(out, "Canonical key", &code(&feature.key));
        self.field(out, "Summary", &feature.summary);
        self.claims(
            out,
            source,
            &feature.anchors,
            &feature.surfaces,
            &feature.specimens,
        )?;
        self.grammar_contracts(out, source, &feature.grammar_contracts)?;
        Ok(())
    }

    fn route_body(
        &mut self,
        out: &mut String,
        route: &VaultRoute,
        source: &NoteRef,
    ) -> Result<(), String> {
        self.section(out, "Route");
        self.field(out, "Title", &route.title);
        self.field(out, "Audiences", &route.audiences.join(", "));
        self.subsection(out, "Route Steps");
        for step in &route.steps {
            let target = self.endpoint_text(source, step.target.clone())?;
            self.list_line(
                out,
                0,
                &format!("{}: {target} - {}", step.order + 1, step.why),
            );
        }
        Ok(())
    }

    fn claims(
        &mut self,
        out: &mut String,
        source: &NoteRef,
        anchors: &[String],
        surfaces: &[String],
        specimens: &[String],
    ) -> Result<(), String> {
        self.subsection(out, "Claims");
        self.id_list(out, source, "Anchors", VaultNodeKind::Anchor, anchors)?;
        self.id_list(out, source, "Surfaces", VaultNodeKind::Surface, surfaces)?;
        self.id_list(out, source, "Specimens", VaultNodeKind::Specimen, specimens)?;
        Ok(())
    }

    fn grammar_contracts(
        &mut self,
        out: &mut String,
        source: &NoteRef,
        contracts: &[VaultGrammarContract],
    ) -> Result<(), String> {
        if contracts.is_empty() {
            return Ok(());
        }
        self.subsection(out, "Grammar Contracts");
        for contract in contracts {
            self.list_line(
                out,
                0,
                &format!(
                    "{} round-trip {}",
                    code(&contract.id),
                    bool_text(contract.round_trip)
                ),
            );
            if let Some(decoder) = &contract.decoder {
                let text = self.endpoint_text(source, anchor_endpoint(decoder))?;
                self.list_line(out, 1, &format!("decoder {text}"));
            }
            if let Some(encoder) = &contract.encoder {
                let text = self.endpoint_text(source, anchor_endpoint(encoder))?;
                self.list_line(out, 1, &format!("encoder {text}"));
            }
            if let Some(surface) = &contract.surface {
                let text = self.endpoint_text(source, surface_endpoint(surface))?;
                self.list_line(out, 1, &format!("surface {text}"));
            }
        }
        Ok(())
    }

    fn relations(
        &mut self,
        out: &mut String,
        endpoint: &VaultEndpoint,
        source: &NoteRef,
    ) -> Result<(), String> {
        self.section(out, self.profile.outline.relation_heading);
        self.relation_list(
            out,
            "Outgoing",
            source,
            relations_from(&self.graph.relations, endpoint),
            false,
        )?;
        self.relation_list(
            out,
            "Incoming",
            source,
            relations_from(&self.graph.reverse_relations, endpoint),
            true,
        )?;
        Ok(())
    }

    fn relation_list(
        &mut self,
        out: &mut String,
        title: &str,
        source: &NoteRef,
        relations: Vec<&VaultRelation>,
        incoming: bool,
    ) -> Result<(), String> {
        self.subsection(out, title);
        if relations.is_empty() {
            self.list_line(out, 0, "none");
            return Ok(());
        }
        for relation in relations {
            self.represent(&relation.from);
            self.represent(&relation.to);
            if incoming {
                self.relation_summaries
                    .insert(relation.reversed().summary());
            } else {
                self.relation_summaries.insert(relation.summary());
            }
            let target = self.endpoint_text(source, relation.to.clone())?;
            let label = match relation.order {
                Some(order) => format!("{}[{}]", relation.rel, order + 1),
                None => relation.rel.clone(),
            };
            self.list_line(out, 0, &format!("{} -> {target}", code(&label)));
        }
        Ok(())
    }

    fn id_list(
        &mut self,
        out: &mut String,
        source: &NoteRef,
        title: &str,
        kind: VaultNodeKind,
        ids: &[String],
    ) -> Result<(), String> {
        if ids.is_empty() {
            return Ok(());
        }
        self.list_line(out, 0, &format!("{title}:"));
        for id in ids {
            let text = self.endpoint_text(source, VaultEndpoint::new(kind, id))?;
            self.list_line(out, 1, &text);
        }
        Ok(())
    }

    fn endpoint_text(
        &mut self,
        source: &NoteRef,
        endpoint: VaultEndpoint,
    ) -> Result<String, String> {
        self.represent(&endpoint);
        let label = self
            .labels
            .get(&endpoint)
            .cloned()
            .unwrap_or_else(|| endpoint.id.clone());
        let Some(target) = self.note_for(&endpoint) else {
            return Ok(code(&endpoint.id));
        };
        let path = target.path_str().to_owned();
        let text = match self.profile.links.style {
            LinkTargetStyle::RelativeCommonMark => target.commonmark_link_from(source, &label),
            LinkTargetStyle::VaultRootWikilink => {
                target.vault_root_wikilink(self.profile.links.root_namespace, &label)
            }
        };
        self.link_targets.insert(path);
        Ok(text)
    }

    fn root_link(&mut self, endpoint: &VaultEndpoint) -> Result<String, String> {
        self.represent(endpoint);
        let target = self
            .note_for(endpoint)
            .ok_or_else(|| format!("missing note for {}", endpoint.summary()))?;
        let label = self
            .labels
            .get(endpoint)
            .cloned()
            .unwrap_or_else(|| endpoint.id.clone());
        let path = target.path_str().to_owned();
        let text = match self.profile.links.style {
            LinkTargetStyle::RelativeCommonMark => {
                format!("[{}]({})", escape_markdown_label(&label), path)
            }
            LinkTargetStyle::VaultRootWikilink => {
                target.vault_root_wikilink(self.profile.links.root_namespace, &label)
            }
        };
        self.link_targets.insert(path);
        Ok(text)
    }

    fn represent(&mut self, endpoint: &VaultEndpoint) {
        self.represented.insert(endpoint.clone());
    }

    fn header(&self, title: &str, properties: &[(&str, String)]) -> String {
        let mut out = String::new();
        match self.profile.metadata.style {
            MetadataStyle::YamlProperties => {
                out.push_str("---\n");
                out.push_str(&format!(
                    "{}: {}\n",
                    self.profile.metadata.profile_property,
                    yaml(self.profile.id)
                ));
                for (key, value) in properties {
                    out.push_str(&format!("{key}: {}\n", yaml(value)));
                }
                out.push_str("---\n\n");
            }
            MetadataStyle::LogseqProperties => {
                out.push_str(&format!(
                    "{}:: {}\n",
                    self.profile.metadata.profile_property, self.profile.id
                ));
                for (key, value) in properties {
                    out.push_str(&format!("{key}:: {value}\n"));
                }
                out.push('\n');
            }
        }
        match self.profile.outline.style {
            OutlineStyle::CommonMarkHeadings => out.push_str(&format!("# {title}\n\n")),
            OutlineStyle::IndentedBlocks => out.push_str(&format!("- {title}\n")),
        }
        out
    }

    fn node_properties(&self, node: &VaultNode) -> Vec<(&str, String)> {
        let endpoint = node.endpoint();
        vec![
            (self.profile.metadata.id_property, endpoint.id),
            ("kind", endpoint.kind.label().to_owned()),
            (
                "granularity",
                granularity_label(self.granularity).to_owned(),
            ),
            ("schema", self.graph.schema.clone()),
            ("generated-by", self.graph.generated_by.clone()),
        ]
    }

    fn section(&self, out: &mut String, title: &str) {
        match self.profile.outline.style {
            OutlineStyle::CommonMarkHeadings => out.push_str(&format!("## {title}\n\n")),
            OutlineStyle::IndentedBlocks => out.push_str(&format!("  - {title}\n")),
        }
    }

    fn subsection(&self, out: &mut String, title: &str) {
        match self.profile.outline.style {
            OutlineStyle::CommonMarkHeadings => out.push_str(&format!("### {title}\n\n")),
            OutlineStyle::IndentedBlocks => out.push_str(&format!("    - {title}\n")),
        }
    }

    fn field(&self, out: &mut String, key: &str, value: &str) {
        self.list_line(out, 0, &format!("{key}: {value}"));
    }

    fn list_line(&self, out: &mut String, indent: usize, text: &str) {
        match self.profile.outline.style {
            OutlineStyle::CommonMarkHeadings => {
                out.push_str(&format!("{}- {text}\n", "  ".repeat(indent)));
            }
            OutlineStyle::IndentedBlocks => {
                out.push_str(&format!("{}- {text}\n", "  ".repeat(indent + 2)));
            }
        }
    }

    fn artifact(&self, path: &str, mut out: String) -> Result<GeneratedArtifact, String> {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        GeneratedArtifact::new(path, out.into_bytes())
    }
}

pub(crate) fn note_index(
    graph: &VaultGraph,
    granularity: VaultGranularity,
) -> Result<BTreeMap<VaultEndpoint, NoteRef>, String> {
    graph
        .nodes
        .iter()
        .map(VaultNode::endpoint)
        .filter(|endpoint| note_kinds(granularity).contains(&endpoint.kind))
        .map(|endpoint| NoteRef::new(endpoint.clone()).map(|note| (endpoint, note)))
        .collect()
}

pub(crate) fn validate_source_paths(graph: &VaultGraph) -> Result<(), String> {
    for node in &graph.nodes {
        if let VaultNode::Specimen(specimen) = node {
            let path = &specimen.path;
            if path.contains('\\') || Path::new(path).is_absolute() || looks_windows_absolute(path)
            {
                return Err(format!(
                    "vault specimen source path must be relative with `/` separators: `{path}`"
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn granularity_label(granularity: VaultGranularity) -> &'static str {
    match granularity {
        VaultGranularity::Compact => "compact",
        VaultGranularity::Full => "full",
    }
}

fn relations_from<'a>(
    relations: &'a [VaultRelation],
    endpoint: &VaultEndpoint,
) -> Vec<&'a VaultRelation> {
    let mut out = relations
        .iter()
        .filter(|relation| relation.from == *endpoint)
        .collect::<Vec<_>>();
    out.sort_by(|left, right| {
        (
            left.order.unwrap_or(usize::MAX),
            left.rel.as_str(),
            left.to.kind,
            left.to.id.as_str(),
        )
            .cmp(&(
                right.order.unwrap_or(usize::MAX),
                right.rel.as_str(),
                right.to.kind,
                right.to.id.as_str(),
            ))
    });
    out
}

fn node_title(node: &VaultNode) -> String {
    match node {
        VaultNode::Subject(node) => node.title.clone(),
        VaultNode::Anchor(node) => node.id.clone(),
        VaultNode::Surface(node) => node.id.clone(),
        VaultNode::Specimen(node) => node.id.clone(),
        VaultNode::Draft(node) => node.title.clone(),
        VaultNode::Feature(node) => node.title.clone(),
        VaultNode::Route(node) => node.title.clone(),
    }
}

fn note_kinds(granularity: VaultGranularity) -> Vec<VaultNodeKind> {
    let mut kinds = vec![
        VaultNodeKind::Subject,
        VaultNodeKind::Feature,
        VaultNodeKind::Surface,
        VaultNodeKind::Specimen,
        VaultNodeKind::Route,
        VaultNodeKind::Draft,
    ];
    if granularity == VaultGranularity::Full {
        kinds.push(VaultNodeKind::Anchor);
    }
    kinds
}

fn plural_kind(kind: VaultNodeKind) -> &'static str {
    match kind {
        VaultNodeKind::Anchor => "Anchors",
        VaultNodeKind::Draft => "Drafts",
        VaultNodeKind::Feature => "Features",
        VaultNodeKind::Route => "Routes",
        VaultNodeKind::Specimen => "Specimens",
        VaultNodeKind::Subject => "Subjects",
        VaultNodeKind::Surface => "Surfaces",
    }
}

fn subject_endpoint(id: &str) -> VaultEndpoint {
    VaultEndpoint::new(VaultNodeKind::Subject, id)
}

fn anchor_endpoint(id: &str) -> VaultEndpoint {
    VaultEndpoint::new(VaultNodeKind::Anchor, id)
}

fn surface_endpoint(id: &str) -> VaultEndpoint {
    VaultEndpoint::new(VaultNodeKind::Surface, id)
}

fn looks_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn code(value: &str) -> String {
    format!("`{}`", value.replace('`', "\\`"))
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn yaml(value: &str) -> String {
    crate::index_vault_link::yaml_property_value(value)
}
