use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

use serde_json::{Value, json};

use super::index_manifest::RepoEntry;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RepoRustContext {
    pub(super) packages: BTreeMap<String, CargoPackage>,
    pub(super) rustdoc_items: BTreeMap<RustdocKey, RustdocItem>,
    pub(super) diagnostics: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CargoPackage {
    pub(super) name: String,
    pub(super) root: String,
    pub(super) features: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct RustdocKey {
    pub(super) crate_name: String,
    pub(super) item_kind: String,
    pub(super) item_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RustdocItem {
    pub(super) docs: Option<String>,
    pub(super) feature_gates: Vec<String>,
    pub(super) span: Option<RustdocSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RustdocSpan {
    pub(super) file: String,
    pub(super) line: usize,
}

pub(super) fn repo_context(repo: &RepoEntry) -> RepoRustContext {
    if !repo.contains_code || !repo.checkout_path.is_dir() {
        return RepoRustContext::default();
    }

    let mut diagnostics = Vec::new();
    let packages = match cargo_metadata_packages(repo) {
        Ok(packages) => packages,
        Err(err) => {
            diagnostics.push(diagnostic(
                "cargo-metadata-unavailable",
                &repo.name,
                format!("{err}; using Cargo.toml package metadata"),
            ));
            manifest_packages(repo)
        }
    };
    let mut rustdoc_items = BTreeMap::new();
    for package in packages.values() {
        match read_rustdoc_json(repo, package) {
            Ok(items) => rustdoc_items.extend(items),
            Err(err) => diagnostics.push(diagnostic("rustdoc-json-invalid", &repo.name, err)),
        }
    }

    RepoRustContext {
        packages,
        rustdoc_items,
        diagnostics,
    }
}

pub(super) fn manifest_packages(repo: &RepoEntry) -> BTreeMap<String, CargoPackage> {
    let mut packages = BTreeMap::new();
    push_manifest_package(".", &repo.checkout_path, &mut packages);
    for source in &repo.source_paths {
        push_manifest_package(source, &repo.checkout_path.join(source), &mut packages);
    }
    packages
}

pub(super) fn package_name(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let value = text.parse::<toml::Value>().ok()?;
    value
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
}

fn cargo_metadata_packages(repo: &RepoEntry) -> Result<BTreeMap<String, CargoPackage>, String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .current_dir(&repo.checkout_path)
        .output()
        .map_err(|err| format!("run cargo metadata for {}: {err}", repo.name))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata for {} failed: {}",
            repo.name,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("parse cargo metadata for {}: {err}", repo.name))?;
    let mut packages = BTreeMap::new();
    for package in value["packages"].as_array().into_iter().flatten() {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        let root = package["manifest_path"]
            .as_str()
            .and_then(|manifest| Path::new(manifest).parent())
            .map(|root| display_path(repo, root))
            .unwrap_or_else(|| ".".to_owned());
        packages.insert(
            name.to_owned(),
            CargoPackage {
                name: name.to_owned(),
                root,
                features: sorted_keys(&package["features"]),
            },
        );
    }
    Ok(packages)
}

fn push_manifest_package(
    display_root: &str,
    root: &Path,
    packages: &mut BTreeMap<String, CargoPackage>,
) {
    let manifest = root.join("Cargo.toml");
    if !manifest.is_file() {
        return;
    }
    let Ok(text) = fs::read_to_string(&manifest) else {
        return;
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return;
    };
    let Some(name) = value
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
    else {
        return;
    };
    packages.insert(
        name.to_owned(),
        CargoPackage {
            name: name.to_owned(),
            root: display_root.to_owned(),
            features: toml_table_keys(value.get("features")),
        },
    );
}

fn read_rustdoc_json(
    repo: &RepoEntry,
    package: &CargoPackage,
) -> Result<BTreeMap<RustdocKey, RustdocItem>, String> {
    let underscored = package.name.replace('-', "_");
    let candidates = [
        repo.checkout_path
            .join("target")
            .join("doc")
            .join(format!("{underscored}.json")),
        repo.checkout_path
            .join("target")
            .join("doc")
            .join(format!("{}.json", package.name)),
    ];
    let Some(path) = candidates.iter().find(|path| path.is_file()) else {
        return Ok(BTreeMap::new());
    };
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|err| format!("parse {}: {err}", path.display()))?;
    Ok(parse_rustdoc_items(&package.name, &value))
}

fn parse_rustdoc_items(crate_name: &str, value: &Value) -> BTreeMap<RustdocKey, RustdocItem> {
    let mut items = BTreeMap::new();
    let Some(index) = value["index"].as_object() else {
        return items;
    };
    for item in index.values() {
        let Some(name) = item["name"].as_str() else {
            continue;
        };
        let Some(kind) = rustdoc_item_kind(item) else {
            continue;
        };
        let attrs = string_array(&item["attrs"]);
        items.insert(
            RustdocKey {
                crate_name: crate_name.to_owned(),
                item_kind: kind.to_owned(),
                item_name: name.to_owned(),
            },
            RustdocItem {
                docs: item["docs"].as_str().map(str::to_owned),
                feature_gates: feature_gates(&attrs),
                span: rustdoc_span(&item["span"]),
            },
        );
    }
    items
}

fn rustdoc_item_kind(item: &Value) -> Option<&'static str> {
    let inner = item["inner"].as_object()?;
    for (key, kind) in [
        ("function", "fn"),
        ("struct", "struct"),
        ("enum", "enum"),
        ("trait", "trait"),
        ("type_alias", "type"),
        ("constant", "const"),
        ("module", "mod"),
    ] {
        if inner.contains_key(key) {
            return Some(kind);
        }
    }
    None
}

fn rustdoc_span(value: &Value) -> Option<RustdocSpan> {
    let file = value["filename"].as_str()?.to_owned();
    let line = value["begin"]
        .as_array()
        .and_then(|begin| begin.first())
        .and_then(Value::as_u64)
        .or_else(|| value.pointer("/begin/line").and_then(Value::as_u64))
        .unwrap_or(1) as usize;
    Some(RustdocSpan { file, line })
}

fn feature_gates(attrs: &[String]) -> Vec<String> {
    let mut gates = BTreeSet::new();
    for attr in attrs {
        let mut rest = attr.as_str();
        while let Some(index) = rest.find("feature") {
            rest = &rest[index + "feature".len()..];
            let Some(start) = rest.find('"') else {
                break;
            };
            let after_start = &rest[start + 1..];
            let Some(end) = after_start.find('"') else {
                break;
            };
            gates.insert(after_start[..end].to_owned());
            rest = &after_start[end + 1..];
        }
    }
    gates.into_iter().collect()
}

fn sorted_keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

fn toml_table_keys(value: Option<&toml::Value>) -> Vec<String> {
    let mut keys = value
        .and_then(toml::Value::as_table)
        .map(|table| table.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    keys
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

fn display_path(repo: &RepoEntry, path: &Path) -> String {
    path.strip_prefix(&repo.checkout_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
        .trim_start_matches("./")
        .to_owned()
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
