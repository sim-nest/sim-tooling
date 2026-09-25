// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Produces an owner-selected Cargo resolver view from one retained unit graph.

use super::{
    TreeLimit, bounded_read, canonical_directory, digest,
    graph::{
        Package, ResolverPackage, canonical_unit_graph, resolver_local_packages,
        resolver_only_package_ids, selected_package_ids, selected_packages,
    },
    manifest::{attach_resolver_package_to_workspace, narrow_workspace_manifest},
    sync_directory, write_new,
};
use serde::Serialize;
use serde_json::Value as Json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

mod checks;
mod options;
mod view;

use checks::{
    exact_file, file_digest, tool_version, validate_graph, validate_metadata, validate_narrow_lock,
};
use options::{Options, Profile, parse};
use view::{
    ViewBinding, ViewBudget, install_package_view, revalidate_view_bindings, write_view_file,
};

const MAXIMUM_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_DIAGNOSTIC_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize)]
struct SelectionReport {
    schema: &'static str,
    inputs: BTreeMap<&'static str, String>,
    cargo: ToolReport,
    rustc: ToolReport,
    selection: Selection,
    selected_local_packages: Vec<String>,
    registry_packages: Vec<String>,
    resolver_local_packages: Vec<String>,
    view_limit: TreeLimit,
    view_entries: usize,
    view_bytes: usize,
    view_bindings: Vec<ViewBinding>,
    outputs: BTreeMap<&'static str, String>,
    canonical_unit_graph_sha256: String,
}

#[derive(Serialize)]
struct ToolReport {
    path: String,
    sha256: String,
    version: String,
}

#[derive(Serialize)]
struct Selection {
    package: String,
    binary: String,
    target: String,
    profile: Profile,
    units: usize,
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let destination = options.destination.clone();
    let report = select(options)?;
    println!(
        "build-inputs: selected {} local + {} registry packages across {} units at {}",
        report.selected_local_packages.len(),
        report.registry_packages.len(),
        report.selection.units,
        destination.display()
    );
    Ok(())
}

fn select(options: Options) -> Result<SelectionReport, String> {
    let workspace = canonical_directory(&options.workspace)?;
    let vendor_root = canonical_directory(&options.vendor_root)?;
    let cargo_home = canonical_directory(&options.cargo_home)?;
    let cargo_registry = canonical_directory(&cargo_home.join("registry"))?;
    if !vendor_root.starts_with(&cargo_registry) {
        return Err("build-inputs select vendor root escaped Cargo home".into());
    }
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
    let cargo = exact_file(
        &options.cargo,
        options.expected.get("cargo").unwrap(),
        "Cargo",
    )?;
    let rustc = exact_file(
        &options.rustc,
        options.expected.get("rustc").unwrap(),
        "rustc",
    )?;
    let inputs = BTreeMap::from([
        ("metadata", digest(&metadata_bytes)),
        ("unit-graph", digest(&graph_bytes)),
        ("workspace-manifest", digest(&manifest_bytes)),
        ("lock", digest(&lock_bytes)),
        ("config", digest(&config_bytes)),
        ("cargo", file_digest(&cargo)?),
        ("rustc", file_digest(&rustc)?),
    ]);
    if inputs != options.expected {
        return Err("build-inputs select exact input digests differ".into());
    }
    let cargo_version = tool_version(&cargo, "cargo", &options.expected_cargo_version)?;
    let rustc_version = tool_version(&rustc, "rustc", &options.expected_rustc_version)?;
    let metadata: Json = serde_json::from_slice(&metadata_bytes)
        .map_err(|error| format!("retained Cargo metadata is invalid: {error}"))?;
    let graph: Json = serde_json::from_slice(&graph_bytes)
        .map_err(|error| format!("retained Cargo unit graph is invalid: {error}"))?;
    let selected = selected_package_ids(&graph)?;
    let packages = selected_packages(&metadata, &selected, &graph)?;
    let local = packages
        .iter()
        .filter(|package| !package.registry)
        .collect::<Vec<_>>();
    let registry = packages
        .iter()
        .filter(|package| package.registry)
        .collect::<Vec<_>>();
    let units = graph
        .get("units")
        .and_then(Json::as_array)
        .ok_or("unit graph has no units")?
        .len();
    if units != options.expected_units
        || local.len() != options.expected_local_packages
        || registry.len() != options.expected_registry_packages
    {
        return Err("build-inputs select retained graph cardinality differs".into());
    }
    let selected_names = local
        .iter()
        .map(|package| package.name.clone())
        .collect::<BTreeSet<_>>();
    if selected_names.len() != local.len() {
        return Err("build-inputs select local package names are ambiguous".into());
    }
    // The retained production graph, not the feature-unified root workspace,
    // owns this resolver view. Following its resolver edges gives Cargo the
    // local packages it may need without admitting every unrelated patch in
    // the constellation workspace.
    let mut resolver_ids = resolver_only_package_ids(&metadata, &selected)?;
    resolver_ids.extend(selected.iter().cloned());
    let resolver_packages = resolver_local_packages(&metadata, &resolver_ids)?;
    let resolver_by_name = resolver_packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect::<BTreeMap<_, _>>();
    if resolver_by_name.len() != resolver_packages.len() {
        return Err("selected resolver maps one package name to multiple local owners".into());
    }
    for package in &local {
        let resolver = resolver_by_name.get(package.name.as_str()).ok_or_else(|| {
            format!(
                "selected local package has no workspace patch: {}",
                package.id
            )
        })?;
        if package
            .manifest
            .canonicalize()
            .map_err(|error| error.to_string())?
            != resolver
                .manifest
                .canonicalize()
                .map_err(|error| error.to_string())?
        {
            return Err(format!(
                "selected local package owner was substituted: {}",
                package.id
            ));
        }
        validate_selected_targets(package, &owner_roots)?;
    }
    for package in &registry {
        validate_selected_targets(package, std::slice::from_ref(&vendor_root))?;
    }
    let manifest_names = resolver_packages
        .iter()
        .map(|package| package.name.clone())
        .collect::<BTreeSet<_>>();
    let narrowed = narrow_workspace_manifest(&manifest_bytes, &selected_names, &manifest_names)?;
    if options.destination.exists() {
        return Err("build-inputs select destination already exists".into());
    }
    let parent = options
        .destination
        .parent()
        .ok_or("build-inputs select destination has no parent")?;
    let parent = canonical_directory(parent)?;
    // Cargo records absolute local package paths in both metadata and the unit
    // graph. Build the private destination in place so those owner identities
    // remain valid after publication; `selection.json`, written last, is the
    // completion receipt that distinguishes a usable view from a preserved
    // interrupted attempt.
    fs::create_dir(&options.destination)
        .map_err(|error| format!("{}: {error}", options.destination.display()))?;
    let result = select_into(SelectionInputs {
        staging: &options.destination,
        workspace: &workspace,
        cargo_home: &cargo_home,
        cargo: &cargo,
        cargo_version: &cargo_version,
        rustc: &rustc,
        rustc_version: &rustc_version,
        package: &options.package,
        binary: &options.binary,
        target: &options.target,
        profile: options.profile,
        metadata: &metadata,
        graph: &graph,
        metadata_bytes: &metadata_bytes,
        graph_bytes: &graph_bytes,
        manifest: &narrowed,
        lock: &lock_bytes,
        config: &config_bytes,
        owner_roots: &owner_roots,
        local: &local,
        registry: &registry,
        resolver_packages: &resolver_packages,
        inputs: &inputs,
        limit: options.view_limit,
    });
    let report = result.map_err(|error| {
        format!(
            "{error}; incomplete selection retained at {}",
            options.destination.display()
        )
    })?;
    sync_directory(&parent)?;
    Ok(report)
}

struct SelectionInputs<'a> {
    staging: &'a Path,
    workspace: &'a Path,
    cargo_home: &'a Path,
    cargo: &'a Path,
    cargo_version: &'a str,
    rustc: &'a Path,
    rustc_version: &'a str,
    package: &'a str,
    binary: &'a str,
    target: &'a str,
    profile: Profile,
    metadata: &'a Json,
    graph: &'a Json,
    metadata_bytes: &'a [u8],
    graph_bytes: &'a [u8],
    manifest: &'a [u8],
    lock: &'a [u8],
    config: &'a [u8],
    owner_roots: &'a [PathBuf],
    local: &'a [&'a Package],
    registry: &'a [&'a Package],
    resolver_packages: &'a [ResolverPackage],
    inputs: &'a BTreeMap<&'static str, String>,
    limit: TreeLimit,
}

fn select_into(input: SelectionInputs<'_>) -> Result<SelectionReport, String> {
    let view = input.staging.join("workspace");
    fs::create_dir(&view).map_err(|error| format!("{}: {error}", view.display()))?;
    let mut budget = ViewBudget::new(input.limit);
    budget.add(input.manifest.len())?;
    write_view_file(&view.join("Cargo.toml"), input.manifest, false)?;
    budget.add(input.lock.len())?;
    write_view_file(&view.join("Cargo.lock"), input.lock, true)?;
    let packages_root = view.join("packages");
    fs::create_dir(&packages_root).map_err(|error| error.to_string())?;
    budget.add(0)?;
    let mut bindings = Vec::new();
    let selected_names = input
        .local
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    for package in input.resolver_packages {
        bindings.push(install_package_view(
            package,
            input.workspace,
            &packages_root,
            input.owner_roots,
            selected_names.contains(package.name.as_str()),
            &mut budget,
        )?);
    }
    write_new(&input.staging.join("config.toml"), input.config, false)?;
    write_new(
        &input.staging.join("retained-metadata.json"),
        input.metadata_bytes,
        false,
    )?;
    write_new(
        &input.staging.join("retained-unit-graph.json"),
        input.graph_bytes,
        false,
    )?;
    let work = input.staging.join("work");
    fs::create_dir(&work).map_err(|error| error.to_string())?;
    for path in [work.join("target"), work.join("tmp")] {
        fs::create_dir(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    let resolver = invoke(
        &input,
        &view,
        &work,
        "resolver-metadata",
        &[
            "metadata",
            "--manifest-path",
            "Cargo.toml",
            "--offline",
            "--format-version",
            "1",
            "--quiet",
        ],
    )?;
    validate_metadata(&resolver.stdout)?;
    let narrowed_lock = bounded_read(&view.join("Cargo.lock"))?;
    validate_narrow_lock(input.lock, &narrowed_lock, input.resolver_packages)?;
    let metadata = invoke(
        &input,
        &view,
        &work,
        "metadata",
        &[
            "metadata",
            "--manifest-path",
            "Cargo.toml",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--quiet",
        ],
    )?;
    validate_metadata(&metadata.stdout)?;
    let mut graph_args = vec![
        "build",
        "--manifest-path",
        "Cargo.toml",
        "--package",
        input.package,
        "--bin",
        input.binary,
        "--target",
        input.target,
        "--locked",
        "--offline",
        "-Z",
        "unstable-options",
        "--unit-graph",
    ];
    if matches!(input.profile, Profile::Release) {
        graph_args.push("--release");
    }
    let graph = invoke(&input, &view, &work, "unit-graph", &graph_args)?;
    validate_graph(&graph.stdout)?;
    let selected_metadata: Json = serde_json::from_slice(&metadata.stdout)
        .map_err(|error| format!("selected Cargo metadata is invalid: {error}"))?;
    let selected_graph: Json = serde_json::from_slice(&graph.stdout)
        .map_err(|error| format!("selected Cargo unit graph is invalid: {error}"))?;
    let retained_canonical = canonical_unit_graph(input.metadata, input.graph)?;
    let selected_canonical = canonical_unit_graph(&selected_metadata, &selected_graph)?;
    if retained_canonical != selected_canonical {
        return Err("selected Cargo unit graph differs from retained production graph".into());
    }
    let selected_ids = selected_package_ids(&selected_graph)?;
    let selected_packages = selected_packages(&selected_metadata, &selected_ids, &selected_graph)?;
    if selected_packages
        .iter()
        .filter(|package| !package.registry)
        .count()
        != input.local.len()
        || selected_packages
            .iter()
            .filter(|package| package.registry)
            .count()
            != input.registry.len()
    {
        return Err("selected Cargo package closure differs from retained graph".into());
    }
    revalidate_view_bindings(&view, &bindings)?;
    fs::set_permissions(view.join("Cargo.lock"), fs::Permissions::from_mode(0o444))
        .map_err(|error| error.to_string())?;
    fs::remove_dir_all(&work).map_err(|error| format!("{}: {error}", work.display()))?;
    let outputs = BTreeMap::from([
        ("workspace-manifest", digest(input.manifest)),
        ("lock", digest(&narrowed_lock)),
        ("config", digest(input.config)),
        ("metadata", digest(&metadata.stdout)),
        ("unit-graph", digest(&graph.stdout)),
    ]);
    write_new(
        &input.staging.join("metadata.json"),
        &metadata.stdout,
        false,
    )?;
    write_new(&input.staging.join("unit-graph.json"), &graph.stdout, false)?;
    let report = SelectionReport {
        schema: "sim.build-input-selected-view/v1",
        inputs: input.inputs.clone(),
        cargo: ToolReport {
            path: input.cargo.display().to_string(),
            sha256: input.inputs["cargo"].clone(),
            version: input.cargo_version.into(),
        },
        rustc: ToolReport {
            path: input.rustc.display().to_string(),
            sha256: input.inputs["rustc"].clone(),
            version: input.rustc_version.into(),
        },
        selection: Selection {
            package: input.package.into(),
            binary: input.binary.into(),
            target: input.target.into(),
            profile: input.profile,
            units: selected_graph["units"].as_array().unwrap().len(),
        },
        selected_local_packages: input
            .local
            .iter()
            .map(|package| package.id.clone())
            .collect(),
        registry_packages: input
            .registry
            .iter()
            .map(|package| package.id.clone())
            .collect(),
        resolver_local_packages: input
            .resolver_packages
            .iter()
            .map(|package| package.id.clone())
            .collect(),
        view_limit: input.limit,
        view_entries: budget.entries,
        view_bytes: budget.bytes,
        view_bindings: bindings,
        outputs,
        canonical_unit_graph_sha256: digest(
            &serde_json::to_vec(&selected_canonical).map_err(|error| error.to_string())?,
        ),
    };
    let report_bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    write_new(&input.staging.join("selection.json"), &report_bytes, false)?;
    sync_directory(&view)?;
    sync_directory(input.staging)?;
    Ok(report)
}

fn validate_selected_targets(package: &Package, owners: &[PathBuf]) -> Result<(), String> {
    for target in &package.targets {
        let source = target
            .source
            .canonicalize()
            .map_err(|error| format!("{}: {error}", target.source.display()))?;
        if !source.is_file() || !owners.iter().any(|owner| source.starts_with(owner)) {
            return Err(format!(
                "selected package target escaped owner roots: {}",
                package.id
            ));
        }
    }
    Ok(())
}

fn invoke(
    input: &SelectionInputs<'_>,
    view: &Path,
    work: &Path,
    role: &str,
    args: &[&str],
) -> Result<Output, String> {
    let output = Command::new(input.cargo)
        .args(args)
        .current_dir(view)
        .env_clear()
        .env("CARGO_HOME", input.cargo_home)
        .env("CARGO_TARGET_DIR", work.join("target"))
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_NET_OFFLINE", "true")
        .env("RUSTC", input.rustc)
        .env("RUSTC_BOOTSTRAP", "1")
        .env("TMPDIR", work.join("tmp"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .map_err(|error| format!("start selected Cargo {role}: {error}"))?;
    if output.stdout.len() > MAXIMUM_OUTPUT_BYTES || output.stderr.len() > MAXIMUM_DIAGNOSTIC_BYTES
    {
        return Err(format!(
            "selected Cargo {role} output exceeds its finite bound"
        ));
    }
    write_new(
        &input.staging.join(format!("{role}.stdout")),
        &output.stdout,
        false,
    )?;
    write_new(
        &input.staging.join(format!("{role}.stderr")),
        &output.stderr,
        false,
    )?;
    if !output.status.success() {
        return Err(format!(
            "selected Cargo {role} failed with {}",
            output.status
        ));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_graph_allows_only_the_owned_local_root_to_move() {
        let metadata = |root: &str| {
            json!({
                "packages": [{
                    "id": format!("path+file://{root}/packages/app#0.1.0"),
                    "name": "app",
                    "version": "0.1.0",
                    "manifest_path": format!("{root}/packages/app/Cargo.toml"),
                    "source": null
                }]
            })
        };
        let graph = |root: &str, feature: &str| {
            json!({
                "version": 1,
                "units": [{
                    "pkg_id": format!("path+file://{root}/packages/app#0.1.0"),
                    "target": {"kind":["bin"], "src_path": format!("{root}/packages/app/src/main.rs")},
                    "profile": {"name":"release"},
                    "features": [feature],
                    "dependencies": []
                }],
                "roots": [0]
            })
        };
        let first = canonical_unit_graph(&metadata("/owner"), &graph("/owner", "selected"))
            .expect("first graph");
        let moved = canonical_unit_graph(&metadata("/view"), &graph("/view", "selected"))
            .expect("moved graph");
        assert_eq!(first, moved);
        let substituted = canonical_unit_graph(&metadata("/view"), &graph("/view", "extra"))
            .expect("substituted graph");
        assert_ne!(first, substituted);
    }
}
