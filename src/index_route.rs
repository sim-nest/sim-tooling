//! Best-use route lookup over the merged SIM Index graph.

use std::path::PathBuf;

use serde_json::{Value, json};
use sim_index_core::{IndexDoc, RouteRecord, RouteStep};

use crate::index_render::load_doc;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = RouteOptions::parse(&args)?;
    let doc = load_doc(&options.input)?;
    let routes = route_rows(&doc, &options.task);
    if options.json {
        let text = serde_json::to_string_pretty(&json!({
            "task": options.task,
            "match_count": routes.len(),
            "routes": routes,
        }))
        .map_err(|err| format!("serialize route results: {err}"))?;
        println!("{text}");
    } else {
        for route in routes {
            println!(
                "{}\t{}\t{}",
                route["route"].as_str().unwrap_or_default(),
                route["title"].as_str().unwrap_or_default(),
                route["audiences"].as_array().map(Vec::len).unwrap_or(0)
            );
            for step in route["steps"].as_array().into_iter().flatten() {
                println!(
                    "  {}\t{}\t{}",
                    step["kind"].as_str().unwrap_or_default(),
                    step["id"].as_str().unwrap_or_default(),
                    step["title"].as_str().unwrap_or_default()
                );
                if let Some(why) = step["why"].as_str().filter(|why| !why.is_empty()) {
                    println!("    {why}");
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RouteOptions {
    input: PathBuf,
    task: String,
    json: bool,
}

impl RouteOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if !matches!(args.get(1).map(String::as_str), Some("index"))
            || !matches!(args.get(2).map(String::as_str), Some("route"))
        {
            return Err(route_usage(program));
        }
        let mut input = None;
        let mut json = false;
        let mut task = Vec::new();
        let mut index = 3;
        while index < args.len() {
            match args[index].as_str() {
                "--input" => {
                    index += 1;
                    input = Some(PathBuf::from(
                        args.get(index).ok_or("--input requires a path")?,
                    ));
                }
                "--task" => {
                    index += 1;
                    task.push(args.get(index).ok_or("--task requires text")?.to_owned());
                }
                "--json" => json = true,
                "-h" | "--help" => return Err(route_usage(program)),
                other if other.starts_with('-') => {
                    return Err(format!(
                        "unknown index route argument `{other}`; {}",
                        route_usage(program)
                    ));
                }
                other => task.push(other.to_owned()),
            }
            index += 1;
        }
        let task = task.join(" ");
        if task.trim().is_empty() {
            return Err(format!(
                "index route requires a task; {}",
                route_usage(program)
            ));
        }
        Ok(Self {
            input: input
                .ok_or_else(|| format!("index route requires --input; {}", route_usage(program)))?,
            task,
            json,
        })
    }
}

fn route_usage(program: &str) -> String {
    format!("usage: {program} index route --input <index.sx> [--json] <task>")
}

pub(crate) fn route_rows(doc: &IndexDoc, task: &str) -> Vec<Value> {
    let terms = terms(task);
    let mut rows = doc
        .routes
        .iter()
        .filter_map(|route| {
            let score = route_score(doc, route, &terms);
            (score > 0).then(|| {
                (
                    score,
                    json!({
                        "route": route.id.as_str(),
                        "title": route.title,
                        "audiences": route.audiences,
                        "score": score,
                        "steps": route.steps.iter().map(|step| step_row(doc, step)).collect::<Vec<_>>(),
                    }),
                )
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1["route"].as_str().cmp(&right.1["route"].as_str()))
    });
    rows.into_iter().map(|(_, row)| row).collect()
}

fn route_score(doc: &IndexDoc, route: &RouteRecord, terms: &[String]) -> usize {
    let haystack = route_text(doc, route);
    if terms.is_empty() {
        return 1;
    }
    let mut score = 0;
    for term in terms {
        if haystack.contains(term) {
            score += 10;
        }
        if route.id.as_str().to_ascii_lowercase().contains(term)
            || route.title.to_ascii_lowercase().contains(term)
        {
            score += 5;
        }
    }
    score
}

fn route_text(doc: &IndexDoc, route: &RouteRecord) -> String {
    let mut parts = vec![
        route.id.as_str().to_owned(),
        route.title.clone(),
        route.audiences.join(" "),
    ];
    for step in &route.steps {
        parts.push(step.id().to_owned());
        parts.push(step.why().to_owned());
        parts.push(step_title(doc, step));
    }
    parts.join(" ").to_ascii_lowercase()
}

fn step_row(doc: &IndexDoc, step: &RouteStep) -> Value {
    json!({
        "kind": step.kind(),
        "id": step.id(),
        "title": step_title(doc, step),
        "why": step.why(),
        "specimen": specimen_meta(doc, step),
    })
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

fn specimen_meta(doc: &IndexDoc, step: &RouteStep) -> Value {
    let RouteStep::Specimen { id, .. } = step else {
        return Value::Null;
    };
    doc.specimens
        .iter()
        .find(|specimen| specimen.id.as_str() == id.as_str())
        .map(|specimen| {
            json!({
                "path": specimen.path,
                "runnable": specimen.runnable,
                "checked": specimen.checked,
                "checked_by": specimen.checked_by,
            })
        })
        .unwrap_or(Value::Null)
}

fn terms(task: &str) -> Vec<String> {
    task.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|term| term.len() > 1 && !STOP_WORDS.contains(&term.as_str()))
        .collect()
}

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "for", "in", "of", "on", "or", "the", "to", "with",
];

#[cfg(test)]
mod tests {
    use sim_index_core::{
        FeatureId, FeatureRecord, IndexDoc, RouteId, RouteRecord, RouteStep, SubjectId,
        SubjectRecord, Visibility, key::CanonicalFeatureKey,
    };

    use super::*;

    #[test]
    fn routes_rank_by_task_terms() {
        let doc = IndexDoc {
            schema: "sim.index".to_owned(),
            generated_by: "test".to_owned(),
            visibility: Visibility::Public,
            subjects: vec![SubjectRecord {
                id: SubjectId::new("crate/demo"),
                kind: "crate".to_owned(),
                title: "demo".to_owned(),
            }],
            anchors: Vec::new(),
            surfaces: Vec::new(),
            specimens: Vec::new(),
            drafts: Vec::new(),
            features: vec![FeatureRecord {
                id: FeatureId::new("feature/demo/parser"),
                key: CanonicalFeatureKey::new("crate/demo/parser"),
                subject: SubjectId::new("crate/demo"),
                title: "Parser path".to_owned(),
                summary: "Parse operator languages.".to_owned(),
                anchors: Vec::new(),
                surfaces: Vec::new(),
                specimens: Vec::new(),
                grammar_contracts: Vec::new(),
                doc_anchor: None,
            }],
            routes: vec![RouteRecord {
                id: RouteId::new("route/demo/parser"),
                title: "Write a parser".to_owned(),
                audiences: vec!["code".to_owned()],
                steps: vec![RouteStep::Feature {
                    id: FeatureId::new("feature/demo/parser"),
                    why: "This feature explains parser assembly.".to_owned(),
                }],
                doc_anchor: None,
            }],
            edges: Vec::new(),
        };

        let rows = route_rows(&doc, "write a parser");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["route"], "route/demo/parser");
        assert_eq!(
            rows[0]["steps"][0]["why"],
            "This feature explains parser assembly."
        );
    }

    #[test]
    fn task_terms_drop_common_words() {
        assert_eq!(
            terms("write a parser with docs"),
            ["write", "parser", "docs"]
        );
    }
}
