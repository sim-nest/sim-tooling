//! Search over generated SIM Index rows.

use std::path::PathBuf;

use serde_json::{Value, json};
use sim_index_core::{IndexDoc, RouteStep};

use crate::index_render::load_doc;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = FindOptions::parse(&args)?;
    let doc = load_doc(&options.input)?;
    let matches = find_rows_filtered(&doc, &options.query, options.audience.as_deref());
    if options.json {
        let text = serde_json::to_string_pretty(&json!({
            "query": options.query,
            "audience": options.audience,
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
        if query.trim().is_empty() {
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
        })
    }
}

fn find_usage(program: &str) -> String {
    format!("usage: {program} index find --input <index.sx> [--json] [--audience <name>] <query>")
}

pub(crate) fn find_rows_filtered(
    doc: &IndexDoc,
    query: &str,
    audience: Option<&str>,
) -> Vec<Value> {
    let needle = query.to_ascii_lowercase();
    let mut rows = Vec::new();
    if audience.is_none() {
        for subject in &doc.subjects {
            if matches_query(
                &needle,
                &[subject.id.as_str(), &subject.kind, &subject.title],
            ) {
                rows.push(json!({
                    "kind": "subject",
                    "id": subject.id.as_str(),
                    "title": subject.title,
                    "summary": subject.kind,
                }));
            }
        }
        for surface in &doc.surfaces {
            if matches_query(
                &needle,
                &[surface.id.as_str(), surface.subject.as_str(), &surface.kind],
            ) {
                rows.push(json!({
                    "kind": "surface",
                    "id": surface.id.as_str(),
                    "title": surface.id.as_str(),
                    "summary": surface.kind,
                }));
            }
        }
    }
    for feature in &doc.features {
        if !feature_matches_audience(doc, feature.id.as_str(), audience) {
            continue;
        }
        if matches_query(
            &needle,
            &[
                feature.id.as_str(),
                feature.key.as_str(),
                feature.subject.as_str(),
                &feature.title,
                &feature.summary,
            ],
        ) {
            rows.push(json!({
                "kind": "feature",
                "id": feature.id.as_str(),
                "title": feature.title,
                "summary": feature.summary,
            }));
        }
    }
    for specimen in &doc.specimens {
        if !specimen_matches_audience(doc, specimen.id.as_str(), audience) {
            continue;
        }
        if matches_query(
            &needle,
            &[
                specimen.id.as_str(),
                specimen.subject.as_str(),
                &specimen.kind,
                &specimen.path,
            ],
        ) {
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
        let route_text = route
            .steps
            .iter()
            .flat_map(|step| [step.id(), step.why()])
            .collect::<Vec<_>>();
        let mut fields = vec![route.id.as_str(), &route.title];
        fields.extend(route.audiences.iter().map(String::as_str));
        fields.extend(route_text);
        if matches_query(&needle, &fields) {
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
