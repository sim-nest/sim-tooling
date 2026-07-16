use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::{
    index::{AtelierIndexOptions, atelier_index},
    io::display_io,
};

mod cli;
mod render;
mod score;

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
    cli::run(args)
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
    let (hints, stale_chunk_ids) = score::rank_hints(&chunks, &options.query);
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
