// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which Cargo manifests a repository contract covers, and the containment
//! rules every covered package must satisfy.
//!
//! The root manifest classifies every git-visible `Cargo.toml` in the
//! repository. A manifest is covered when it is the root manifest, a declared
//! nested workspace root, or a member of one of those workspaces; otherwise it
//! must sit under an explicit exclusion that states why:
//!
//! ```toml
//! [workspace.metadata.sim]
//! contract-workspaces = ["crates"]
//! contract-exclusions = [
//!     { path = "tests/ui", reason = "compile-fail fixtures, not packages" },
//! ]
//! ```
//!
//! Covered packages, their manifests, and their target sources must resolve
//! beneath the canonical repository root, and no declared workspace may be
//! reached through a symlink, so every fact in the contract is bound by the
//! repository's own workspace hash.

use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use serde_json::Value;

const CONTRACT_WORKSPACES_KEY: &str = "contract-workspaces";
const CONTRACT_EXCLUSIONS_KEY: &str = "contract-exclusions";

/// One `contract-exclusions` entry: a repository-relative directory whose
/// Cargo manifests are deliberately outside the contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContractExclusion {
    pub(crate) path: String,
    pub(crate) reason: String,
}

/// Returns the repository-relative nested workspace roots declared under
/// `[workspace.metadata.sim] contract-workspaces` in the root manifest.
pub(crate) fn declared_contract_workspaces(metadata: &Value) -> Result<Vec<String>, String> {
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
        if !is_plain_relative(path) {
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

/// Returns the `[workspace.metadata.sim] contract-exclusions` entries.
pub(crate) fn contract_exclusions(metadata: &Value) -> Result<Vec<ContractExclusion>, String> {
    let Some(declared) = metadata["metadata"]["sim"].get(CONTRACT_EXCLUSIONS_KEY) else {
        return Ok(Vec::new());
    };
    let entries = declared.as_array().ok_or_else(|| {
        format!("workspace.metadata.sim.{CONTRACT_EXCLUSIONS_KEY} must be an array of tables")
    })?;
    let mut seen = BTreeSet::new();
    let mut exclusions = Vec::new();
    for entry in entries {
        let path = entry["path"].as_str().ok_or_else(|| {
            format!("every {CONTRACT_EXCLUSIONS_KEY} entry needs a string `path`")
        })?;
        let reason = entry["reason"].as_str().map(str::trim).unwrap_or_default();
        if !is_plain_relative(path) {
            return Err(format!(
                "contract exclusion {path:?} must be a plain repository-relative directory"
            ));
        }
        if reason.is_empty() {
            return Err(format!(
                "contract exclusion {path:?} needs a non-empty reason"
            ));
        }
        if !seen.insert(path.to_owned()) {
            return Err(format!("contract exclusion {path:?} is declared twice"));
        }
        exclusions.push(ContractExclusion {
            path: path.to_owned(),
            reason: reason.to_owned(),
        });
    }
    Ok(exclusions)
}

/// Refuses a declared workspace reached through a symlink anywhere below the
/// repository root.
pub(crate) fn reject_symlinked_path(repo: &Path, relative: &str) -> Result<(), String> {
    let mut path = repo.to_path_buf();
    for component in Path::new(relative).components() {
        path.push(component);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| format!("contract workspace {relative}: {err}"))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "contract workspace {relative} is reached through a symlink"
            ));
        }
    }
    Ok(())
}

/// Requires `path` to resolve beneath the canonical repository root.
pub(crate) fn ensure_within(repo: &Path, path: &Path, role: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|err| format!("{role} {}: {err}", path.display()))?;
    if !canonical.starts_with(repo) {
        return Err(format!(
            "{role} {} resolves outside the repository",
            path.display()
        ));
    }
    Ok(canonical)
}

/// Requires every covered package manifest and target source to resolve
/// beneath the canonical repository root.
pub(crate) fn ensure_packages_within(repo: &Path, metadata: &Value) -> Result<(), String> {
    let members = member_ids(metadata)?;
    for package in metadata["packages"]
        .as_array()
        .ok_or("cargo metadata missing packages")?
    {
        if !package["id"]
            .as_str()
            .is_some_and(|id| members.contains(id))
        {
            continue;
        }
        let name = package["name"].as_str().unwrap_or("<unnamed>");
        let manifest = package["manifest_path"]
            .as_str()
            .ok_or_else(|| format!("cargo metadata package {name} missing manifest_path"))?;
        ensure_within(repo, Path::new(manifest), "package manifest")?;
        for target in package["targets"].as_array().into_iter().flatten() {
            let source = target["src_path"]
                .as_str()
                .ok_or_else(|| format!("cargo metadata target of {name} missing src_path"))?;
            ensure_within(repo, Path::new(source), "package target source")?;
        }
    }
    Ok(())
}

/// Requires every git-visible `Cargo.toml` to be covered by the contract or
/// excluded with a reason, and every exclusion to be live and disjoint from
/// the covered manifests.
pub(crate) fn ensure_manifests_classified(
    repo: &Path,
    metadata: &Value,
    declared: &[String],
    exclusions: &[ContractExclusion],
) -> Result<(), String> {
    let mut covered = BTreeSet::from(["Cargo.toml".to_owned()]);
    covered.extend(declared.iter().map(|root| format!("{root}/Cargo.toml")));
    let members = member_ids(metadata)?;
    for package in metadata["packages"]
        .as_array()
        .ok_or("cargo metadata missing packages")?
    {
        if !package["id"]
            .as_str()
            .is_some_and(|id| members.contains(id))
        {
            continue;
        }
        let manifest = package["manifest_path"]
            .as_str()
            .ok_or("cargo metadata package missing manifest_path")?;
        let canonical = Path::new(manifest)
            .canonicalize()
            .map_err(|err| format!("package manifest {manifest}: {err}"))?;
        covered.insert(slash_relative(repo, &canonical)?);
    }

    let manifests = visible_manifests(repo)?;
    let mut unclassified = Vec::new();
    let mut live = BTreeSet::new();
    for manifest in &manifests {
        let exclusion = exclusions
            .iter()
            .find(|exclusion| Path::new(manifest).starts_with(&exclusion.path));
        match (covered.contains(manifest), exclusion) {
            (true, Some(exclusion)) => {
                return Err(format!(
                    "contract exclusion {} covers contract manifest {manifest}",
                    exclusion.path
                ));
            }
            (true, None) => {}
            (false, Some(exclusion)) => {
                live.insert(exclusion.path.as_str());
            }
            (false, None) => unclassified.push(manifest.as_str()),
        }
    }
    if !unclassified.is_empty() {
        return Err(format!(
            "unclassified Cargo manifests: {}; declare their workspace under \
             [workspace.metadata.sim] {CONTRACT_WORKSPACES_KEY} or exclude them under \
             {CONTRACT_EXCLUSIONS_KEY} with a reason",
            unclassified.join(", ")
        ));
    }
    if let Some(stale) = exclusions
        .iter()
        .find(|exclusion| !live.contains(exclusion.path.as_str()))
    {
        return Err(format!(
            "contract exclusion {} matches no Cargo manifest",
            stale.path
        ));
    }
    Ok(())
}

fn visible_manifests(repo: &Path) -> Result<BTreeSet<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .map_err(|err| format!("git ls-files: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files failed in {}; cannot classify Cargo manifests",
            repo.display()
        ));
    }
    let listing = String::from_utf8(output.stdout)
        .map_err(|_| "git ls-files returned a non-UTF-8 path".to_owned())?;
    Ok(listing
        .split('\0')
        .filter(|path| {
            Path::new(path)
                .file_name()
                .is_some_and(|name| name == "Cargo.toml")
        })
        .filter(|path| repo.join(path).is_file())
        .map(str::to_owned)
        .collect())
}

fn member_ids(metadata: &Value) -> Result<BTreeSet<&str>, String> {
    Ok(metadata["workspace_members"]
        .as_array()
        .ok_or("cargo metadata missing workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect())
}

fn slash_relative(repo: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(repo)
        .map_err(|_| format!("{} resolves outside the repository", path.display()))?;
    relative
        .to_str()
        .map(|text| text.replace(std::path::MAIN_SEPARATOR, "/"))
        .ok_or_else(|| format!("{} is not UTF-8", relative.display()))
}

fn is_plain_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
#[path = "workspace_policy_tests.rs"]
mod tests;
