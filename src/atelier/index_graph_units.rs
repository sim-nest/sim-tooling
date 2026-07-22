use std::{
    collections::BTreeMap,
    fs,
    path::{MAIN_SEPARATOR, Path},
};

use sim_codec_index::{IndexCodec, IndexForm};
use sim_index_core::{
    DiscoveredSpecimen, DiscoveredSurface, FeatureRecord, GrammarContract, IndexDoc, RouteRecord,
    RouteStep, SubjectRecord, check_index_doc,
};

use super::{index_manifest::RepoEntry, index_units::SourceUnit, io::display_io};

pub(super) fn collect_index_graph_units(repos: &[RepoEntry]) -> Result<Vec<SourceUnit>, String> {
    let Some(sim_say) = repos.iter().find(|repo| repo.name == "sim-say") else {
        return Ok(Vec::new());
    };
    let source = sim_say.checkout_path.join("docs/index/index.sx");
    if !source.is_file() {
        return Ok(Vec::new());
    }

    let doc = read_index_doc(&source)?;
    let graph_path = display_path(sim_say, &source);
    let mut units = Vec::new();
    units.extend(subject_units(&doc, sim_say, &graph_path));
    units.extend(surface_units(&doc, sim_say, &graph_path));
    units.extend(specimen_units(&doc, sim_say, &graph_path));
    units.extend(feature_units(&doc, sim_say, &graph_path));
    units.extend(grammar_units(&doc, sim_say, &graph_path));
    units.extend(route_units(&doc, sim_say, &graph_path));
    units.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(units)
}

fn read_index_doc(path: &Path) -> Result<IndexDoc, String> {
    let source = fs::read_to_string(path).map_err(display_io)?;
    let form = if source.trim_start().starts_with('{') {
        IndexForm::Json
    } else {
        IndexForm::Sx
    };
    let doc = IndexCodec
        .decode(form, &source)
        .map_err(|err| format!("decode {}: {err}", path.display()))?;
    check_index_doc(&doc).map_err(|err| format!("invalid index {}: {err}", path.display()))?;
    Ok(doc)
}

fn subject_units(doc: &IndexDoc, sim_say: &RepoEntry, path: &str) -> Vec<SourceUnit> {
    let source = GraphSource { sim_say, path };
    let contained_by = contained_by(doc);
    doc.subjects
        .iter()
        .map(|subject| {
            let contained = doc
                .edges
                .iter()
                .filter(|edge| edge.rel == "contains" && edge.from == subject.id.as_str())
                .map(|edge| edge.to.clone())
                .collect::<Vec<_>>();
            let parents = contained_by
                .get(subject.id.as_str())
                .cloned()
                .unwrap_or_default();
            let kind = subject_unit_kind(subject);
            graph_unit(
                &source,
                kind,
                subject.id.as_str(),
                &subject.title,
                [
                    format!("id: {}", subject.id),
                    format!("kind: {}", subject.kind),
                    format!("title: {}", subject.title),
                    format!("contained_by: {}", parents.join(" ")),
                    format!("contains: {}", contained.join(" ")),
                    "panel: already exists".to_owned(),
                ]
                .join("\n"),
                parents.into_iter().chain(contained).collect(),
                &["already-exists"],
            )
        })
        .collect()
}

fn surface_units(doc: &IndexDoc, sim_say: &RepoEntry, path: &str) -> Vec<SourceUnit> {
    let source = GraphSource { sim_say, path };
    doc.surfaces
        .iter()
        .map(|surface| {
            graph_unit(
                &source,
                "surface",
                surface.id.as_str(),
                surface.id.as_str(),
                surface_body(doc, surface),
                vec![surface.subject.to_string()],
                &["already-exists"],
            )
        })
        .collect()
}

fn specimen_units(doc: &IndexDoc, sim_say: &RepoEntry, path: &str) -> Vec<SourceUnit> {
    let source = GraphSource { sim_say, path };
    doc.specimens
        .iter()
        .map(|specimen| {
            graph_unit(
                &source,
                "specimen",
                specimen.id.as_str(),
                &specimen.path,
                specimen_body(doc, specimen),
                vec![specimen.subject.to_string()],
                &["run-this-example"],
            )
        })
        .collect()
}

fn feature_units(doc: &IndexDoc, sim_say: &RepoEntry, path: &str) -> Vec<SourceUnit> {
    let source = GraphSource { sim_say, path };
    doc.features
        .iter()
        .map(|feature| {
            let mut panels = vec!["already-exists"];
            if feature_has_runnable_specimen(doc, feature) {
                panels.push("run-this-example");
            }
            graph_unit(
                &source,
                "feature",
                feature.id.as_str(),
                &feature.title,
                feature_body(doc, feature),
                feature_related_ids(feature),
                &panels,
            )
        })
        .collect()
}

fn grammar_units(doc: &IndexDoc, sim_say: &RepoEntry, path: &str) -> Vec<SourceUnit> {
    let source = GraphSource { sim_say, path };
    let mut by_id = BTreeMap::new();
    for feature in &doc.features {
        for grammar in &feature.grammar_contracts {
            by_id.entry(grammar.id.clone()).or_insert_with(|| {
                graph_unit(
                    &source,
                    "grammar",
                    &grammar.id,
                    &grammar.id,
                    grammar_body(feature, grammar),
                    grammar_related_ids(feature, grammar),
                    &["already-exists"],
                )
            });
        }
    }
    by_id.into_values().collect()
}

fn route_units(doc: &IndexDoc, sim_say: &RepoEntry, path: &str) -> Vec<SourceUnit> {
    let source = GraphSource { sim_say, path };
    doc.routes
        .iter()
        .map(|route| {
            let mut panels = vec!["reuse-route"];
            if route
                .steps
                .iter()
                .any(|step| matches!(step, RouteStep::Specimen { .. }))
            {
                panels.push("run-this-example");
            }
            graph_unit(
                &source,
                "route",
                route.id.as_str(),
                &route.title,
                route_body(doc, route),
                route
                    .steps
                    .iter()
                    .map(|step| step.id().to_owned())
                    .collect(),
                &panels,
            )
        })
        .collect()
}

struct GraphSource<'a> {
    sim_say: &'a RepoEntry,
    path: &'a str,
}

fn graph_unit(
    source: &GraphSource<'_>,
    kind: &str,
    id: &str,
    title: &str,
    body: String,
    related_ids: Vec<String>,
    panels: &[&str],
) -> SourceUnit {
    let text = format!("# {title}\n{body}\n");
    SourceUnit {
        id: id.to_owned(),
        repo: source.sim_say.name.clone(),
        crate_name: None,
        kind: kind.to_owned(),
        path: source.path.to_owned(),
        line: 1,
        text,
        graph_id: Some(id.to_owned()),
        related_ids,
        panels: panels.iter().map(|panel| (*panel).to_owned()).collect(),
    }
}

fn subject_unit_kind(subject: &SubjectRecord) -> &'static str {
    match subject.kind.as_str() {
        "repo" | "crate" | "runtime-lib" => "package",
        "language" => "language",
        "grammar" => "grammar",
        _ => "subject",
    }
}

fn contained_by(doc: &IndexDoc) -> BTreeMap<&str, Vec<String>> {
    let mut out: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for edge in &doc.edges {
        if edge.rel == "contains" {
            out.entry(edge.to.as_str())
                .or_default()
                .push(edge.from.clone());
        }
    }
    out
}

fn surface_body(doc: &IndexDoc, surface: &DiscoveredSurface) -> String {
    [
        format!("id: {}", surface.id),
        format!("kind: {}", surface.kind),
        format!("subject: {}", surface.subject),
        format!(
            "subject_title: {}",
            subject_title(doc, surface.subject.as_str())
        ),
        "panel: already exists".to_owned(),
    ]
    .join("\n")
}

fn specimen_body(doc: &IndexDoc, specimen: &DiscoveredSpecimen) -> String {
    [
        format!("id: {}", specimen.id),
        format!("kind: {}", specimen.kind),
        format!("subject: {}", specimen.subject),
        format!(
            "subject_title: {}",
            subject_title(doc, specimen.subject.as_str())
        ),
        format!("path: {}", specimen.path),
        format!("language: {}", specimen.language.as_deref().unwrap_or("")),
        format!("runnable: {}", specimen.runnable),
        format!("checked: {}", specimen.checked),
        format!(
            "checked_by: {}",
            specimen.checked_by.as_deref().unwrap_or("")
        ),
        "panel: run this example".to_owned(),
    ]
    .join("\n")
}

fn feature_body(doc: &IndexDoc, feature: &FeatureRecord) -> String {
    let specimens = feature
        .specimens
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    [
        format!("id: {}", feature.id),
        format!("title: {}", feature.title),
        format!("summary: {}", feature.summary),
        format!("subject: {}", feature.subject),
        format!(
            "subject_title: {}",
            subject_title(doc, feature.subject.as_str())
        ),
        format!(
            "anchors: {}",
            feature
                .anchors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        ),
        format!(
            "surfaces: {}",
            feature
                .surfaces
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        ),
        format!("specimens: {}", specimens.join(" ")),
        format!(
            "grammars: {}",
            feature
                .grammar_contracts
                .iter()
                .map(|grammar| grammar.id.clone())
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "panel: already exists".to_owned(),
        if specimens.is_empty() {
            String::new()
        } else {
            "panel: run this example".to_owned()
        },
    ]
    .into_iter()
    .filter(|line| !line.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn grammar_body(feature: &FeatureRecord, grammar: &GrammarContract) -> String {
    [
        format!("id: {}", grammar.id),
        format!("feature: {}", feature.id),
        format!("feature_title: {}", feature.title),
        format!("decoder: {}", optional_id(grammar.decoder.as_ref())),
        format!("encoder: {}", optional_id(grammar.encoder.as_ref())),
        format!("surface: {}", optional_id(grammar.surface.as_ref())),
        format!("round_trip: {}", grammar.round_trip),
        "panel: already exists".to_owned(),
    ]
    .join("\n")
}

fn route_body(doc: &IndexDoc, route: &RouteRecord) -> String {
    let steps = route
        .steps
        .iter()
        .map(|step| format!("{} {} {}", step.kind(), step.id(), step_title(doc, step)))
        .collect::<Vec<_>>();
    let whys = route
        .steps
        .iter()
        .map(|step| step.why().to_owned())
        .collect::<Vec<_>>();
    [
        format!("id: {}", route.id),
        format!("title: {}", route.title),
        format!("audiences: {}", route.audiences.join(" ")),
        format!("steps: {}", steps.join(" | ")),
        format!("why: {}", whys.join(" | ")),
        "panel: reuse route".to_owned(),
        if route
            .steps
            .iter()
            .any(|step| matches!(step, RouteStep::Specimen { .. }))
        {
            "panel: run this example".to_owned()
        } else {
            String::new()
        },
    ]
    .into_iter()
    .filter(|line| !line.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn feature_has_runnable_specimen(doc: &IndexDoc, feature: &FeatureRecord) -> bool {
    feature.specimens.iter().any(|id| {
        doc.specimens
            .iter()
            .any(|specimen| specimen.id.as_str() == id.as_str() && specimen.runnable)
    })
}

fn feature_related_ids(feature: &FeatureRecord) -> Vec<String> {
    let mut ids = vec![feature.subject.to_string()];
    ids.extend(feature.anchors.iter().map(ToString::to_string));
    ids.extend(feature.surfaces.iter().map(ToString::to_string));
    ids.extend(feature.specimens.iter().map(ToString::to_string));
    ids.extend(
        feature
            .grammar_contracts
            .iter()
            .map(|grammar| grammar.id.clone()),
    );
    ids
}

fn grammar_related_ids(feature: &FeatureRecord, grammar: &GrammarContract) -> Vec<String> {
    let mut ids = vec![feature.id.to_string()];
    if let Some(decoder) = &grammar.decoder {
        ids.push(decoder.to_string());
    }
    if let Some(encoder) = &grammar.encoder {
        ids.push(encoder.to_string());
    }
    if let Some(surface) = &grammar.surface {
        ids.push(surface.to_string());
    }
    ids
}

fn optional_id<T: ToString>(value: Option<&T>) -> String {
    value.map(ToString::to_string).unwrap_or_default()
}

fn subject_title(doc: &IndexDoc, id: &str) -> String {
    doc.subjects
        .iter()
        .find(|subject| subject.id.as_str() == id)
        .map(|subject| subject.title.clone())
        .unwrap_or_else(|| id.to_owned())
}

fn step_title(doc: &IndexDoc, step: &RouteStep) -> String {
    match step {
        RouteStep::Feature { id, .. } => doc
            .features
            .iter()
            .find(|feature| feature.id.as_str() == id.as_str())
            .map(|feature| feature.title.clone())
            .unwrap_or_else(|| id.to_string()),
        RouteStep::Specimen { id, .. } => doc
            .specimens
            .iter()
            .find(|specimen| specimen.id.as_str() == id.as_str())
            .map(|specimen| specimen.path.clone())
            .unwrap_or_else(|| id.to_string()),
    }
}

fn display_path(repo: &RepoEntry, path: &Path) -> String {
    path.strip_prefix(&repo.checkout_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(MAIN_SEPARATOR, "/")
}
