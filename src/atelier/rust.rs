use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};

use super::{
    index_manifest::RepoEntry,
    index_units::SourceUnit,
    rust_metadata::{
        RepoRustContext, RustdocKey, RustdocSpan, manifest_packages, package_name, repo_context,
    },
};

const RUST_SCHEMA: &str = "sim.atelier.rust-intelligence.v1";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RustIntelligence {
    items_by_unit: BTreeMap<String, RustItemFact>,
    diagnostics: Vec<Value>,
    rust_analyzer: String,
}

impl RustIntelligence {
    pub(super) fn diagnostics(&self) -> &[Value] {
        &self.diagnostics
    }

    pub(super) fn fact_for_unit(&self, unit_id: &str) -> Option<&RustItemFact> {
        self.items_by_unit.get(unit_id)
    }

    pub(super) fn index_json(&self) -> Value {
        json!({
            "schema": RUST_SCHEMA,
            "adapter": "cargo-metadata+rustdoc-json",
            "optional_tools": {
                "rust_analyzer": self.rust_analyzer,
            },
            "items": self.items_by_unit.values().map(RustItemFact::json).collect::<Vec<_>>(),
            "diagnostics": self.diagnostics,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RustItemFact {
    source_unit: String,
    ide_object_id: String,
    repo: String,
    crate_name: String,
    module: String,
    item_kind: String,
    item_name: String,
    source: RustSpan,
    docs: Option<RustSpan>,
    docs_summary: Option<String>,
    feature_gates: Vec<String>,
    crate_features: Vec<String>,
    linked_tests: Vec<RustSpan>,
}

impl RustItemFact {
    pub(super) fn json(&self) -> Value {
        json!({
            "source_unit": self.source_unit,
            "ide_object_id": self.ide_object_id,
            "repo": self.repo,
            "crate": self.crate_name,
            "module": self.module,
            "item_kind": self.item_kind,
            "item_name": self.item_name,
            "source": self.source.json(),
            "docs": self.docs.as_ref().map(RustSpan::json),
            "docs_summary": self.docs_summary,
            "feature_gates": self.feature_gates,
            "crate_features": self.crate_features,
            "linked_tests": self.linked_tests.iter().map(RustSpan::json).collect::<Vec<_>>(),
            "browse": {
                "kind": "sim-browse-object",
                "id": self.ide_object_id,
                "object_kind": "rust-item",
                "surfaces": ["source", "docs", "tests", "features"],
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RustSpan {
    pub(super) repo: String,
    pub(super) file: String,
    pub(super) line: usize,
}

impl RustSpan {
    fn json(&self) -> Value {
        json!({
            "repo": self.repo,
            "file": self.file,
            "line": self.line,
        })
    }
}

pub(super) fn build_rust_intelligence(
    repos: &[RepoEntry],
    units: &[SourceUnit],
) -> RustIntelligence {
    let mut contexts = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for repo in repos {
        let context = repo_context(repo);
        diagnostics.extend(context.diagnostics.clone());
        contexts.insert(repo.name.clone(), context);
    }

    let repos_by_name = repos
        .iter()
        .map(|repo| (repo.name.as_str(), repo))
        .collect::<BTreeMap<_, _>>();
    let mut items_by_unit = BTreeMap::new();
    for unit in units.iter().filter(|unit| unit.kind.starts_with("rust-")) {
        let Some(repo) = repos_by_name.get(unit.repo.as_str()) else {
            continue;
        };
        let Some(context) = contexts.get(&unit.repo) else {
            continue;
        };
        if let Some(fact) = item_fact(repo, context, repos, unit) {
            items_by_unit.insert(unit.id.clone(), fact);
        }
    }

    RustIntelligence {
        items_by_unit,
        diagnostics,
        rust_analyzer: rust_analyzer_status(),
    }
}

pub(super) fn remap_meta_workspace_path(repos: &[RepoEntry], path: &Path) -> Option<RustSpan> {
    let parts = normalized_parts(path);
    let meta_index = parts.iter().position(|part| part == ".meta-workspace")?;
    if parts
        .get(meta_index + 1)
        .is_none_or(|part| part != "packages")
    {
        return None;
    }
    let package = parts.get(meta_index + 2)?;
    let rest = parts.get(meta_index + 3..).unwrap_or_default().join("/");
    if rest.is_empty() {
        return None;
    }

    for repo in repos {
        let Some(source_root) = source_root_for_package(repo, package) else {
            continue;
        };
        let file = if source_root == "." {
            rest.clone()
        } else {
            format!("{source_root}/{rest}")
        };
        return Some(RustSpan {
            repo: repo.name.clone(),
            file,
            line: 1,
        });
    }
    None
}

fn item_fact(
    repo: &RepoEntry,
    context: &RepoRustContext,
    repos: &[RepoEntry],
    unit: &SourceUnit,
) -> Option<RustItemFact> {
    let crate_name = unit
        .crate_name
        .clone()
        .or_else(|| crate_from_path(context, &unit.path))?;
    let item_kind = unit.kind.strip_prefix("rust-")?.to_owned();
    let item_name = item_name_from_unit(unit)?;
    let rustdoc = context.rustdoc_items.get(&RustdocKey {
        crate_name: crate_name.clone(),
        item_kind: item_kind.clone(),
        item_name: item_name.clone(),
    });
    let source = rustdoc
        .and_then(|item| item.span.as_ref())
        .and_then(|span| resolve_rustdoc_span(repo, repos, span))
        .unwrap_or_else(|| RustSpan {
            repo: unit.repo.clone(),
            file: unit.path.clone(),
            line: unit.line,
        });
    let docs_summary = rustdoc
        .and_then(|item| item.docs.clone())
        .or_else(|| docs_summary_from_unit(unit));
    let docs = docs_summary.as_ref().map(|_| RustSpan {
        repo: source.repo.clone(),
        file: source.file.clone(),
        line: source.line,
    });
    let feature_gates = rustdoc
        .map(|item| item.feature_gates.clone())
        .unwrap_or_default();
    let crate_features = context
        .packages
        .get(&crate_name)
        .map(|package| package.features.clone())
        .unwrap_or_default();
    let module = module_from_path(&source.file);
    let ide_object_id = ide_object_id(
        &source.repo,
        &crate_name,
        &module,
        &item_kind,
        &source.file,
        source.line,
    );

    Some(RustItemFact {
        source_unit: unit.id.clone(),
        ide_object_id,
        repo: source.repo.clone(),
        crate_name,
        module,
        item_kind,
        item_name: item_name.clone(),
        source,
        docs,
        docs_summary,
        feature_gates,
        crate_features,
        linked_tests: linked_tests(repo, &item_name),
    })
}

fn resolve_rustdoc_span(
    repo: &RepoEntry,
    repos: &[RepoEntry],
    span: &RustdocSpan,
) -> Option<RustSpan> {
    let path = Path::new(&span.file);
    if span.file.split('/').any(|part| part == ".meta-workspace") {
        return remap_meta_workspace_path(repos, path).map(|mut remapped| {
            remapped.line = span.line.max(1);
            remapped
        });
    }
    Some(RustSpan {
        repo: repo.name.clone(),
        file: display_path(repo, path),
        line: span.line.max(1),
    })
}

fn linked_tests(repo: &RepoEntry, item_name: &str) -> Vec<RustSpan> {
    let mut spans = BTreeMap::new();
    for root in test_roots(repo) {
        for path in rust_files(&root) {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let file = display_path(repo, &path);
            if !is_test_source(&file, &text) {
                continue;
            }
            let Some(line) = line_containing(&text, item_name) else {
                continue;
            };
            spans.insert(
                file.clone(),
                RustSpan {
                    repo: repo.name.clone(),
                    file,
                    line,
                },
            );
        }
    }
    spans.into_values().collect()
}

fn is_test_source(path: &str, text: &str) -> bool {
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.ends_with("_test.rs")
        || path.ends_with("_tests.rs")
        || text.contains("#[test]")
}

fn test_roots(repo: &RepoEntry) -> Vec<PathBuf> {
    let mut roots = vec![repo.checkout_path.join("tests")];
    for package in manifest_packages(repo).values() {
        let root = if package.root == "." {
            repo.checkout_path.clone()
        } else {
            repo.checkout_path.join(&package.root)
        };
        roots.push(root.join("tests"));
        roots.push(root.join("src"));
    }
    roots
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if path
            .file_name()
            .is_some_and(|name| name == ".meta-workspace")
        {
            continue;
        }
        if path.is_dir() {
            if let Ok(entries) = fs::read_dir(&path) {
                for entry in entries.flatten() {
                    stack.push(entry.path());
                }
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn source_root_for_package(repo: &RepoEntry, package: &str) -> Option<String> {
    if package_name(&repo.checkout_path).as_deref() == Some(package) {
        return Some(".".to_owned());
    }
    for source in &repo.source_paths {
        let root = repo.checkout_path.join(source);
        if package_name(&root).as_deref() == Some(package) {
            return Some(source.clone());
        }
    }
    if repo.crate_names.iter().any(|name| name == package) {
        return repo
            .source_paths
            .iter()
            .find(|source| source.rsplit('/').next() == Some(package))
            .cloned()
            .or_else(|| (repo.source_paths.len() == 1).then(|| repo.source_paths[0].clone()));
    }
    None
}

fn crate_from_path(context: &RepoRustContext, path: &str) -> Option<String> {
    context
        .packages
        .values()
        .filter(|package| package.root == "." || path.starts_with(&format!("{}/", package.root)))
        .max_by_key(|package| package.root.len())
        .map(|package| package.name.clone())
}

fn item_name_from_unit(unit: &SourceUnit) -> Option<String> {
    let signature = unit.text.lines().find_map(|line| {
        line.strip_prefix("signature:")
            .map(str::trim)
            .filter(|text| !text.is_empty())
    })?;
    signature
        .split_whitespace()
        .last()
        .map(|name| name.trim_end_matches("()").to_owned())
}

fn docs_summary_from_unit(unit: &SourceUnit) -> Option<String> {
    let docs = unit.text.split_once("docs:\n")?.1.trim();
    (!docs.is_empty()).then(|| docs.lines().next().unwrap_or(docs).to_owned())
}

fn module_from_path(path: &str) -> String {
    let module_path = path
        .strip_prefix("src/")
        .unwrap_or(path)
        .trim_end_matches(".rs");
    match module_path {
        "lib" | "main" => "crate".to_owned(),
        path if path.ends_with("/mod") => path.trim_end_matches("/mod").replace('/', "::"),
        path => path.replace('/', "::"),
    }
}

fn ide_object_id(
    repo: &str,
    crate_name: &str,
    module: &str,
    item_kind: &str,
    file: &str,
    line: usize,
) -> String {
    format!(
        "ide://rust/{}/{}/{}/{}/{}@{}",
        stable_path_id(repo),
        stable_path_id(crate_name),
        stable_path_id(module),
        stable_path_id(item_kind),
        stable_path_id(file),
        line.max(1)
    )
}

fn line_containing(text: &str, needle: &str) -> Option<usize> {
    text.lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
}

fn display_path(repo: &RepoEntry, path: &Path) -> String {
    path.strip_prefix(&repo.checkout_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
        .trim_start_matches("./")
        .to_owned()
}

fn normalized_parts(path: &Path) -> Vec<String> {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn stable_path_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn rust_analyzer_status() -> String {
    match Command::new("rust-analyzer").arg("--version").output() {
        Ok(output) if output.status.success() => "available".to_owned(),
        Ok(_) | Err(_) => "missing".to_owned(),
    }
}
