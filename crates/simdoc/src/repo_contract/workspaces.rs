// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Package discovery for repo contracts: the root Cargo workspace plus every
//! nested workspace the root manifest declares as part of the repository.
//!
//! A repository may keep packages under a separate resolver root, for example
//! to isolate their dependency graph from the repository tooling. Such a root
//! joins the contract only when the root manifest names it:
//!
//! ```toml
//! [workspace.metadata.sim]
//! contract-workspaces = ["crates"]
//! ```
//!
//! Undeclared nested workspaces (test fixtures, focused-test harnesses,
//! standalone tools) stay out of the contract.

use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    process::Command,
};

use serde_json::Value;

use super::*;

const CONTRACT_WORKSPACES_KEY: &str = "contract-workspaces";

#[derive(Debug, Clone)]
pub(crate) struct PackageContract {
    pub(crate) name: String,
    pub(crate) crate_name: String,
    pub(crate) manifest: String,
    pub(crate) root: String,
    pub(crate) group: String,
    pub(crate) publish: String,
    pub(crate) description: String,
    pub(crate) target_kinds: Vec<String>,
    pub(crate) targets: Vec<Value>,
    pub(crate) dependencies: Vec<Value>,
    pub(crate) source_dependencies: Vec<SourceDependency>,
    pub(crate) features: Vec<Value>,
    pub(crate) rustdoc_summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceDependency {
    pub(crate) package: String,
    pub(crate) crate_name: String,
    pub(crate) kind: String,
}

/// Reads `cargo metadata` for the root workspace and appends the members of
/// every declared contract workspace, failing on an invalid declaration or a
/// package name claimed by two workspaces.
pub(crate) fn cargo_metadata(repo: &Path) -> Result<Value, String> {
    let mut merged = workspace_metadata(&repo.join("Cargo.toml"))?;
    for declared in declared_contract_workspaces(&merged)? {
        let root = repo.join(&declared);
        let nested = workspace_metadata(&root.join("Cargo.toml"))?;
        let nested_root = nested["workspace_root"]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| format!("cargo metadata for {declared} missing workspace_root"))?;
        if nested_root.canonicalize().ok() != root.canonicalize().ok() {
            return Err(format!(
                "contract workspace {declared} is not a Cargo workspace root"
            ));
        }
        append_workspace(&mut merged, nested)?;
    }
    Ok(merged)
}

fn workspace_metadata(manifest: &Path) -> Result<Value, String> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--all-features",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .map_err(display_io)?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed for {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|err| format!("parse cargo metadata: {err}"))
}

/// Returns the repository-relative nested workspace roots declared under
/// `[workspace.metadata.sim] contract-workspaces` in the root manifest.
pub(super) fn declared_contract_workspaces(metadata: &Value) -> Result<Vec<String>, String> {
    let Some(declared) = metadata["metadata"]["sim"].get(CONTRACT_WORKSPACES_KEY) else {
        return Ok(Vec::new());
    };
    let entries = declared.as_array().ok_or_else(|| {
        format!("workspace.metadata.sim.{CONTRACT_WORKSPACES_KEY} must be an array of paths")
    })?;
    let mut seen = BTreeSet::new();
    let mut roots = Vec::new();
    for entry in entries {
        let path = entry.as_str().ok_or_else(|| {
            format!("workspace.metadata.sim.{CONTRACT_WORKSPACES_KEY} entries must be strings")
        })?;
        let relative = Path::new(path);
        if path.is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(format!(
                "contract workspace {path:?} must be a plain repository-relative directory"
            ));
        }
        if !seen.insert(path.to_owned()) {
            return Err(format!("contract workspace {path:?} is declared twice"));
        }
        roots.push(path.to_owned());
    }
    Ok(roots)
}

fn append_workspace(merged: &mut Value, nested: Value) -> Result<(), String> {
    let existing = workspace_package_names(merged)?;
    if let Some(duplicate) = workspace_package_names(&nested)?
        .intersection(&existing)
        .next()
    {
        return Err(format!(
            "package {duplicate} is a member of more than one contract workspace"
        ));
    }
    for key in ["packages", "workspace_members"] {
        let additions = nested[key]
            .as_array()
            .ok_or_else(|| format!("cargo metadata missing {key}"))?
            .clone();
        merged[key]
            .as_array_mut()
            .ok_or_else(|| format!("cargo metadata missing {key}"))?
            .extend(additions);
    }
    Ok(())
}

pub(super) fn workspace_packages(
    repo: &Path,
    metadata: &Value,
    cut: &SplitCut,
) -> Result<Vec<PackageContract>, String> {
    let member_ids = metadata["workspace_members"]
        .as_array()
        .ok_or("cargo metadata missing workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let workspace_names = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata missing packages")?
        .iter()
        .filter_map(|package| {
            let id = package["id"].as_str()?;
            member_ids
                .contains(id)
                .then(|| package["name"].as_str())
                .flatten()
        })
        .collect::<BTreeSet<_>>();

    let mut packages = Vec::new();
    for package in metadata["packages"].as_array().unwrap_or(&Vec::new()) {
        let Some(id) = package["id"].as_str() else {
            continue;
        };
        if !member_ids.contains(id) {
            continue;
        }
        let name = package["name"]
            .as_str()
            .ok_or("cargo metadata package missing name")?
            .to_owned();
        let manifest = PathBuf::from(
            package["manifest_path"]
                .as_str()
                .ok_or("cargo metadata package missing manifest_path")?,
        );
        let root = manifest
            .parent()
            .ok_or_else(|| format!("manifest has no parent: {}", manifest.display()))?;
        let group = cut
            .package_groups
            .get(&name)
            .ok_or_else(|| format!("{name} is missing from the repo contract cut"))?
            .clone();
        packages.push(PackageContract {
            crate_name: preferred_crate_name(package).unwrap_or(&name).to_owned(),
            target_kinds: target_kinds(package),
            targets: targets(repo, package)?,
            dependencies: dependencies(package, &workspace_names),
            source_dependencies: source_dependencies(package),
            features: features(package, &workspace_names),
            rustdoc_summary: docs_summary(package).unwrap_or_else(|| description(package)),
            publish: publish(package),
            description: description(package),
            manifest: rel_path(repo, &manifest)?,
            root: rel_path(repo, root)?,
            group,
            name,
        });
    }
    packages.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(packages)
}

pub(super) fn source_dependencies(package: &Value) -> Vec<SourceDependency> {
    let mut dependencies = package["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|dependency| {
            let package = dependency["name"].as_str()?;
            let kind = dependency["kind"].as_str().unwrap_or("normal");
            (kind == "normal").then(|| SourceDependency {
                package: package.to_owned(),
                crate_name: dependency["rename"]
                    .as_str()
                    .unwrap_or(package)
                    .replace('-', "_"),
                kind: kind.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    dependencies.sort_by(|left, right| {
        left.package
            .cmp(&right.package)
            .then(left.crate_name.cmp(&right.crate_name))
    });
    dependencies.dedup();
    dependencies
}

pub(super) fn workspace_package_names(metadata: &Value) -> Result<BTreeSet<String>, String> {
    let member_ids = metadata["workspace_members"]
        .as_array()
        .ok_or("cargo metadata missing workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    Ok(metadata["packages"]
        .as_array()
        .ok_or("cargo metadata missing packages")?
        .iter()
        .filter_map(|package| {
            let id = package["id"].as_str()?;
            member_ids
                .contains(id)
                .then(|| package["name"].as_str())
                .flatten()
                .map(str::to_owned)
        })
        .collect())
}

pub(super) fn validate_cut(packages: &[PackageContract], cut: &SplitCut) -> Result<(), String> {
    let workspace = packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    for package in cut.package_groups.keys() {
        if !workspace.contains(package.as_str()) {
            return Err(format!(
                "repo contract cut references non-workspace package {package}"
            ));
        }
    }
    Ok(())
}
