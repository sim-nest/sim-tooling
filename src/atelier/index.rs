use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sim_cookbook::fnv1a64_hex;

use super::{
    index_doc::{f1_recursive_chunks, line_for_offset},
    index_manifest::{RepoEntry, read_repos_manifest},
    index_units::{SourceUnit, collect_units},
    io::{check_cache, display_io, write_cache},
    rust::{RustIntelligence, build_rust_intelligence},
};

const DEFAULT_CACHE_DIR: &str = ".sim/atelier/index";
const INDEX_FILE: &str = "index.json";
const SCHEMA: &str = "sim.atelier.constellation-index.v1";
const DEFAULT_MAX_CHUNK_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AtelierIndexOptions {
    pub(super) control_root: PathBuf,
    pub(super) repos_manifest: Option<PathBuf>,
    pub(super) cache_dir: Option<PathBuf>,
    pub(super) check: bool,
    pub(super) max_chunk_bytes: usize,
}

impl Default for AtelierIndexOptions {
    fn default() -> Self {
        Self {
            control_root: PathBuf::from("."),
            repos_manifest: None,
            cache_dir: None,
            check: false,
            max_chunk_bytes: DEFAULT_MAX_CHUNK_BYTES,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AtelierIndexReport {
    pub(super) index: Value,
    pub(super) cache_file: PathBuf,
    pub(super) cache_changed: bool,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierIndexOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierIndexOptions::default()
    };
    let mut print = true;
    let mut args = args.into_iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control-root" => {
                options.control_root = next_path(&mut args, "--control-root")?;
            }
            "--repos-manifest" => {
                options.repos_manifest = Some(next_path(&mut args, "--repos-manifest")?);
            }
            "--cache" => {
                options.cache_dir = Some(next_path(&mut args, "--cache")?);
            }
            "--max-chunk-bytes" => {
                options.max_chunk_bytes = next_usize(&mut args, "--max-chunk-bytes")?;
            }
            "--check" => {
                options.check = true;
                print = false;
            }
            "--refresh-only" => {
                print = false;
            }
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown atelier-index option: {other}")),
        }
    }

    let report = atelier_index(options)?;
    if print {
        print!("{}", pretty_json(&report.index)?);
    }
    let status = if report.cache_changed {
        "updated"
    } else {
        "current"
    };
    eprintln!(
        "atelier-index: cache {status}: {}",
        report.cache_file.display()
    );
    Ok(())
}

pub(super) fn atelier_index(options: AtelierIndexOptions) -> Result<AtelierIndexReport, String> {
    let manifest_path = options
        .repos_manifest
        .clone()
        .unwrap_or_else(|| options.control_root.join("repos.toml"));
    let repos = read_repos_manifest(&options.control_root, &manifest_path)?;
    let mut diagnostics = repo_diagnostics(&repos);
    let mut units = Vec::new();
    for repo in &repos {
        match collect_units(repo) {
            Ok(repo_units) => units.extend(repo_units),
            Err(err) => diagnostics.push(diagnostic("unit-collection-failed", &repo.name, err)),
        }
    }
    units.sort_by(|left, right| left.id.cmp(&right.id));
    let rust = build_rust_intelligence(&repos, &units);
    diagnostics.extend(rust.diagnostics().iter().cloned());
    let chunks = chunks_for_units(&repos, &units, &rust, options.max_chunk_bytes);
    let chunks_include_meta_workspace = chunks.iter().any(chunk_references_meta_workspace);
    if chunks_include_meta_workspace {
        diagnostics.push(diagnostic(
            "meta-workspace-chunk",
            "index",
            "chunk references .meta-workspace",
        ));
    }

    let index = json!({
        "schema": SCHEMA,
        "chunker": {
            "codec": "sim-codec-doc",
            "operation": "doc/chunk-recursive",
            "max_bytes": options.max_chunk_bytes
        },
        "source_policy": {
            "repos_manifest": manifest_display(&options.control_root, &manifest_path),
            "generated_roots": [".meta-workspace/"],
            "chunks_include_meta_workspace": chunks_include_meta_workspace
        },
        "repos": repos.iter().map(repo_json).collect::<Vec<_>>(),
        "units": units.iter().map(unit_json).collect::<Vec<_>>(),
        "rust": rust.index_json(),
        "chunks": chunks,
        "diagnostics": diagnostics,
    });
    let cache_file = cache_file(&options);
    let content = pretty_json(&index)?;
    let cache_changed = if options.check {
        check_cache(&cache_file, &content, "xtask atelier-index")?
    } else {
        write_cache(&cache_file, &content)?
    };
    Ok(AtelierIndexReport {
        index,
        cache_file,
        cache_changed,
    })
}

fn chunks_for_units(
    repos: &[RepoEntry],
    units: &[SourceUnit],
    rust: &RustIntelligence,
    max_chunk_bytes: usize,
) -> Vec<Value> {
    let mut chunks = Vec::new();
    for unit in units {
        let pin = repos
            .iter()
            .find(|repo| repo.name == unit.repo)
            .map(|repo| repo.pin.as_str())
            .unwrap_or("");
        for (index, chunk) in f1_recursive_chunks(&unit.text, max_chunk_bytes)
            .into_iter()
            .filter(|chunk| !chunk.text.is_empty())
            .enumerate()
        {
            let mut value = json!({
                "id": format!("{}#{}", unit.id, index),
                "repo": unit.repo,
                "crate": unit.crate_name,
                "kind": unit.kind,
                "span": {
                    "file": unit.path,
                    "line": line_for_offset(&unit.text, chunk.start, unit.line),
                },
                "text": chunk.text,
                "heading_path": chunk.heading_path,
                "source_unit": unit.id,
                "capabilities": infer_capabilities(&unit.text),
                "codecs": infer_codecs(&unit.text),
                "pin": pin,
                "chunker": "sim-codec-doc/doc/chunk-recursive",
            });
            if let Some(fact) = rust.fact_for_unit(&unit.id) {
                value["rust"] = fact.json();
            }
            chunks.push(value);
        }
    }
    chunks
}

fn repo_diagnostics(repos: &[RepoEntry]) -> Vec<Value> {
    repos
        .iter()
        .filter_map(|repo| {
            repo.status.diagnostic_kind().map(|kind| {
                diagnostic(
                    kind,
                    &repo.name,
                    format!("{} status is {}", repo.local_path, repo.status.as_str()),
                )
            })
        })
        .collect()
}

fn repo_json(repo: &RepoEntry) -> Value {
    json!({
        "name": repo.name,
        "kind": repo.kind,
        "local_path": repo.local_path,
        "contains_code": repo.contains_code,
        "crates": repo.crate_names,
        "source_paths": repo.source_paths,
        "validation_command": repo.validation_command,
        "docs_command": repo.docs_command,
        "pin": repo.pin,
        "status": repo.status.as_str(),
    })
}

fn unit_json(unit: &SourceUnit) -> Value {
    json!({
        "id": unit.id,
        "repo": unit.repo,
        "crate": unit.crate_name,
        "kind": unit.kind,
        "path": unit.path,
        "line": unit.line,
        "text_hash": content_hash(&unit.text),
    })
}

fn diagnostic(
    kind: impl Into<String>,
    repo: impl Into<String>,
    message: impl Into<String>,
) -> Value {
    json!({
        "kind": kind.into(),
        "repo": repo.into(),
        "message": message.into(),
    })
}

fn chunk_references_meta_workspace(chunk: &Value) -> bool {
    chunk
        .pointer("/span/file")
        .and_then(Value::as_str)
        .is_some_and(|path| path.split('/').any(|part| part == ".meta-workspace"))
}

fn infer_capabilities(text: &str) -> Vec<&'static str> {
    let lower = text.to_ascii_lowercase();
    let mut capabilities = Vec::new();
    for (needle, capability) in [
        ("validation", "validation"),
        ("cargo ", "validation"),
        ("simdoc", "docs"),
        ("codec", "codec"),
        ("stream", "stream"),
        ("agent", "agent"),
        ("capability", "capability"),
    ] {
        if lower.contains(needle) && !capabilities.contains(&capability) {
            capabilities.push(capability);
        }
    }
    capabilities
}

fn infer_codecs(text: &str) -> Vec<&'static str> {
    let lower = text.to_ascii_lowercase();
    let mut codecs = Vec::new();
    for (needle, codec) in [
        ("lisp", "lisp"),
        ("json", "json"),
        ("algol", "algol"),
        ("chat", "chat"),
        ("mcp", "mcp"),
        ("binary", "binary"),
    ] {
        if lower.contains(needle) {
            codecs.push(codec);
        }
    }
    codecs
}

fn content_hash(text: &str) -> String {
    format!("fnv1a64:{}", fnv1a64_hex(text.as_bytes()))
}

fn cache_file(options: &AtelierIndexOptions) -> PathBuf {
    options
        .cache_dir
        .clone()
        .unwrap_or_else(|| options.control_root.join(DEFAULT_CACHE_DIR))
        .join(INDEX_FILE)
}

fn manifest_display(control_root: &Path, manifest_path: &Path) -> String {
    manifest_path
        .strip_prefix(control_root)
        .unwrap_or(manifest_path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier index json: {err}"))
}

fn print_usage() {
    println!(
        "usage: xtask atelier-index [--control-root PATH] [--repos-manifest PATH] [--cache DIR] [--max-chunk-bytes N] [--check|--refresh-only]"
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
