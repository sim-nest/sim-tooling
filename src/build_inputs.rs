// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Materializes one exact Cargo unit graph into finite regular-file inputs.

use self::graph::{
    Package, ResolverPackage, local_patch_packages, resolver_local_packages,
    resolver_only_package_ids, selected_package_ids, selected_packages,
};
use self::limits::LimitsReport;
use self::manifest::{attach_resolver_package_to_workspace, narrow_workspace_manifest};
use serde::Serialize;
use serde_json::Value as Json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

mod bounded_process;
mod derive;
mod finalize;
mod graph;
mod limits;
mod manifest;
mod registry;
mod selected_view;
mod tree;

pub(crate) use self::tree::{
    FileRecord, TreeLimit, TreeReport, TreeWriter, bounded_read, canonical_directory, digest,
    staging_path, sync_directory, write_new,
};

struct Options {
    metadata: PathBuf,
    unit_graph: PathBuf,
    workspace: PathBuf,
    vendor_root: PathBuf,
    lock: PathBuf,
    config: PathBuf,
    destination: PathBuf,
    owner_roots: Vec<PathBuf>,
    expected: BTreeMap<&'static str, String>,
    source_limit: TreeLimit,
    vendor_limit: TreeLimit,
    input_limit: TreeLimit,
}

#[derive(Serialize)]
struct Report {
    schema: &'static str,
    inputs: BTreeMap<&'static str, String>,
    unit_graph_units: usize,
    local_packages: Vec<String>,
    resolver_only_local_packages: Vec<String>,
    registry_packages: Vec<String>,
    resolver_only_registry_packages: Vec<String>,
    limits: LimitsReport,
    source: TreeReport,
    vendor: TreeReport,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    if args.get(2).map(String::as_str) == Some("select") {
        return selected_view::run(&args);
    }
    if args.get(2).map(String::as_str) == Some("derive") {
        return derive::run(&args);
    }
    if args.get(2).map(String::as_str) == Some("finalize") {
        return finalize::run(&args);
    }
    if args.get(2).map(String::as_str) != Some("materialize") {
        return Err(usage(args.first().map(String::as_str).unwrap_or("xtask")));
    }
    let options = parse(&args)?;
    let destination = options.destination.clone();
    let report = materialize(options)?;
    println!(
        "build-inputs: {} local + {} registry packages; source {}/{} bytes, vendor {}/{} bytes; {}",
        report.local_packages.len(),
        report.registry_packages.len(),
        report.source.entries,
        report.source.bytes,
        report.vendor.entries,
        report.vendor.bytes,
        destination.display()
    );
    Ok(())
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut values = BTreeMap::new();
    let mut owner_roots = Vec::new();
    let mut index = 3;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        if flag == "--owner-root" {
            owner_roots.push(PathBuf::from(value));
        } else if values.insert(flag, value.clone()).is_some() {
            return Err(format!("build-inputs repeats {flag}"));
        }
        index += 2;
    }
    let required = |name: &'static str| {
        values
            .get(name)
            .cloned()
            .ok_or_else(|| format!("build-inputs requires {name}"))
    };
    if owner_roots.is_empty() {
        return Err("build-inputs requires at least one --owner-root".into());
    }
    let number = |name: &'static str, hard: usize| -> Result<usize, String> {
        let value = required(name)?
            .parse::<usize>()
            .map_err(|_| format!("{name} is not a canonical positive integer"))?;
        if value == 0 || value > hard {
            return Err(format!("{name} exceeds its hard bound"));
        }
        Ok(value)
    };
    let mut expected = BTreeMap::new();
    for (key, flag) in [
        ("metadata", "--expected-metadata-sha256"),
        ("unit-graph", "--expected-unit-graph-sha256"),
        ("workspace-manifest", "--expected-workspace-manifest-sha256"),
        ("lock", "--expected-lock-sha256"),
        ("config", "--expected-config-sha256"),
    ] {
        let digest = required(flag)?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{flag} is not a SHA-256 digest"));
        }
        expected.insert(key, digest.to_ascii_lowercase());
    }
    Ok(Options {
        metadata: PathBuf::from(required("--metadata")?),
        unit_graph: PathBuf::from(required("--unit-graph")?),
        workspace: PathBuf::from(required("--workspace")?),
        vendor_root: PathBuf::from(required("--vendor-root")?),
        lock: PathBuf::from(required("--lock")?),
        config: PathBuf::from(required("--config")?),
        destination: PathBuf::from(required("--destination")?),
        owner_roots,
        expected,
        source_limit: TreeLimit {
            entries: number("--max-source-entries", 4096)?,
            bytes: number("--max-source-bytes", 256 * 1024 * 1024)?,
        },
        vendor_limit: TreeLimit {
            entries: number("--max-vendor-entries", 4096)?,
            bytes: number("--max-vendor-bytes", 256 * 1024 * 1024)?,
        },
        input_limit: TreeLimit {
            entries: number("--max-input-entries", 8192)?,
            bytes: number("--max-input-bytes", 512 * 1024 * 1024)?,
        },
    })
}

fn materialize(options: Options) -> Result<Report, String> {
    let workspace = canonical_directory(&options.workspace)?;
    let vendor_root = canonical_directory(&options.vendor_root)?;
    let owner_roots = options
        .owner_roots
        .iter()
        .map(|path| canonical_directory(path))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata_bytes = bounded_read(&options.metadata)?;
    let graph_bytes = bounded_read(&options.unit_graph)?;
    let manifest_path = workspace.join("Cargo.toml");
    let manifest_bytes = bounded_read(&manifest_path)?;
    let lock_bytes = bounded_read(&options.lock)?;
    let config_bytes = bounded_read(&options.config)?;
    let inputs = BTreeMap::from([
        ("metadata", digest(&metadata_bytes)),
        ("unit-graph", digest(&graph_bytes)),
        ("workspace-manifest", digest(&manifest_bytes)),
        ("lock", digest(&lock_bytes)),
        ("config", digest(&config_bytes)),
    ]);
    if inputs != options.expected {
        return Err("build-inputs exact input digests differ".into());
    }
    let graph: Json = serde_json::from_slice(&graph_bytes).map_err(|error| error.to_string())?;
    let metadata: Json =
        serde_json::from_slice(&metadata_bytes).map_err(|error| error.to_string())?;
    let selected = selected_package_ids(&graph)?;
    let packages = selected_packages(&metadata, &selected, &graph)?;
    let resolver_only = resolver_only_package_ids(&metadata, &selected)?;
    let resolver_packages = resolver_local_packages(&metadata, &resolver_only)?;
    let local = packages
        .iter()
        .filter(|package| !package.registry)
        .collect::<Vec<_>>();
    let registry = packages
        .iter()
        .filter(|package| package.registry)
        .collect::<Vec<_>>();
    let local_names = local
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut resolver_by_name = resolver_packages
        .into_iter()
        .filter(|package| !local_names.contains(package.name.as_str()))
        .map(|package| (package.name.clone(), package))
        .collect::<BTreeMap<_, _>>();
    for package in local_patch_packages(&workspace, &manifest_bytes)? {
        if !local_names.contains(package.name.as_str()) {
            resolver_by_name.insert(package.name.clone(), package);
        }
    }
    let resolver_packages = resolver_by_name.into_values().collect::<Vec<_>>();
    let resolver_registry_packages = registry::locked_resolver_packages(
        &lock_bytes,
        &vendor_root,
        &registry,
        &resolver_only,
        &resolver_packages,
    )?;
    if local.is_empty() || registry.is_empty() {
        return Err("build-inputs selected closure lacks local or registry packages".into());
    }
    if options.destination.exists() {
        return Err("build-inputs destination already exists".into());
    }
    let parent = options
        .destination
        .parent()
        .ok_or("build-inputs destination has no parent")?;
    let parent = canonical_directory(parent)?;
    let staging = staging_path(&parent, &options.destination)?;
    fs::create_dir(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let result = materialize_into(MaterializeInputs {
        staging: &staging,
        workspace: &workspace,
        vendor_root: &vendor_root,
        owner_roots: &owner_roots,
        manifest_bytes: &manifest_bytes,
        lock_bytes: &lock_bytes,
        config_bytes: &config_bytes,
        graph_units: graph
            .get("units")
            .and_then(Json::as_array)
            .ok_or("unit graph has no units")?
            .len(),
        packages: &packages,
        resolver_packages: &resolver_packages,
        resolver_registry_packages: &resolver_registry_packages,
        source_limit: options.source_limit,
        vendor_limit: options.vendor_limit,
        input_limit: options.input_limit,
        inputs,
    });
    let report = result.map_err(|error| {
        format!(
            "{error}; incomplete materialization retained at {}",
            staging.display()
        )
    })?;
    fs::rename(&staging, &options.destination).map_err(|error| {
        format!(
            "{} -> {}: {error}",
            staging.display(),
            options.destination.display()
        )
    })?;
    sync_directory(&parent)?;
    Ok(report)
}

struct MaterializeInputs<'a> {
    staging: &'a Path,
    workspace: &'a Path,
    vendor_root: &'a Path,
    owner_roots: &'a [PathBuf],
    manifest_bytes: &'a [u8],
    lock_bytes: &'a [u8],
    config_bytes: &'a [u8],
    graph_units: usize,
    packages: &'a [Package],
    resolver_packages: &'a [ResolverPackage],
    resolver_registry_packages: &'a [ResolverPackage],
    source_limit: TreeLimit,
    vendor_limit: TreeLimit,
    input_limit: TreeLimit,
    inputs: BTreeMap<&'static str, String>,
}

fn materialize_into(input: MaterializeInputs<'_>) -> Result<Report, String> {
    let source_root = input.staging.join("source");
    let vendor_root = input.staging.join("vendor");
    let mut source = TreeWriter::create(source_root, input.source_limit)?;
    let mut vendor = TreeWriter::create(vendor_root, input.vendor_limit)?;
    let local_names = input
        .packages
        .iter()
        .filter(|package| !package.registry)
        .map(|package| package.name.clone())
        .collect::<BTreeSet<_>>();
    let resolver_names = input
        .resolver_packages
        .iter()
        .map(|package| package.name.clone())
        .collect::<BTreeSet<_>>();
    let manifest_names = local_names.union(&resolver_names).cloned().collect();
    let root_manifest =
        narrow_workspace_manifest(input.manifest_bytes, &local_names, &manifest_names)?;
    source.write_file(Path::new("Cargo.toml"), &root_manifest, false)?;
    source.write_file(Path::new("Cargo.lock"), input.lock_bytes, false)?;
    source.write_file(Path::new(".cargo/config.toml"), input.config_bytes, false)?;
    let mut local_packages = Vec::new();
    let mut registry_packages = Vec::new();
    let mut resolver_only_local_packages = Vec::new();
    let mut resolver_only_registry_packages = Vec::new();
    let mut allowed = vec![input.workspace.to_owned()];
    allowed.extend_from_slice(input.owner_roots);
    for package in input.resolver_packages {
        let package_root = package
            .manifest
            .parent()
            .ok_or("resolver package manifest has no parent")?;
        let relative = package_root.strip_prefix(input.workspace).map_err(|_| {
            format!(
                "resolver package escaped selected workspace: {}",
                package.id
            )
        })?;
        let manifest = bounded_read(&package.manifest)?;
        let manifest = attach_resolver_package_to_workspace(&manifest)?;
        source.write_file(&relative.join("Cargo.toml"), &manifest, false)?;
        for target in &package.target_entries {
            let target_relative = target
                .strip_prefix(package_root)
                .map_err(|_| format!("resolver target escaped selected package: {}", package.id))?;
            source.copy_tree(target, &relative.join(target_relative), &allowed)?;
        }
        resolver_only_local_packages.push(package.id.clone());
    }
    for package in input.resolver_registry_packages {
        registry::copy_resolver(&mut vendor, package, input.vendor_root, input.lock_bytes)?;
        resolver_only_registry_packages.push(package.id.clone());
    }
    for package in input.packages {
        if package.registry {
            registry::copy_selected(&mut vendor, package, input.vendor_root, input.lock_bytes)?;
            registry_packages.push(package.id.clone());
        } else {
            let package_root = package
                .manifest
                .parent()
                .ok_or("local package manifest has no parent")?;
            let relative = package_root
                .strip_prefix(input.workspace)
                .map_err(|_| format!("local package escaped selected workspace: {}", package.id))?;
            // The whole package directory, not just each declared target's
            // own source directory: a target's compilation can still read
            // `include_str!`/`include_bytes!` assets or a build script's own
            // sibling modules outside that directory, and cargo metadata
            // names neither. Nothing here compiles to catch an incomplete
            // tree, so the safe input closure is the package's own directory
            // (bounded by the owner roots and the tree limits exactly as
            // before; `copy_tree` already excludes `.git`, `.sim`, and
            // `target`).
            source.copy_tree(package_root, relative, &allowed)?;
            local_packages.push(package.id.clone());
        }
    }
    local_packages.sort();
    registry_packages.sort();
    let source = source.finish()?;
    let vendor = vendor.finish()?;
    if source.entries + vendor.entries > input.input_limit.entries
        || source.bytes + vendor.bytes > input.input_limit.bytes
    {
        return Err("materialized input trees exceed their combined finite limit".into());
    }
    let report = Report {
        schema: "sim.build-input-materialization/v1",
        inputs: input.inputs,
        unit_graph_units: input.graph_units,
        local_packages,
        resolver_only_local_packages,
        registry_packages,
        resolver_only_registry_packages,
        limits: LimitsReport {
            source: input.source_limit,
            vendor: input.vendor_limit,
            combined_input: input.input_limit,
        },
        source,
        vendor,
    };
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    write_new(&input.staging.join("materialization.json"), &bytes, false)?;
    sync_directory(input.staging)?;
    Ok(report)
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} build-inputs <select --metadata <metadata.json> --unit-graph <unit-graph.json> --workspace <path> --lock <Cargo.lock> --config <config.toml> --vendor-root <path> --cargo-home <path> --cargo <path> --rustc <path> --destination <new-path> --owner-root <path>... --package <name> --bin <name> --target <triple> --profile development|release --expected-<input>-sha256 <hex> --expected-cargo-version <version> --expected-rustc-version <version> --expected-units <n> --expected-local-packages <n> --expected-registry-packages <n> --max-view-entries <n> --max-view-bytes <n>|derive --workspace <materialized-source> --vendor-root <materialized-vendor> --materialization <materialization.json> --expected-materialization-sha256 <hex> --cargo <path> --expected-cargo-sha256 <hex> --rustc <path> --expected-rustc-sha256 <hex> --package <name> --bin <name> --target <triple> --profile development|release --destination <new-path>|materialize --metadata <metadata.json> --unit-graph <unit-graph.json> --workspace <path> --vendor-root <path> --lock <Cargo.lock> --config <config.toml> --destination <new-path> --owner-root <path>... --expected-metadata-sha256 <hex> --expected-unit-graph-sha256 <hex> --expected-workspace-manifest-sha256 <hex> --expected-lock-sha256 <hex> --expected-config-sha256 <hex> --max-source-entries <n> --max-source-bytes <n> --max-vendor-entries <n> --max-vendor-bytes <n> --max-input-entries <n> --max-input-bytes <n>|finalize <materialize arguments> --assembled-materialization <record> --expected-assembled-materialization-sha256 <hex> --destination <new-record>>"
    )
}

#[cfg(test)]
mod tests;
