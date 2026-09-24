use super::registry;
use serde_json::Value as Json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

/// Canonicalizes a Cargo unit graph across a deliberate local-workspace move.
///
/// Registry identities remain exact. Local package paths are replaced only
/// after metadata binds their package name/version and each target source is
/// proven relative to that package's manifest root. Every other unit, profile,
/// feature, dependency-index and root field remains byte-semantic JSON.
pub(super) fn canonical_unit_graph(metadata: &Json, graph: &Json) -> Result<Json, String> {
    let packages = metadata
        .get("packages")
        .and_then(Json::as_array)
        .ok_or("Cargo metadata has no packages")?;
    let mut identities = BTreeMap::new();
    for package in packages {
        let id = string(package, "id")?;
        let name = string(package, "name")?;
        let version = string(package, "version")?;
        let manifest = PathBuf::from(string(package, "manifest_path")?);
        let root = manifest
            .parent()
            .ok_or("Cargo package manifest has no parent")?
            .to_owned();
        let source = package.get("source").and_then(Json::as_str);
        let identity = match source {
            Some(source) if source.starts_with("registry+") => {
                format!("{source}#{name}@{version}")
            }
            Some(_) => return Err(format!("Cargo package has unsupported source: {id}")),
            None => format!("local:{name}@{version}"),
        };
        if identities.insert(id.to_owned(), (identity, root)).is_some() {
            return Err("Cargo metadata repeats a package identity".into());
        }
    }
    let mut canonical = graph.clone();
    if canonical.get("version").and_then(Json::as_u64) != Some(1) {
        return Err("unit graph schema is not version 1".into());
    }
    let units = canonical
        .get_mut("units")
        .and_then(Json::as_array_mut)
        .ok_or("unit graph has no units")?;
    if units.is_empty() || units.len() > 16_384 {
        return Err("unit graph unit count is outside its finite bound".into());
    }
    for unit in units {
        let id = unit
            .get("pkg_id")
            .and_then(Json::as_str)
            .ok_or("unit graph package id is invalid")?
            .to_owned();
        let (identity, root) = identities
            .get(&id)
            .ok_or_else(|| format!("unit graph package is absent from metadata: {id}"))?;
        let target = unit
            .get_mut("target")
            .and_then(Json::as_object_mut)
            .ok_or("unit graph target is invalid")?;
        let source = PathBuf::from(
            target
                .get("src_path")
                .and_then(Json::as_str)
                .ok_or("unit graph target source is invalid")?,
        );
        let relative = source
            .strip_prefix(root)
            .map_err(|_| format!("unit graph target escaped package root: {id}"))?;
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(format!("unit graph target path is not canonical: {id}"));
        }
        let relative = relative
            .to_str()
            .filter(|value| value.len() <= 4096 && !value.contains('\\'))
            .ok_or("unit graph target relative path is invalid")?;
        target.insert("src_path".into(), Json::String(relative.into()));
        unit.as_object_mut()
            .ok_or("unit graph unit is invalid")?
            .insert("pkg_id".into(), Json::String(identity.clone()));
    }
    Ok(canonical)
}

pub(super) struct Package {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) version: String,
    pub(super) manifest: PathBuf,
    pub(super) registry: bool,
    pub(super) targets: Vec<SelectedTarget>,
}

pub(super) struct SelectedTarget {
    pub(super) source: PathBuf,
    pub(super) custom_build: bool,
}

pub(super) struct ResolverPackage {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) manifest: PathBuf,
    pub(super) target_entries: Vec<PathBuf>,
}

pub(super) fn selected_package_ids(graph: &Json) -> Result<BTreeSet<String>, String> {
    if graph.get("version").and_then(Json::as_u64) != Some(1) {
        return Err("unit graph schema is not version 1".into());
    }
    let units = graph
        .get("units")
        .and_then(Json::as_array)
        .ok_or("unit graph has no units")?;
    if units.is_empty() || units.len() > 16_384 {
        return Err("unit graph unit count is outside its finite bound".into());
    }
    units
        .iter()
        .map(|unit| {
            unit.get("pkg_id")
                .and_then(Json::as_str)
                .filter(|id| !id.is_empty() && id.len() <= 4096)
                .map(str::to_owned)
                .ok_or_else(|| "unit graph package id is invalid".into())
        })
        .collect()
}

pub(super) fn selected_packages(
    metadata: &Json,
    selected: &BTreeSet<String>,
    graph: &Json,
) -> Result<Vec<Package>, String> {
    let mut targets = selected_targets(graph)?;
    let rows = metadata
        .get("packages")
        .and_then(Json::as_array)
        .ok_or("Cargo metadata has no packages")?;
    let mut packages = Vec::new();
    for row in rows {
        let id = string(row, "id")?;
        if !selected.contains(id) {
            continue;
        }
        let source = row.get("source").and_then(Json::as_str);
        if source.is_some_and(|source| !source.starts_with("registry+")) {
            return Err(format!("selected package has unsupported source: {id}"));
        }
        packages.push(Package {
            id: id.to_owned(),
            name: string(row, "name")?.to_owned(),
            version: string(row, "version")?.to_owned(),
            manifest: PathBuf::from(string(row, "manifest_path")?),
            registry: source.is_some(),
            targets: targets
                .remove(id)
                .ok_or_else(|| format!("selected package has no unit-graph target: {id}"))?,
        });
    }
    packages.sort_by(|left, right| left.id.cmp(&right.id));
    let found = packages
        .iter()
        .map(|package| package.id.clone())
        .collect::<BTreeSet<_>>();
    if &found != selected {
        return Err("unit graph and Cargo metadata package sets differ".into());
    }
    if !targets.is_empty() {
        return Err("unit graph target package set exceeds Cargo metadata".into());
    }
    Ok(packages)
}

pub(super) fn resolver_local_packages(
    metadata: &Json,
    resolver: &BTreeSet<String>,
) -> Result<Vec<ResolverPackage>, String> {
    let rows = metadata
        .get("packages")
        .and_then(Json::as_array)
        .ok_or("Cargo metadata has no packages")?;
    let local = rows
        .iter()
        .filter(|row| row.get("source").is_none_or(Json::is_null))
        .map(|row| Ok((string(row, "id")?.to_owned(), row)))
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let mut packages = local
        .into_iter()
        .filter(|(id, _)| resolver.contains(id))
        .map(|(id, row)| {
            Ok(ResolverPackage {
                id: id.clone(),
                name: string(row, "name")?.to_owned(),
                manifest: PathBuf::from(string(row, "manifest_path")?),
                target_entries: target_entries(row)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    packages.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(packages)
}

pub(super) fn local_patch_packages(
    workspace: &Path,
    root_manifest: &[u8],
) -> Result<Vec<ResolverPackage>, String> {
    let root: toml::Value =
        toml::from_str(std::str::from_utf8(root_manifest).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let workspace_version = root
        .get("workspace")
        .and_then(|value| value.get("package"))
        .and_then(|value| value.get("version"))
        .and_then(toml::Value::as_str);
    let mut packages: BTreeMap<String, ResolverPackage> = BTreeMap::new();
    let Some(registries) = root.get("patch").and_then(toml::Value::as_table) else {
        return Ok(Vec::new());
    };
    for entries in registries.values().filter_map(toml::Value::as_table) {
        for value in entries.values() {
            let Some(relative) = value
                .as_table()
                .and_then(|table| table.get("path"))
                .and_then(toml::Value::as_str)
            else {
                continue;
            };
            let relative = Path::new(relative);
            if relative.components().count() != 2
                || relative.components().next()
                    != Some(std::path::Component::Normal("packages".as_ref()))
            {
                return Err(format!("local patch path is not canonical: {relative:?}"));
            }
            let package_root = workspace.join(relative).canonicalize().map_err(|error| {
                format!(
                    "local patch package {}: {error}",
                    workspace.join(relative).display()
                )
            })?;
            if package_root.parent() != Some(&workspace.join("packages")) {
                return Err("local patch package escaped the workspace package root".into());
            }
            let manifest = package_root.join("Cargo.toml");
            let package_manifest = fs::read_to_string(&manifest)
                .map_err(|error| format!("{}: {error}", manifest.display()))?;
            let package_manifest: toml::Value =
                toml::from_str(&package_manifest).map_err(|error| error.to_string())?;
            let package = package_manifest
                .get("package")
                .and_then(toml::Value::as_table)
                .ok_or("local patch manifest has no package table")?;
            let name = package
                .get("name")
                .and_then(toml::Value::as_str)
                .filter(|name| !name.is_empty() && name.len() <= 128)
                .ok_or("local patch package name is invalid")?
                .to_owned();
            let version = match package.get("version") {
                Some(toml::Value::String(version)) => version.as_str(),
                Some(toml::Value::Table(inherited))
                    if inherited.get("workspace").and_then(toml::Value::as_bool) == Some(true) =>
                {
                    workspace_version.ok_or("local patch inherits a missing workspace version")?
                }
                _ => return Err("local patch package version is invalid".into()),
            };
            let target_entries = registry::manifest_target_entries(&manifest, &package_root)?;
            let id = format!("path+file://{}#{version}", package_root.display());
            let candidate = ResolverPackage {
                id,
                name: name.clone(),
                manifest,
                target_entries,
            };
            if let Some(existing) = packages.get(&name) {
                if existing.id == candidate.id
                    && existing.manifest == candidate.manifest
                    && existing.target_entries == candidate.target_entries
                {
                    continue;
                }
                return Err("local patch catalog maps one package name to divergent owners".into());
            }
            packages.insert(name, candidate);
        }
    }
    Ok(packages.into_values().collect())
}

pub(super) fn resolver_only_package_ids(
    metadata: &Json,
    selected: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    let nodes = metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(Json::as_array)
        .ok_or("Cargo metadata has no resolver nodes")?;
    if nodes.is_empty() || nodes.len() > 16_384 {
        return Err("Cargo resolver node count is outside its finite bound".into());
    }
    let mut edges = BTreeMap::new();
    for node in nodes {
        let id = string(node, "id")?.to_owned();
        let dependencies = node
            .get("dependencies")
            .and_then(Json::as_array)
            .ok_or("Cargo resolver node dependencies are invalid")?
            .iter()
            .map(|dependency| {
                dependency
                    .as_str()
                    .filter(|value| !value.is_empty() && value.len() <= 4096)
                    .map(str::to_owned)
                    .ok_or_else(|| "Cargo resolver dependency identity is invalid".to_owned())
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if edges.insert(id, dependencies).is_some() {
            return Err("Cargo metadata repeats a resolver node".into());
        }
    }
    let mut closure = selected.clone();
    let mut pending = selected.iter().cloned().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        let dependencies = edges
            .get(&id)
            .ok_or_else(|| format!("selected package has no Cargo resolver node: {id}"))?;
        for dependency in dependencies {
            if !edges.contains_key(dependency) {
                return Err(format!(
                    "Cargo resolver dependency has no package node: {dependency}"
                ));
            }
            if closure.insert(dependency.clone()) {
                pending.push(dependency.clone());
            }
        }
    }
    Ok(closure.difference(selected).cloned().collect())
}

pub(super) fn target_entries(row: &Json) -> Result<Vec<PathBuf>, String> {
    let mut entries = row
        .get("targets")
        .and_then(Json::as_array)
        .ok_or("resolver package targets are invalid")?
        .iter()
        .filter(|target| {
            target
                .get("kind")
                .and_then(Json::as_array)
                .is_some_and(|kinds| {
                    !kinds.is_empty()
                        && kinds.iter().all(|kind| {
                            !matches!(kind.as_str(), Some("test" | "example" | "bench"))
                        })
                })
        })
        .map(|target| Ok(PathBuf::from(string(target, "src_path")?)))
        .collect::<Result<Vec<_>, String>>()?;
    entries.sort();
    entries.dedup();
    Ok(entries)
}

fn selected_targets(graph: &Json) -> Result<BTreeMap<String, Vec<SelectedTarget>>, String> {
    let units = graph
        .get("units")
        .and_then(Json::as_array)
        .ok_or("unit graph has no units")?;
    let mut targets = BTreeMap::<String, Vec<SelectedTarget>>::new();
    for unit in units {
        let id = string(unit, "pkg_id")?.to_owned();
        let target = unit
            .get("target")
            .and_then(Json::as_object)
            .ok_or("unit graph target is invalid")?;
        let source = target
            .get("src_path")
            .and_then(Json::as_str)
            .filter(|path| !path.is_empty() && path.len() <= 4096)
            .ok_or("unit graph target source is invalid")?;
        let kinds = target
            .get("kind")
            .and_then(Json::as_array)
            .ok_or("unit graph target kind is invalid")?;
        let custom_build = kinds
            .iter()
            .any(|kind| kind.as_str() == Some("custom-build"));
        let selected = targets.entry(id).or_default();
        if !selected
            .iter()
            .any(|target| target.source == Path::new(source) && target.custom_build == custom_build)
        {
            selected.push(SelectedTarget {
                source: PathBuf::from(source),
                custom_build,
            });
        }
    }
    for selected in targets.values_mut() {
        selected.sort_by(|left, right| left.source.cmp(&right.source));
    }
    Ok(targets)
}

fn string<'a>(value: &'a Json, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Json::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .ok_or_else(|| format!("Cargo metadata {field} is invalid"))
}
