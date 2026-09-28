// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Which Cargo manifests a repository contract covers.
//!
//! The root manifest classifies every Cargo manifest its Git worktree owns. A
//! manifest is covered when it is the root manifest, a declared nested
//! workspace root, or a member of one of those workspaces. Anything else must
//! sit under a typed exclusion whose class the files themselves prove:
//!
//! ```toml
//! [workspace.metadata.sim]
//! contract-workspaces = ["crates"]
//! contract-exclusions = [
//!     { path = "crates/macros/tests/ui", class = "test-fixture", consumer = "crates/macros/tests/ui.rs", reason = "compile-fail cases" },
//! ]
//! ```
//!
//! | class | consumer | native evidence (see [`super::exclusion_witness`]) |
//! | --- | --- | --- |
//! | `test-fixture` | a `test` target source | a live `#[test]` function actually invokes a reviewed `consume_fixture(...)` helper with that fixture path, never merely holding the string as text (see [`super::fixture_consumption`]) |
//! | `recipe-fixture` | a `recipe.toml` under a covered package's `recipes/` | the file is parsed as TOML; its typed top-level `fixtures` array names the fixture path |
//! | `focused-test-harness` | one of the root manifest's `validation-commands` | that command runs the harness by `--manifest-path`, and the harness has a matching test or binary target |
//!
//! Every excluded package must also declare `publish = false`, and a
//! focused harness must path-depend on a covered package. No covered
//! package may depend on anything under an exclusion, so a real first-party
//! crate can never be excluded. The exclusions are projected into the contract.

use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use serde_json::{Value, json};

use crate::worktree::Worktree;

const CONTRACT_WORKSPACES_KEY: &str = "contract-workspaces";
const CONTRACT_EXCLUSIONS_KEY: &str = "contract-exclusions";
/// Ceiling for declared nested workspaces.
pub(crate) const MAX_CONTRACT_WORKSPACES: usize = 64;
/// Ceiling for covered packages across all workspaces.
pub(crate) const MAX_CONTRACT_PACKAGES: usize = 4096;
/// Ceiling for Cargo manifests a worktree may own.
const MAX_MANIFESTS: usize = 16_384;
/// Ceiling for one excluded manifest read as evidence.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Why a directory of Cargo manifests is outside the contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExclusionClass {
    TestFixture,
    RecipeFixture,
    FocusedTestHarness,
}

impl ExclusionClass {
    const NAMES: [(&'static str, Self); 3] = [
        ("test-fixture", Self::TestFixture),
        ("recipe-fixture", Self::RecipeFixture),
        ("focused-test-harness", Self::FocusedTestHarness),
    ];

    fn parse(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, class)| *class)
    }

    pub(crate) fn as_str(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, class)| *class == self)
            .map_or("unknown", |(name, _)| name)
    }
}

/// One `contract-exclusions` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContractExclusion {
    pub(crate) path: String,
    pub(crate) class: ExclusionClass,
    /// What consumes the fixture: a test target source for `test-fixture`,
    /// a recipe, book, or chapter manifest for `recipe-fixture`, and the
    /// `validation-commands` entry that runs it for `focused-test-harness`.
    pub(crate) consumer: Option<String>,
    pub(crate) reason: String,
}

/// An exclusion together with the manifests it removes from the contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExcludedManifests {
    pub(crate) exclusion: ContractExclusion,
    pub(crate) manifests: Vec<String>,
}

impl ExcludedManifests {
    /// The contract projection of this exclusion.
    pub(crate) fn projection(&self) -> Value {
        json!({
            "path": self.exclusion.path,
            "class": self.exclusion.class.as_str(),
            "consumer": self.exclusion.consumer,
            "reason": self.exclusion.reason,
            "manifests": self.manifests,
        })
    }
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
    if entries.len() > MAX_CONTRACT_WORKSPACES {
        return Err(format!(
            "more than {MAX_CONTRACT_WORKSPACES} contract workspaces are declared"
        ));
    }
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

/// Returns the typed `[workspace.metadata.sim] contract-exclusions` entries.
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
        let table = entry
            .as_object()
            .ok_or_else(|| format!("every {CONTRACT_EXCLUSIONS_KEY} entry must be a table"))?;
        if let Some(key) = table
            .keys()
            .find(|key| !matches!(key.as_str(), "path" | "class" | "consumer" | "reason"))
        {
            return Err(format!("contract exclusion has unexpected key `{key}`"));
        }
        let path = entry["path"].as_str().ok_or_else(|| {
            format!("every {CONTRACT_EXCLUSIONS_KEY} entry needs a string `path`")
        })?;
        if !is_plain_relative(path) {
            return Err(format!(
                "contract exclusion {path:?} must be a plain repository-relative directory"
            ));
        }
        let class_name = entry["class"].as_str().unwrap_or_default();
        let class = ExclusionClass::parse(class_name).ok_or_else(|| {
            format!(
                "contract exclusion {path:?} needs a class: one of {}",
                ExclusionClass::NAMES
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let reason = entry["reason"].as_str().map(str::trim).unwrap_or_default();
        if reason.is_empty() {
            return Err(format!(
                "contract exclusion {path:?} needs a non-empty reason"
            ));
        }
        let consumer = match (class, entry.get("consumer")) {
            (ExclusionClass::FocusedTestHarness, Some(Value::String(command)))
                if !command.trim().is_empty() && command.trim() == command =>
            {
                Some(command.clone())
            }
            (ExclusionClass::FocusedTestHarness, _) => {
                return Err(format!(
                    "focused-test-harness exclusion {path:?} needs a `consumer`: the exact \
                     `validation-commands` entry that runs the harness"
                ));
            }
            (_, Some(Value::String(consumer))) if is_plain_relative(consumer) => {
                Some(consumer.clone())
            }
            (_, _) => {
                return Err(format!(
                    "contract exclusion {path:?} needs a `consumer`: the repository-relative \
                     covered file that consumes the fixture"
                ));
            }
        };
        if !seen.insert(path.to_owned()) {
            return Err(format!("contract exclusion {path:?} is declared twice"));
        }
        exclusions.push(ContractExclusion {
            path: path.to_owned(),
            class,
            consumer,
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

/// Requires every covered package manifest and target source to be an
/// ordinary file owned by the repository's Git worktree, and returns the
/// repository-relative manifests of the covered packages.
pub(crate) fn owned_package_manifests(
    worktree: &Worktree,
    metadata: &Value,
) -> Result<BTreeSet<String>, String> {
    let mut manifests = BTreeSet::new();
    for package in member_packages(metadata)? {
        let name = package["name"].as_str().unwrap_or("<unnamed>");
        let manifest = package["manifest_path"]
            .as_str()
            .ok_or_else(|| format!("cargo metadata package {name} missing manifest_path"))?;
        manifests.insert(worktree.owned_file(Path::new(manifest), "package manifest")?);
        for target in package["targets"].as_array().into_iter().flatten() {
            let source = target["src_path"]
                .as_str()
                .ok_or_else(|| format!("cargo metadata target of {name} missing src_path"))?;
            worktree.owned_file(Path::new(source), "package target source")?;
        }
    }
    if manifests.len() > MAX_CONTRACT_PACKAGES {
        return Err(format!(
            "more than {MAX_CONTRACT_PACKAGES} contract packages; refusing an unbounded contract"
        ));
    }
    Ok(manifests)
}

/// Classifies every Cargo manifest the worktree owns: covered, or under an
/// exclusion whose class and first-party guard are proved. Returns each
/// exclusion with the manifests it removes.
pub(crate) fn classify_manifests(
    worktree: &Worktree,
    metadata: &Value,
    declared: &[String],
    exclusions: &[ContractExclusion],
) -> Result<Vec<ExcludedManifests>, String> {
    let mut covered = owned_package_manifests(worktree, metadata)?;
    covered.insert(worktree.owned_file(&worktree.root().join("Cargo.toml"), "root manifest")?);
    for root in declared {
        covered.insert(worktree.owned_file(
            &worktree.root().join(root).join("Cargo.toml"),
            "contract workspace manifest",
        )?);
    }

    let manifests = worktree.manifests();
    if manifests.len() > MAX_MANIFESTS {
        return Err(format!(
            "more than {MAX_MANIFESTS} Cargo manifests; refusing an unbounded contract"
        ));
    }
    let mut excluded = exclusions
        .iter()
        .map(|exclusion| ExcludedManifests {
            exclusion: exclusion.clone(),
            manifests: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut unclassified = Vec::new();
    for manifest in &manifests {
        let exclusion = excluded
            .iter_mut()
            .find(|entry| Path::new(manifest).starts_with(&entry.exclusion.path));
        match (covered.contains(manifest), exclusion) {
            (true, Some(entry)) => {
                return Err(format!(
                    "contract exclusion {} covers contract manifest {manifest}",
                    entry.exclusion.path
                ));
            }
            (true, None) => {}
            (false, Some(entry)) => entry.manifests.push(manifest.clone()),
            (false, None) => unclassified.push(manifest.as_str()),
        }
    }
    if !unclassified.is_empty() {
        return Err(format!(
            "unclassified Cargo manifests: {}; declare their workspace under \
             [workspace.metadata.sim] {CONTRACT_WORKSPACES_KEY} or exclude them under \
             {CONTRACT_EXCLUSIONS_KEY} with a class and a reason",
            unclassified.join(", ")
        ));
    }
    let package_roots = covered_package_roots(metadata)?;
    for entry in &excluded {
        if entry.manifests.is_empty() {
            return Err(format!(
                "contract exclusion {} matches no Cargo manifest",
                entry.exclusion.path
            ));
        }
        for manifest in &entry.manifests {
            worktree.owned_file(&worktree.root().join(manifest), "excluded manifest")?;
        }
        prove_exclusion(worktree.root(), entry, &package_roots)?;
        super::exclusion_witness::prove(worktree, metadata, entry)?;
    }
    reject_first_party_dependencies(worktree.root(), metadata, &excluded)?;
    Ok(excluded)
}

fn prove_exclusion(
    repo: &Path,
    entry: &ExcludedManifests,
    package_roots: &BTreeSet<PathBuf>,
) -> Result<(), String> {
    let exclusion = &entry.exclusion;
    let has_component = |name: &str| {
        Path::new(&exclusion.path)
            .components()
            .any(|component| component.as_os_str() == name)
    };
    match exclusion.class {
        ExclusionClass::TestFixture if !has_component("tests") => {
            return Err(format!(
                "test-fixture exclusion {} has no `tests` path component",
                exclusion.path
            ));
        }
        ExclusionClass::RecipeFixture if !has_component("recipes") => {
            return Err(format!(
                "recipe-fixture exclusion {} has no `recipes` path component",
                exclusion.path
            ));
        }
        _ => {}
    }
    for manifest in &entry.manifests {
        let path = repo.join(manifest);
        let table = read_manifest(&path)?;
        let Some(package) = table.get("package").and_then(toml::Value::as_table) else {
            continue;
        };
        if !declares_unpublished(package) {
            return Err(format!(
                "excluded package manifest {manifest} is publishable; a first-party crate \
                 cannot be excluded (fixtures declare `publish = false`)"
            ));
        }
        if exclusion.class == ExclusionClass::FocusedTestHarness
            && !path_dependency_targets(&path, &table)
                .iter()
                .any(|target| package_roots.contains(target))
        {
            return Err(format!(
                "focused-test-harness package {manifest} has no path dependency on a contract package"
            ));
        }
    }
    Ok(())
}

fn reject_first_party_dependencies(
    repo: &Path,
    metadata: &Value,
    excluded: &[ExcludedManifests],
) -> Result<(), String> {
    for package in member_packages(metadata)? {
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            let Some(path) = dependency["path"].as_str() else {
                continue;
            };
            let Ok(target) = Path::new(path).canonicalize() else {
                continue;
            };
            if let Some(entry) = excluded
                .iter()
                .find(|entry| target.starts_with(repo.join(&entry.exclusion.path)))
            {
                return Err(format!(
                    "contract package {} depends on {}, under exclusion {}; first-party code \
                     cannot be excluded",
                    package["name"].as_str().unwrap_or("<unnamed>"),
                    dependency["name"].as_str().unwrap_or("<unnamed>"),
                    entry.exclusion.path
                ));
            }
        }
    }
    Ok(())
}

fn covered_package_roots(metadata: &Value) -> Result<BTreeSet<PathBuf>, String> {
    member_packages(metadata)?
        .into_iter()
        .map(|package| {
            let manifest = package["manifest_path"]
                .as_str()
                .ok_or("cargo metadata package missing manifest_path")?;
            Path::new(manifest)
                .parent()
                .ok_or_else(|| format!("manifest has no parent: {manifest}"))?
                .canonicalize()
                .map_err(|err| format!("{manifest}: {err}"))
        })
        .collect()
}

pub(super) fn read_manifest(path: &Path) -> Result<toml::Table, String> {
    let length = fs::metadata(path)
        .map_err(|err| format!("{}: {err}", path.display()))?
        .len();
    if length > MAX_MANIFEST_BYTES {
        return Err(format!(
            "{} exceeds {MAX_MANIFEST_BYTES} bytes",
            path.display()
        ));
    }
    crate::owned::read_to_string(path)
        .map_err(|err| format!("{}: {err}", path.display()))?
        .parse::<toml::Table>()
        .map_err(|err| format!("{}: {err}", path.display()))
}

fn declares_unpublished(package: &toml::Table) -> bool {
    match package.get("publish") {
        Some(toml::Value::Boolean(false)) => true,
        Some(toml::Value::Array(registries)) => registries.is_empty(),
        _ => false,
    }
}

fn path_dependency_targets(manifest: &Path, table: &toml::Table) -> Vec<PathBuf> {
    let Some(dir) = manifest.parent() else {
        return Vec::new();
    };
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .iter()
        .filter_map(|key| table.get(*key).and_then(toml::Value::as_table))
        .flat_map(|dependencies| dependencies.values())
        .filter_map(|spec| spec.get("path").and_then(toml::Value::as_str))
        .filter_map(|path| dir.join(path).canonicalize().ok())
        .collect()
}

pub(super) fn member_packages(metadata: &Value) -> Result<Vec<&Value>, String> {
    let members = metadata["workspace_members"]
        .as_array()
        .ok_or("cargo metadata missing workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    Ok(metadata["packages"]
        .as_array()
        .ok_or("cargo metadata missing packages")?
        .iter()
        .filter(|package| {
            package["id"]
                .as_str()
                .is_some_and(|id| members.contains(id))
        })
        .collect())
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
