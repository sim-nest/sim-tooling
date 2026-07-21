use serde_json::{Value, json};

use super::{RadarHint, RadarReport};

pub(super) fn report_json(report: &RadarReport) -> Value {
    json!({
        "schema": "sim.atelier.radar-report.v1",
        "ranker": "sim-lib-rank/retrieve",
        "confidence": "sim-lib-numbers-stats/bayesian-update-binary+entropy",
        "index": report.index_file.to_string_lossy(),
        "stale_index": report.stale_index,
        "stale_chunk_ids": report.stale_chunk_ids,
        "hints": report.hints.iter().map(hint_json).collect::<Vec<_>>(),
    })
}

fn hint_json(hint: &RadarHint) -> Value {
    let mut value = json!({
        "chunk_id": hint.chunk_id,
        "title": hint.title,
        "span": {
            "repo": hint.repo,
            "file": hint.path,
            "line": hint.line,
        },
        "graph_id": hint.graph_id,
        "graph_kind": hint.graph_kind,
        "related_ids": hint.related_ids,
        "panels": hint.panels,
        "capabilities": hint.capabilities,
        "preferred_codec": hint.preferred_codec,
        "confidence": hint.confidence,
    });
    if let Some(rust) = &hint.rust {
        value["rust"] = rust.clone();
    }
    value
}

pub(super) fn print_text_report(report: &RadarReport) {
    println!(
        "atelier-radar: {} hint(s), stale_index={}",
        report.hints.len(),
        report.stale_index
    );
    for (index, hint) in report.hints.iter().enumerate() {
        println!(
            "{}. {:.3} {} {}:{}",
            index + 1,
            hint.confidence,
            hint.repo,
            hint.path,
            hint.line
        );
        println!("   {}", hint.title);
        println!("   chunk: {}", hint.chunk_id);
        if let Some(graph_id) = &hint.graph_id {
            println!("   graph: {graph_id}");
        }
        if !hint.panels.is_empty() {
            println!("   panels: {}", hint.panels.join(", "));
        }
        if !hint.capabilities.is_empty() {
            println!("   capabilities: {}", hint.capabilities.join(", "));
        }
        if let Some(codec) = &hint.preferred_codec {
            println!("   preferred_codec: {codec}");
        }
        if let Some(ide_object) = hint
            .rust
            .as_ref()
            .and_then(|rust| rust["ide_object_id"].as_str())
        {
            println!("   ide_object: {ide_object}");
        }
    }
    if report.stale_index {
        eprintln!(
            "atelier-radar: stale index spans dropped: {}",
            report.stale_chunk_ids.join(", ")
        );
    }
}

pub(super) fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier radar json: {err}"))
}
