use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use sim_cookbook::fnv1a64;

use super::{
    index::{AtelierIndexOptions, atelier_index},
    io::display_io,
};

const DEFAULT_INDEX_FILE: &str = ".sim/atelier/index/index.json";
const DEFAULT_LIMIT: usize = 8;
const EMBED_DIM: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AtelierRadarOptions {
    pub(super) control_root: PathBuf,
    pub(super) index_file: Option<PathBuf>,
    pub(super) query: RadarQuery,
    pub(super) json: bool,
}

impl Default for AtelierRadarOptions {
    fn default() -> Self {
        Self {
            control_root: PathBuf::from("."),
            index_file: None,
            query: RadarQuery::default(),
            json: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RadarQuery {
    pub(super) text: String,
    pub(super) repo: Option<String>,
    pub(super) crate_name: Option<String>,
    pub(super) kind: Option<String>,
    pub(super) capability: Option<String>,
    pub(super) codec: Option<String>,
    pub(super) agent_role: Option<String>,
    pub(super) limit: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RadarReport {
    pub(super) hints: Vec<RadarHint>,
    pub(super) stale_index: bool,
    pub(super) stale_chunk_ids: Vec<String>,
    pub(super) index_file: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RadarHint {
    pub(super) chunk_id: String,
    pub(super) title: String,
    pub(super) repo: String,
    pub(super) path: String,
    pub(super) line: usize,
    pub(super) capabilities: Vec<String>,
    pub(super) preferred_codec: Option<String>,
    pub(super) rust: Option<Value>,
    pub(super) confidence: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RadarChunk {
    chunk_id: String,
    title: String,
    repo: String,
    path: String,
    line: usize,
    crate_name: Option<String>,
    kind: String,
    text: String,
    capabilities: Vec<String>,
    codecs: Vec<String>,
    agent_roles: Vec<String>,
    rust: Option<Value>,
    live: bool,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierRadarOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierRadarOptions::default()
    };
    options.query.limit = DEFAULT_LIMIT;

    let mut args = args.into_iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control-root" => options.control_root = next_path(&mut args, "--control-root")?,
            "--index" => options.index_file = Some(next_path(&mut args, "--index")?),
            "--repo" => options.query.repo = Some(next_string(&mut args, "--repo")?),
            "--crate" => options.query.crate_name = Some(next_string(&mut args, "--crate")?),
            "--kind" => options.query.kind = Some(next_string(&mut args, "--kind")?),
            "--capability" => {
                options.query.capability = Some(next_string(&mut args, "--capability")?);
            }
            "--codec" => options.query.codec = Some(next_string(&mut args, "--codec")?),
            "--agent-role" => {
                options.query.agent_role = Some(next_string(&mut args, "--agent-role")?);
            }
            "--limit" => options.query.limit = next_usize(&mut args, "--limit")?,
            "--json" => options.json = true,
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown atelier-radar option: {other}"));
            }
            text if options.query.text.is_empty() => options.query.text = text.to_owned(),
            extra => return Err(format!("unexpected atelier-radar argument: {extra}")),
        }
    }
    if options.query.text.is_empty() {
        return Err("atelier-radar requires a query string".to_owned());
    }

    let report = atelier_radar(options.clone())?;
    if options.json {
        println!("{}", pretty_json(&report_json(&report))?);
    } else {
        print_text_report(&report);
    }
    Ok(())
}

pub(super) fn atelier_radar(options: AtelierRadarOptions) -> Result<RadarReport, String> {
    let index_file = options
        .index_file
        .clone()
        .unwrap_or_else(|| options.control_root.join(DEFAULT_INDEX_FILE));
    ensure_index(&options.control_root, &index_file)?;
    let value: Value = serde_json::from_str(&fs::read_to_string(&index_file).map_err(display_io)?)
        .map_err(|err| format!("parse {}: {err}", index_file.display()))?;
    let repo_roots = repo_roots(&options.control_root, &value)?;
    let chunks = parse_chunks(&value, &options.control_root, &repo_roots)?;
    let (hints, stale_chunk_ids) = rank_hints(&chunks, &options.query);
    Ok(RadarReport {
        hints,
        stale_index: !stale_chunk_ids.is_empty(),
        stale_chunk_ids,
        index_file,
    })
}

fn ensure_index(control_root: &Path, index_file: &Path) -> Result<(), String> {
    if index_file.is_file() {
        return Ok(());
    }
    let cache_dir = index_file
        .parent()
        .ok_or_else(|| format!("index path has no parent: {}", index_file.display()))?;
    atelier_index(AtelierIndexOptions {
        control_root: control_root.to_path_buf(),
        repos_manifest: Some(control_root.join("repos.toml")),
        cache_dir: Some(cache_dir.to_path_buf()),
        check: false,
        ..AtelierIndexOptions::default()
    })?;
    Ok(())
}

fn repo_roots(control_root: &Path, index: &Value) -> Result<BTreeMap<String, PathBuf>, String> {
    let repos = index["repos"]
        .as_array()
        .ok_or_else(|| "atelier index has no repos array".to_owned())?;
    let mut roots = BTreeMap::new();
    for repo in repos {
        let Some(name) = repo["name"].as_str() else {
            continue;
        };
        let Some(local_path) = repo["local_path"].as_str() else {
            continue;
        };
        roots.insert(name.to_owned(), control_root.join(local_path));
    }
    Ok(roots)
}

fn parse_chunks(
    index: &Value,
    control_root: &Path,
    repo_roots: &BTreeMap<String, PathBuf>,
) -> Result<Vec<RadarChunk>, String> {
    let chunks = index["chunks"]
        .as_array()
        .ok_or_else(|| "atelier index has no chunks array".to_owned())?;
    let mut parsed = Vec::new();
    for chunk in chunks {
        let Some(chunk_id) = chunk["id"].as_str() else {
            continue;
        };
        let Some(repo) = chunk["repo"].as_str() else {
            continue;
        };
        let Some(path) = chunk.pointer("/span/file").and_then(Value::as_str) else {
            continue;
        };
        let line = chunk
            .pointer("/span/line")
            .and_then(Value::as_u64)
            .unwrap_or(1) as usize;
        let text = chunk["text"].as_str().unwrap_or_default().to_owned();
        parsed.push(RadarChunk {
            chunk_id: chunk_id.to_owned(),
            title: chunk_title(chunk, &text),
            repo: repo.to_owned(),
            path: path.to_owned(),
            line,
            crate_name: chunk["crate"].as_str().map(str::to_owned),
            kind: chunk["kind"].as_str().unwrap_or("chunk").to_owned(),
            text: text.clone(),
            capabilities: string_array(&chunk["capabilities"]),
            codecs: string_array(&chunk["codecs"]),
            agent_roles: infer_agent_roles(&text),
            rust: chunk.get("rust").cloned(),
            live: live_span(control_root, repo_roots, repo, path, line),
        });
    }
    Ok(parsed)
}

fn rank_hints(chunks: &[RadarChunk], query: &RadarQuery) -> (Vec<RadarHint>, Vec<String>) {
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
                cosine(&query_embedding, &embedding(&chunk.search_text())),
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
        [
            self.title.as_str(),
            self.kind.as_str(),
            self.crate_name.as_deref().unwrap_or_default(),
            self.text.as_str(),
            capabilities.as_str(),
            codecs.as_str(),
            roles.as_str(),
            rust_text.as_str(),
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

fn live_span(
    control_root: &Path,
    repo_roots: &BTreeMap<String, PathBuf>,
    repo: &str,
    path: &str,
    line: usize,
) -> bool {
    if line == 0 {
        return false;
    }
    if path == "repos.toml" {
        return control_root.join("repos.toml").is_file();
    }
    let Some(root) = repo_roots.get(repo) else {
        return false;
    };
    root.join(path).is_file()
}

fn chunk_title(chunk: &Value, text: &str) -> String {
    if let Some(title) = chunk["heading_path"]
        .as_array()
        .and_then(|items| items.iter().rev().find_map(Value::as_str))
        .filter(|title| !title.trim().is_empty())
    {
        return truncate(title.trim(), 96);
    }
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("chunk");
    truncate(line.trim(), 96)
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    text.chars().take(max.saturating_sub(3)).collect::<String>() + "..."
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

fn infer_agent_roles(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    [
        "agent",
        "planner",
        "retriever",
        "validator",
        "guard",
        "editor",
        "docs",
        "pin",
    ]
    .into_iter()
    .filter(|role| lower.contains(role))
    .map(str::to_owned)
    .collect()
}

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

fn report_json(report: &RadarReport) -> Value {
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
        "capabilities": hint.capabilities,
        "preferred_codec": hint.preferred_codec,
        "confidence": hint.confidence,
    });
    if let Some(rust) = &hint.rust {
        value["rust"] = rust.clone();
    }
    value
}

fn print_text_report(report: &RadarReport) {
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

fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier radar json: {err}"))
}

fn print_usage() {
    println!(
        "usage: xtask atelier-radar <query> [--repo NAME] [--crate NAME] [--kind KIND] [--capability NAME] [--codec NAME] [--agent-role NAME] [--limit N] [--control-root PATH] [--index PATH] [--json]"
    );
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    next_string(args, flag).map(PathBuf::from)
}

fn next_usize(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<usize, String> {
    let value = next_string(args, flag)?;
    value
        .parse::<usize>()
        .map_err(|err| format!("{flag} requires a positive integer: {err}"))
}

fn next_string(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}
