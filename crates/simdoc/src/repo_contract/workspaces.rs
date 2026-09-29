// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Package discovery for repo contracts: the root Cargo workspace plus every
//! nested workspace the root manifest declares as part of the repository,
//! checked against the classification and containment rules in
//! `workspace_policy`.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::workspace_policy::{
    ExcludedManifests, MAX_CONTRACT_PACKAGES, classify_manifests, contract_exclusions,
    declared_contract_workspaces, reject_symlinked_path,
};
use super::*;
use crate::bounded_process::run_bounded;
use crate::worktree::Worktree;

/// Ceiling for one `cargo metadata --no-deps` document as read.
const MAX_METADATA_BYTES: usize = 64 * 1024 * 1024;
/// Ceiling for the projected metadata retained across every workspace.
const MAX_RETAINED_METADATA_BYTES: usize = 64 * 1024 * 1024;
/// Ceiling for Cargo diagnostics.
const MAX_DIAGNOSTIC_BYTES: usize = 1024 * 1024;

/// The packages a repository contract covers.
#[derive(Debug)]
pub(crate) struct ContractPackages {
    root: PathBuf,
    /// Merged `cargo metadata` of the root and every declared workspace.
    pub(crate) metadata: Value,
    /// Repository-relative roots of the declared nested workspaces.
    pub(crate) declared: Vec<String>,
    /// Proved exclusions with the manifests each removes.
    pub(crate) exclusions: Vec<ExcludedManifests>,
}

impl ContractPackages {
    /// The declared nested workspace that owns `package`, or `None` for a
    /// member of the root workspace.
    pub(crate) fn workspace_of(&self, package: &Value) -> Option<&str> {
        let manifest = Path::new(package["manifest_path"].as_str()?);
        let relative = manifest
            .canonicalize()
            .ok()?
            .strip_prefix(&self.root)
            .ok()?
            .to_path_buf();
        self.declared
            .iter()
            .filter(|root| relative.starts_with(root.as_str()))
            .max_by_key(|root| root.len())
            .map(String::as_str)
    }
}

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
/// every declared contract workspace, then classifies every Cargo manifest
/// the Git worktree owns. Fails on an invalid declaration, a package claimed
/// by two workspaces, a manifest or target source the worktree does not own
/// as an ordinary file, an unclassified manifest, an unproved exclusion, or
/// output beyond its ceilings.
pub(crate) fn contract_packages(repo: &Path) -> Result<ContractPackages, String> {
    let worktree = Worktree::open(repo)?;
    let repo = worktree.root().to_path_buf();
    let mut retained = 0;
    let mut merged = workspace_metadata(&repo.join("Cargo.toml"), &mut retained)?;
    if workspace_root(&merged)? != repo {
        return Err(format!(
            "{} is not the root of its Cargo workspace",
            repo.display()
        ));
    }
    let declared = declared_contract_workspaces(&merged)?;
    let exclusions = contract_exclusions(&merged)?;
    for relative in &declared {
        reject_symlinked_path(&repo, relative)?;
        let root = repo.join(relative);
        worktree.owned_file(&root.join("Cargo.toml"), "contract workspace manifest")?;
        let nested = workspace_metadata(&root.join("Cargo.toml"), &mut retained)?;
        if workspace_root(&nested)? != root {
            return Err(format!(
                "contract workspace {relative} is not a Cargo workspace root"
            ));
        }
        append_workspace(&mut merged, nested)?;
        if merged["packages"].as_array().map_or(0, Vec::len) > MAX_CONTRACT_PACKAGES {
            return Err(format!(
                "more than {MAX_CONTRACT_PACKAGES} contract packages; refusing an unbounded contract"
            ));
        }
    }
    let exclusions = classify_manifests(&worktree, &merged, &declared, &exclusions)?;
    Ok(ContractPackages {
        root: repo,
        metadata: merged,
        declared,
        exclusions,
    })
}

/// The merged `cargo metadata` of every covered workspace.
pub(crate) fn cargo_metadata(repo: &Path) -> Result<Value, String> {
    Ok(contract_packages(repo)?.metadata)
}

fn workspace_root(metadata: &Value) -> Result<PathBuf, String> {
    let root = metadata["workspace_root"]
        .as_str()
        .ok_or("cargo metadata missing workspace_root")?;
    Path::new(root)
        .canonicalize()
        .map_err(|err| format!("workspace root {root}: {err}"))
}

/// Reads one workspace's `cargo metadata --no-deps` and retains only its
/// bounded projection, charging it against `retained`.
fn workspace_metadata(manifest: &Path, retained: &mut usize) -> Result<Value, String> {
    let dir = manifest.parent().unwrap_or(manifest);
    let mut command = crate::tools::tools()?.cargo_in(dir, &[manifest], None)?;
    command
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
        .arg(manifest);
    let captured = run_bounded(
        command,
        "cargo metadata",
        MAX_METADATA_BYTES,
        MAX_DIAGNOSTIC_BYTES,
    )?;
    if !captured.status.success() {
        return Err(format!(
            "cargo metadata failed for {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&captured.stderr)
        ));
    }
    let document: Value = serde_json::from_slice(&captured.stdout)
        .map_err(|err| format!("parse cargo metadata: {err}"))?;
    drop(captured);
    let projected = project_metadata(&document);
    *retained += serde_json::to_vec(&projected)
        .map_err(|err| format!("measure cargo metadata: {err}"))?
        .len();
    if *retained > MAX_RETAINED_METADATA_BYTES {
        return Err(format!(
            "projected cargo metadata exceeds {MAX_RETAINED_METADATA_BYTES} bytes across the \
             contract workspaces; refusing an unbounded contract"
        ));
    }
    Ok(projected)
}

/// The fields of `cargo metadata` the contract engine reads, and nothing
/// else: the workspace root and members, the `sim` workspace metadata, and
/// per package its identity, manifest, description, publish setting,
/// features, targets, and dependencies.
pub(crate) fn project_metadata(document: &Value) -> Value {
    let pick = |value: &Value, keys: &[&str]| {
        Value::Object(
            keys.iter()
                .filter_map(|key| {
                    value
                        .get(*key)
                        .map(|field| ((*key).to_owned(), field.clone()))
                })
                .collect(),
        )
    };
    let packages = document["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|package| {
            let mut projected = pick(
                package,
                &[
                    "id",
                    "name",
                    "version",
                    "source",
                    "manifest_path",
                    "description",
                    "publish",
                    "features",
                ],
            );
            projected["targets"] = Value::Array(
                package["targets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|target| {
                        pick(
                            target,
                            &[
                                "name",
                                "kind",
                                "crate_types",
                                "src_path",
                                "test",
                                "required-features",
                            ],
                        )
                    })
                    .collect(),
            );
            projected["dependencies"] = Value::Array(
                package["dependencies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|dependency| {
                        pick(
                            dependency,
                            &[
                                "name", "source", "kind", "optional", "rename", "target", "path",
                            ],
                        )
                    })
                    .collect(),
            );
            projected
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "packages": packages,
        "workspace_members": document["workspace_members"],
        "workspace_root": document["workspace_root"],
        "metadata": {"sim": document["metadata"]["sim"]},
    })
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

pub(crate) fn workspace_package_names(metadata: &Value) -> Result<BTreeSet<String>, String> {
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
