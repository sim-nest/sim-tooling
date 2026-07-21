use std::cmp::Ordering;

use serde_json::Value;
use sim_cookbook::fnv1a64;

use super::{EMBED_DIM, RadarChunk, RadarHint, RadarQuery};

pub(super) fn rank_hints(
    chunks: &[RadarChunk],
    query: &RadarQuery,
) -> (Vec<RadarHint>, Vec<String>) {
    let matching = chunks
        .iter()
        .filter(|chunk| chunk.matches(query))
        .collect::<Vec<_>>();
    let stale = matching
        .iter()
        .filter(|chunk| !chunk.live)
        .map(|chunk| chunk.chunk_id.clone())
        .collect::<Vec<_>>();
    let query_embedding = embedding(&query.search_text());
    let mut scored = matching
        .into_iter()
        .filter(|chunk| chunk.live)
        .map(|chunk| {
            (
                cosine(&query_embedding, &embedding(&chunk.search_text()))
                    + graph_boost(chunk, query),
                chunk,
            )
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.1.chunk_id.cmp(&right.1.chunk_id))
    });
    let hints = scored
        .into_iter()
        .take(query.limit)
        .map(|(score, chunk)| RadarHint {
            chunk_id: chunk.chunk_id.clone(),
            title: chunk.title.clone(),
            repo: chunk.repo.clone(),
            path: chunk.path.clone(),
            line: chunk.line,
            capabilities: chunk.capabilities.clone(),
            preferred_codec: query
                .codec
                .clone()
                .or_else(|| chunk.codecs.first().cloned()),
            rust: chunk.rust.clone(),
            graph_id: chunk.graph_id.clone(),
            graph_kind: chunk.graph_kind.clone(),
            related_ids: chunk.related_ids.clone(),
            panels: chunk.panels.clone(),
            confidence: confidence_from_score(score),
        })
        .collect();
    (hints, stale)
}

impl RadarChunk {
    fn matches(&self, query: &RadarQuery) -> bool {
        matches_field(&self.repo, &query.repo)
            && matches_optional(self.crate_name.as_deref(), &query.crate_name)
            && matches_field(&self.kind, &query.kind)
            && matches_list(&self.capabilities, &query.capability)
            && matches_list(&self.codecs, &query.codec)
            && matches_list_or_text(&self.agent_roles, &self.text, &query.agent_role)
    }

    fn search_text(&self) -> String {
        let capabilities = self.capabilities.join(" ");
        let codecs = self.codecs.join(" ");
        let roles = self.agent_roles.join(" ");
        let rust_text = rust_search_text(&self.rust);
        let related_ids = self.related_ids.join(" ");
        let panels = self.panels.join(" ");
        [
            self.title.as_str(),
            self.kind.as_str(),
            self.crate_name.as_deref().unwrap_or_default(),
            self.text.as_str(),
            capabilities.as_str(),
            codecs.as_str(),
            roles.as_str(),
            rust_text.as_str(),
            self.graph_id.as_deref().unwrap_or_default(),
            self.graph_kind.as_deref().unwrap_or_default(),
            related_ids.as_str(),
            panels.as_str(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
    }
}

impl RadarQuery {
    fn search_text(&self) -> String {
        [
            self.text.as_str(),
            self.repo.as_deref().unwrap_or_default(),
            self.crate_name.as_deref().unwrap_or_default(),
            self.kind.as_deref().unwrap_or_default(),
            self.capability.as_deref().unwrap_or_default(),
            self.codec.as_deref().unwrap_or_default(),
            self.agent_role.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
    }
}

fn graph_boost(chunk: &RadarChunk, query: &RadarQuery) -> f32 {
    let mut boost = 0.0;
    let query_text = query.search_text().to_ascii_lowercase();
    let terms = search_terms(&query_text);
    let topical_terms = topical_terms(&terms);
    if let Some(graph_id) = &chunk.graph_id {
        let id = graph_id.to_ascii_lowercase();
        if query_text.contains(&id) || terms.iter().any(|term| term == &id) {
            boost += 2.0;
        }
        let id_matches = terms.iter().filter(|term| id.contains(*term)).count();
        boost += (id_matches as f32 * 0.4).min(1.2);
    }
    let text = chunk.text.to_ascii_lowercase();
    let topical_matches = topical_terms
        .iter()
        .filter(|term| text.contains(*term))
        .count();
    if let Some(kind) = &chunk.graph_kind {
        match kind.as_str() {
            "grammar" if terms.iter().any(|term| term == "grammar") && topical_matches > 0 => {
                boost += 1.6;
            }
            "grammar" => boost += 0.2,
            "route" if topical_matches > 0 => boost += 0.9,
            "route" => boost += 0.3,
            "feature" if topical_matches > 0 => boost += 0.8,
            "feature" => boost += 0.3,
            "specimen" if topical_matches > 0 => boost += 0.35,
            "specimen" => boost += 0.1,
            "package" => boost += 0.25,
            _ => {}
        }
    }
    let text_matches = terms.iter().filter(|term| text.contains(*term)).count();
    boost += (text_matches as f32 * 0.2).min(0.8);
    if chunk.panels.iter().any(|panel| panel == "run-this-example") {
        boost += 0.15;
    }
    if chunk.panels.iter().any(|panel| panel == "reuse-route") {
        boost += 0.4;
    }
    if terms.iter().any(|term| term == "framework")
        && chunk
            .text
            .to_ascii_lowercase()
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|term| term == "framework")
    {
        boost += 0.4;
    }
    if chunk.related_ids.iter().any(|id| {
        terms
            .iter()
            .any(|term| id.to_ascii_lowercase().contains(term))
    }) {
        boost += 0.3;
    }
    boost
}

fn search_terms(text: &str) -> Vec<String> {
    let mut terms = text
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '/' && ch != '-')
        .filter(|term| term.len() > 1)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let singulars = terms
        .iter()
        .filter_map(|term| term.strip_suffix('s').filter(|stem| stem.len() > 2))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    terms.extend(singulars);
    terms
}

fn topical_terms(terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .filter(|term| !GRAPH_KIND_WORDS.contains(&term.as_str()))
        .cloned()
        .collect()
}

const GRAPH_KIND_WORDS: &[&str] = &[
    "a",
    "an",
    "and",
    "code",
    "example",
    "feature",
    "for",
    "framework",
    "grammar",
    "language",
    "package",
    "route",
    "run",
    "specimen",
    "surface",
    "the",
    "this",
    "to",
    "user",
];

fn embedding(text: &str) -> [f32; EMBED_DIM] {
    let mut vector = [0.0; EMBED_DIM];
    let mut saw_token = false;
    for token in text
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
    {
        saw_token = true;
        vector[stable_hash(&token.to_ascii_lowercase()) % EMBED_DIM] += 1.0;
    }
    if !saw_token {
        vector[stable_hash("atelier") % EMBED_DIM] = 1.0;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn stable_hash(text: &str) -> usize {
    fnv1a64(text.as_bytes()) as usize
}

fn cosine(left: &[f32; EMBED_DIM], right: &[f32; EMBED_DIM]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn confidence_from_score(score: f32) -> f64 {
    let likelihood = ((score as f64 + 1.0) / 2.0).clamp(0.0, 1.0);
    let evidence = 0.5 * likelihood + 0.5 * 0.25;
    let posterior = if evidence == 0.0 {
        0.0
    } else {
        (0.5 * likelihood / evidence).clamp(0.0, 1.0)
    };
    let uncertainty = entropy_binary(posterior);
    (posterior * 0.85 + (1.0 - uncertainty) * 0.15).clamp(0.0, 1.0)
}

fn entropy_binary(probability: f64) -> f64 {
    [probability, 1.0 - probability]
        .into_iter()
        .filter(|value| *value > 0.0)
        .map(|value| -value * value.log2())
        .sum()
}

fn matches_field(value: &str, filter: &Option<String>) -> bool {
    filter
        .as_deref()
        .is_none_or(|filter| value.eq_ignore_ascii_case(filter))
}

fn matches_optional(value: Option<&str>, filter: &Option<String>) -> bool {
    filter
        .as_deref()
        .is_none_or(|filter| value.is_some_and(|value| value.eq_ignore_ascii_case(filter)))
}

fn matches_list(values: &[String], filter: &Option<String>) -> bool {
    filter.as_deref().is_none_or(|filter| {
        values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(filter))
    })
}

fn matches_list_or_text(values: &[String], text: &str, filter: &Option<String>) -> bool {
    filter.as_deref().is_none_or(|filter| {
        values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(filter))
            || text
                .to_ascii_lowercase()
                .contains(&filter.to_ascii_lowercase())
    })
}

fn rust_search_text(rust: &Option<Value>) -> String {
    let Some(rust) = rust else {
        return String::new();
    };
    let mut parts = Vec::new();
    for pointer in ["/ide_object_id", "/module", "/item_kind", "/item_name"] {
        if let Some(text) = rust.pointer(pointer).and_then(Value::as_str) {
            parts.push(text.to_owned());
        }
    }
    parts.extend(string_array(&rust["feature_gates"]));
    parts.extend(string_array(&rust["crate_features"]));
    parts.join(" ")
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
