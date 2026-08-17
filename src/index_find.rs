//! Search over generated SIM Index rows.

use std::path::PathBuf;

use serde_json::{Value, json};
use sim_index_core::{DeclarationRole, FeatureRecord, IndexDoc, RouteRecord, RouteStep};

use crate::index_render::load_doc;

mod facts;
use facts::{protocol_relation_json, protocol_relation_summary, selected_fact_anchors};

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = FindOptions::parse(&args)?;
    let doc = load_doc(&options.input)?;
    let matches = find_rows(&doc, &options);
    if options.json {
        let text = serde_json::to_string_pretty(&json!({
            "query": options.query,
            "audience": options.audience,
            "surface": options.surface,
            "declaration_kind": options.declaration_kind.map(DeclarationRole::as_str),
            "implements": options.implements,
            "resolved": options.resolution.map(ResolutionFilter::as_str),
            "feature": options.feature,
            "match_count": matches.len(),
            "matches": matches,
        }))
        .map_err(|err| format!("serialize search results: {err}"))?;
        println!("{text}");
    } else {
        for row in matches {
            println!("{}\t{}\t{}", row["kind"], row["id"], row["title"]);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FindOptions {
    input: PathBuf,
    query: String,
    json: bool,
    audience: Option<String>,
    surface: Option<String>,
    declaration_kind: Option<DeclarationRole>,
    implements: Option<String>,
    resolution: Option<ResolutionFilter>,
    feature: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolutionFilter {
    Resolved,
    Unresolved,
}

impl ResolutionFilter {
    fn as_str(self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::Unresolved => "unresolved",
        }
    }
}

impl FindOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if !matches!(args.get(1).map(String::as_str), Some("index"))
            || !matches!(args.get(2).map(String::as_str), Some("find"))
        {
            return Err(find_usage(program));
        }
        let mut input = None;
        let mut json = false;
        let mut audience = None;
        let mut surface = None;
        let mut declaration_kind = None;
        let mut implements = None;
        let mut resolution = None;
        let mut feature = None;
        let mut query = Vec::new();
        let mut index = 3;
        while index < args.len() {
            match args[index].as_str() {
                "--input" => {
                    index += 1;
                    input = Some(PathBuf::from(
                        args.get(index).ok_or("--input requires a path")?,
                    ));
                }
                "--audience" => {
                    index += 1;
                    let value = args.get(index).ok_or("--audience requires a value")?;
                    if value.trim().is_empty() {
                        return Err("--audience requires a non-empty value".to_owned());
                    }
                    audience = Some(value.to_owned());
                }
                "--surface" => {
                    index += 1;
                    let value = args.get(index).ok_or("--surface requires a value")?;
                    if value.trim().is_empty() {
                        return Err("--surface requires a non-empty value".to_owned());
                    }
                    surface = Some(value.to_owned());
                }
                "--declaration-kind" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or("--declaration-kind requires a value")?;
                    declaration_kind = Some(parse_declaration_role(value)?);
                }
                "--implements" => {
                    index += 1;
                    implements = Some(non_empty_option(args.get(index), "--implements")?);
                }
                "--resolved" => {
                    if resolution.replace(ResolutionFilter::Resolved).is_some() {
                        return Err("choose only one of --resolved and --unresolved".to_owned());
                    }
                }
                "--unresolved" => {
                    if resolution.replace(ResolutionFilter::Unresolved).is_some() {
                        return Err("choose only one of --resolved and --unresolved".to_owned());
                    }
                }
                "--feature" => {
                    index += 1;
                    feature = Some(non_empty_option(args.get(index), "--feature")?);
                }
                "--query" => {
                    index += 1;
                    query.push(args.get(index).ok_or("--query requires text")?.to_owned());
                }
                "--json" => json = true,
                "-h" | "--help" => return Err(find_usage(program)),
                other if other.starts_with('-') => {
                    return Err(format!(
                        "unknown index find argument `{other}`; {}",
                        find_usage(program)
                    ));
                }
                other => query.push(other.to_owned()),
            }
            index += 1;
        }
        let query = query.join(" ");
        if query.trim().is_empty()
            && declaration_kind.is_none()
            && implements.is_none()
            && resolution.is_none()
            && feature.is_none()
        {
            return Err(format!(
                "index find requires a query; {}",
                find_usage(program)
            ));
        }
        Ok(Self {
            input: input
                .ok_or_else(|| format!("index find requires --input; {}", find_usage(program)))?,
            query,
            json,
            audience,
            surface,
            declaration_kind,
            implements,
            resolution,
            feature,
        })
    }
}

fn non_empty_option(value: Option<&String>, option: &str) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("{option} requires a value"))?;
    if value.trim().is_empty() {
        return Err(format!("{option} requires a non-empty value"));
    }
    Ok(value.to_owned())
}

fn parse_declaration_role(value: &str) -> Result<DeclarationRole, String> {
    match value {
        "const" => Ok(DeclarationRole::Const),
        "enum" => Ok(DeclarationRole::Enum),
        "function" => Ok(DeclarationRole::Function),
        "module" => Ok(DeclarationRole::Module),
        "re-export" => Ok(DeclarationRole::ReExport),
        "static" => Ok(DeclarationRole::Static),
        "struct" => Ok(DeclarationRole::Struct),
        "trait" => Ok(DeclarationRole::Trait),
        "type-alias" => Ok(DeclarationRole::TypeAlias),
        _ => Err(format!(
            "unknown declaration kind `{value}`; expected const, enum, function, module, re-export, static, struct, trait, or type-alias"
        )),
    }
}

fn find_usage(program: &str) -> String {
    format!(
        "usage: {program} index find --input <index.sx> [--json] [--audience <name>] [--surface <kind-or-id>] [--declaration-kind <kind>] [--implements <protocol>] [--resolved|--unresolved] [--feature <id-or-key>] [<query>]"
    )
}

#[cfg(test)]
pub(crate) fn find_rows_filtered(
    doc: &IndexDoc,
    query: &str,
    audience: Option<&str>,
    surface: Option<&str>,
) -> Vec<Value> {
    let options = FindOptions {
        input: PathBuf::new(),
        query: query.to_owned(),
        json: false,
        audience: audience.map(str::to_owned),
        surface: surface.map(str::to_owned),
        declaration_kind: None,
        implements: None,
        resolution: None,
        feature: None,
    };
    find_rows(doc, &options)
}

fn find_rows(doc: &IndexDoc, options: &FindOptions) -> Vec<Value> {
    let query = &options.query;
    let audience = options.audience.as_deref();
    let surface = options.surface.as_deref();
    let needle = query.to_ascii_lowercase();
    let mut rows = Vec::new();
    let fact_filter_active = options.declaration_kind.is_some()
        || options.implements.is_some()
        || options.resolution.is_some()
        || options.feature.is_some();
    let selected_anchors = selected_fact_anchors(doc, options, &needle);
    if audience.is_none() {
        for subject in &doc.subjects {
            if !subject_matches_surface(doc, subject.id.as_str(), surface) {
                continue;
            }
            if (!fact_filter_active
                && matches_query(
                    &needle,
                    &[subject.id.as_str(), &subject.kind, &subject.title],
                ))
                || selected_anchors.iter().any(|id| {
                    doc.anchors.iter().any(|anchor| {
                        &anchor.id == id && anchor.subject.as_str() == subject.id.as_str()
                    })
                })
            {
                rows.push(json!({
                    "kind": if subject.kind == "crate" { "package" } else { "subject" },
                    "id": subject.id.as_str(),
                    "title": subject.title,
                    "summary": subject.kind,
                    "subject_kind": subject.kind,
                }));
            }
        }
        for anchor in &doc.anchors {
            if !selected_anchors.contains(&anchor.id) {
                continue;
            }
            let declarations = doc
                .declarations
                .iter()
                .filter(|fact| fact.anchor == anchor.id)
                .map(|fact| json!({
                    "kind": fact.role.as_str(),
                    "module_path": fact.module_path,
                    "location": format!("{}#declaration-{}", fact.location.file, fact.location.declaration),
                }))
                .collect::<Vec<_>>();
            let implementations = doc
                .protocol_relations
                .iter()
                .filter(|relation| relation.anchor == anchor.id)
                .map(protocol_relation_json)
                .collect::<Vec<_>>();
            let title = doc
                .protocol_relations
                .iter()
                .filter(|relation| relation.anchor == anchor.id)
                .map(protocol_relation_summary)
                .chain(
                    doc.declarations
                        .iter()
                        .filter(|fact| fact.anchor == anchor.id)
                        .map(|fact| {
                            format!("{} declaration {}", fact.role.as_str(), fact.module_path)
                        }),
                )
                .collect::<Vec<_>>()
                .join("; ");
            rows.push(json!({
                "kind": "anchor",
                "id": anchor.id.as_str(),
                "title": title,
                "summary": anchor.kind,
                "owner": anchor.subject.as_str(),
                "declarations": declarations,
                "protocol_relations": implementations,
            }));
        }
        for record in &doc.surfaces {
            if !surface_matches_filter(record.id.as_str(), &record.kind, surface) {
                continue;
            }
            if matches_query(
                &needle,
                &[record.id.as_str(), record.subject.as_str(), &record.kind],
            ) {
                rows.push(json!({
                    "kind": "surface",
                    "id": record.id.as_str(),
                    "title": record.id.as_str(),
                    "summary": record.kind,
                }));
            }
        }
    }
    for feature in &doc.features {
        if !feature_matches_audience(doc, feature.id.as_str(), audience) {
            continue;
        }
        if !feature_matches_surface(doc, feature, surface) {
            continue;
        }
        let surface_text = feature
            .surfaces
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let owns_selected_anchor = feature
            .anchors
            .iter()
            .any(|id| selected_anchors.contains(id));
        if (!fact_filter_active
            && matches_query(
                &needle,
                &[
                    feature.id.as_str(),
                    feature.key.as_str(),
                    feature.subject.as_str(),
                    &feature.title,
                    &feature.summary,
                    &surface_text,
                ],
            ))
            || owns_selected_anchor
        {
            rows.push(json!({
                "kind": "feature",
                "id": feature.id.as_str(),
                "title": feature.title,
                "summary": feature.summary,
                "owner": feature.subject.as_str(),
                "surfaces": feature.surfaces.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "specimens": feature.specimens.iter().map(ToString::to_string).collect::<Vec<_>>(),
            }));
        }
    }
    for specimen in &doc.specimens {
        if !specimen_matches_audience(doc, specimen.id.as_str(), audience) {
            continue;
        }
        if !specimen_matches_surface(doc, specimen.id.as_str(), surface) {
            continue;
        }
        let linked_to_selected = doc.features.iter().any(|feature| {
            feature
                .specimens
                .iter()
                .any(|id| id.as_str() == specimen.id.as_str())
                && feature
                    .anchors
                    .iter()
                    .any(|id| selected_anchors.contains(id))
        });
        if (!fact_filter_active
            && (matches_query(
                &needle,
                &[
                    specimen.id.as_str(),
                    specimen.subject.as_str(),
                    &specimen.kind,
                    &specimen.path,
                ],
            ) || specimen_linked_feature_matches_query(
                doc,
                specimen.id.as_str(),
                &needle,
                surface,
            )))
            || linked_to_selected
        {
            rows.push(json!({
                "kind": "specimen",
                "id": specimen.id.as_str(),
                "title": specimen.path,
                "summary": specimen.kind,
            }));
        }
    }
    for route in &doc.routes {
        if !route_matches_audience(route.audiences.iter().map(String::as_str), audience) {
            continue;
        }
        if !route_matches_surface(doc, route, surface) {
            continue;
        }
        let route_text = route
            .steps
            .iter()
            .flat_map(|step| [step.id(), step.why()])
            .collect::<Vec<_>>();
        let mut fields = vec![route.id.as_str(), &route.title];
        fields.extend(route.audiences.iter().map(String::as_str));
        fields.extend(route_text);
        if !fact_filter_active && matches_query(&needle, &fields) {
            rows.push(json!({
                "kind": "route",
                "id": route.id.as_str(),
                "title": route.title,
                "summary": route.steps.len().to_string(),
            }));
        }
    }
    rows.sort_by(|left, right| {
        (
            left["kind"].as_str().unwrap_or_default(),
            left["id"].as_str().unwrap_or_default(),
        )
            .cmp(&(
                right["kind"].as_str().unwrap_or_default(),
                right["id"].as_str().unwrap_or_default(),
            ))
    });
    rows
}

fn subject_matches_surface(doc: &IndexDoc, subject_id: &str, filter: Option<&str>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    if doc.surfaces.iter().any(|surface| {
        surface.subject.as_str() == subject_id
            && surface_matches_filter(surface.id.as_str(), &surface.kind, Some(filter))
    }) {
        return true;
    }
    let Some(grammar_tail) = subject_id.strip_prefix("grammar/") else {
        return false;
    };
    doc.surfaces.iter().any(|surface| {
        surface_matches_filter(surface.id.as_str(), &surface.kind, Some(filter))
            && surface.id.as_str().rsplit('/').next() == Some(grammar_tail)
    })
}

fn feature_matches_surface(doc: &IndexDoc, feature: &FeatureRecord, filter: Option<&str>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    feature.surfaces.iter().any(|id| {
        doc.surfaces
            .iter()
            .find(|surface| surface.id.as_str() == id.as_str())
            .map(|surface| surface_matches_filter(id.as_str(), &surface.kind, Some(filter)))
            .unwrap_or_else(|| surface_id_matches_filter(id.as_str(), filter))
    }) || feature.grammar_contracts.iter().any(|contract| {
        contract
            .surface
            .as_ref()
            .map(|id| surface_id_matches_filter(id.as_str(), filter))
            .unwrap_or(false)
    })
}

fn specimen_matches_surface(doc: &IndexDoc, specimen_id: &str, filter: Option<&str>) -> bool {
    filter.is_none()
        || doc.features.iter().any(|feature| {
            feature
                .specimens
                .iter()
                .any(|id| id.as_str() == specimen_id)
                && feature_matches_surface(doc, feature, filter)
        })
}

fn route_matches_surface(doc: &IndexDoc, route: &RouteRecord, filter: Option<&str>) -> bool {
    filter.is_none()
        || route.steps.iter().any(|step| match step {
            RouteStep::Feature { id, .. } => doc.features.iter().any(|feature| {
                feature.id.as_str() == id.as_str() && feature_matches_surface(doc, feature, filter)
            }),
            RouteStep::Specimen { id, .. } => specimen_matches_surface(doc, id.as_str(), filter),
        })
}

fn specimen_linked_feature_matches_query(
    doc: &IndexDoc,
    specimen_id: &str,
    needle: &str,
    surface: Option<&str>,
) -> bool {
    doc.features.iter().any(|feature| {
        feature
            .specimens
            .iter()
            .any(|id| id.as_str() == specimen_id)
            && feature_matches_surface(doc, feature, surface)
            && matches_query(
                needle,
                &[
                    feature.id.as_str(),
                    feature.key.as_str(),
                    feature.subject.as_str(),
                    &feature.title,
                    &feature.summary,
                ],
            )
    })
}

fn surface_matches_filter(id: &str, kind: &str, filter: Option<&str>) -> bool {
    filter
        .map(|filter| kind == filter || surface_id_matches_filter(id, filter))
        .unwrap_or(true)
}

fn surface_id_matches_filter(id: &str, filter: &str) -> bool {
    id == filter
        || id
            .strip_prefix(filter)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn feature_matches_audience(doc: &IndexDoc, feature_id: &str, audience: Option<&str>) -> bool {
    let Some(audience) = audience else {
        return true;
    };
    doc.routes.iter().any(|route| {
        route.audiences.iter().any(|item| item == audience)
            && route.steps.iter().any(|step| match step {
                RouteStep::Feature { id, .. } => id.as_str() == feature_id,
                RouteStep::Specimen { .. } => false,
            })
    })
}

fn specimen_matches_audience(doc: &IndexDoc, specimen_id: &str, audience: Option<&str>) -> bool {
    let Some(audience) = audience else {
        return true;
    };
    doc.routes.iter().any(|route| {
        route.audiences.iter().any(|item| item == audience)
            && route.steps.iter().any(|step| match step {
                RouteStep::Feature { .. } => false,
                RouteStep::Specimen { id, .. } => id.as_str() == specimen_id,
            })
    })
}

fn route_matches_audience<'a>(
    audiences: impl Iterator<Item = &'a str>,
    audience: Option<&str>,
) -> bool {
    match audience {
        Some(expected) => audiences.into_iter().any(|item| item == expected),
        None => true,
    }
}

fn matches_query(needle: &str, fields: &[&str]) -> bool {
    fields
        .iter()
        .any(|field| field.to_ascii_lowercase().contains(needle))
}

#[cfg(test)]
#[path = "index_find_tests.rs"]
mod tests;
