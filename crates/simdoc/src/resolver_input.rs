// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The optional shared resolver input for the API-docs lane.
//!
//! `SIMDOC_CARGO_MANIFEST_PATH` may name a Cargo workspace manifest whose
//! resolver builds the API docs instead of the repository's own lock (the
//! pre-release route, where standalone workspaces pin unpublished siblings).
//! It is accepted only when it names an ordinary `Cargo.toml` that declares a
//! `[workspace]` and sits beside a `Cargo.lock`; anything else is refused.
//!
//! The environment is read and validated exactly once per run. The resulting
//! identity binds the manifest, the lock, the packages selected for this
//! repository, and the complete source closure of the path packages those
//! selections reach (which `Cargo.lock` does not bind); the same retained
//! identity drives cargo, keys the docs cache, and is recorded in provenance,
//! and it is measured again after use so any change refuses the run. It lives
//! outside the repository (often in a private, gitignored location), so it is
//! recorded only by content digests: never by path or directory name.
//!
//! A meta-workspace is a symlink farm: each package directory holds a
//! generated manifest and links (`src`, `recipes`, `README.md`, ...) into the
//! constellation's repository checkouts. A link is followed to its canonical
//! target and accepted only when that target lies in a Git worktree that is a
//! sibling checkout of the repository being documented, is an ordinary file
//! (or a directory of ordinary files with no link inside), and every such file
//! is tracked by that worktree (see [`crate::worktree`]). Every other entry,
//! ignored or not, refuses the input (see `package_files`), except one typed
//! exception for a nested crate's untracked `Cargo.lock`. What is digested is
//! the canonical target's content, under the link's name inside the package,
//! so the identity binds exactly what cargo reads and records no path.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{bounded_process::run_bounded, content_digest::content_digest, worktree::Worktree};

/// Environment variable naming the shared resolver manifest.
pub(crate) const RESOLVER_ENV: &str = "SIMDOC_CARGO_MANIFEST_PATH";
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LOCK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 256 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 1024 * 1024;
/// Ceiling for files in the bound path-package source closure.
pub(crate) const MAX_CLOSURE_FILES: usize = 200_000;
/// Ceiling for bytes in the bound path-package source closure.
pub(crate) const MAX_CLOSURE_BYTES: u64 = 1024 * 1024 * 1024;

/// A validated shared resolver input and everything it selects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolverInput {
    /// Canonical path of the workspace manifest; used to run cargo, never
    /// recorded.
    pub(crate) manifest: PathBuf,
    /// SHA-256 of the workspace manifest.
    pub(crate) manifest_sha256: String,
    /// SHA-256 of the workspace's `Cargo.lock`.
    pub(crate) lock_sha256: String,
    /// Workspace members documented for this repository; empty means the
    /// whole workspace.
    pub(crate) selected: Vec<String>,
    /// SHA-256 over every file of every path package the selection reaches.
    pub(crate) closure_sha256: String,
    /// Number of path packages in that closure.
    pub(crate) closure_packages: usize,
}

impl ResolverInput {
    /// The provenance projection: content digests only, no path.
    pub(crate) fn projection(&self) -> Value {
        json!({
            "kind": "shared-resolver",
            "manifest_sha256": self.manifest_sha256,
            "lock_sha256": self.lock_sha256,
            "closure_sha256": self.closure_sha256,
            "closure_packages": self.closure_packages,
        })
    }

    /// The docs-cache key: every digest the identity binds.
    pub(crate) fn cache_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.manifest_sha256,
            self.lock_sha256,
            self.closure_sha256,
            self.selected.join(",")
        )
    }

    /// Measures the input again and refuses any change since validation.
    pub(crate) fn remeasure(&self, repo: &Path) -> Result<(), String> {
        let again = validate(&self.manifest, repo)?;
        if again == *self {
            Ok(())
        } else {
            Err(format!(
                "{RESOLVER_ENV} changed while simdoc was using it; nothing was produced"
            ))
        }
    }
}

/// Reads the environment once and validates the shared resolver input for
/// `repo`, or returns `None` when it is unset.
pub(crate) fn resolver_input(repo: &Path) -> Result<Option<ResolverInput>, String> {
    match env::var_os(RESOLVER_ENV) {
        None => Ok(None),
        Some(value) => validate(Path::new(&value), repo).map(Some),
    }
}

/// Validates one resolver manifest path and binds what it selects for `repo`.
pub(crate) fn validate(path: &Path, repo: &Path) -> Result<ResolverInput, String> {
    let refused = |why: &str| format!("{RESOLVER_ENV}={} refused: {why}", path.display());
    if path.as_os_str().is_empty() {
        return Err(refused("empty path"));
    }
    if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        return Err(refused("not a Cargo.toml"));
    }
    let manifest_bytes = read_ordinary(path, MAX_MANIFEST_BYTES).map_err(|why| refused(&why))?;
    let manifest = path
        .canonicalize()
        .map_err(|err| refused(&err.to_string()))?;
    let manifest_text =
        String::from_utf8(manifest_bytes.clone()).map_err(|_| refused("manifest is not UTF-8"))?;
    let table = manifest_text
        .parse::<toml::Table>()
        .map_err(|err| refused(&format!("invalid TOML: {err}")))?;
    let Some(workspace) = table.get("workspace").and_then(toml::Value::as_table) else {
        return Err(refused("not a workspace manifest"));
    };
    let lock = manifest.with_file_name("Cargo.lock");
    if fs::symlink_metadata(&lock).is_err() {
        return Err(refused("no Cargo.lock"));
    }
    let lock_bytes = read_ordinary(&lock, MAX_LOCK_BYTES)
        .map_err(|why| refused(&format!("Cargo.lock {why}")))?;

    let members = workspace
        .get("members")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .filter_map(|member| member.rsplit('/').next().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let selected = if members.is_empty() {
        Vec::new()
    } else {
        crate::simdoc_rustdoc::repo_packages(repo)?
            .into_iter()
            .filter(|package| members.contains(package))
            .collect()
    };
    let (closure_sha256, closure_packages, selected) = source_closure(&manifest, &selected, repo)?;
    Ok(ResolverInput {
        manifest,
        manifest_sha256: content_digest(&manifest_bytes),
        lock_sha256: content_digest(&lock_bytes),
        selected,
        closure_sha256,
        closure_packages,
    })
}

fn read_ordinary(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|err| err.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("not an ordinary file".to_owned());
    }
    if metadata.len() > limit {
        return Err("exceeds its size bound".to_owned());
    }
    fs::read(path).map_err(|err| err.to_string())
}

/// Digest of every file of every path package reachable from `selected`
/// (or every workspace member when `selected` is empty) in the locked
/// resolve, and the number of those packages.
fn source_closure(
    manifest: &Path,
    selected: &[String],
    repo: &Path,
) -> Result<(String, usize, Vec<String>), String> {
    let mut boundary = Boundary::new(repo);
    let dir = manifest.parent().unwrap_or(manifest);
    let lock = fs::read_to_string(manifest.with_file_name("Cargo.lock"))
        .map_err(|err| format!("{RESOLVER_ENV} refused: Cargo.lock: {err}"))?;
    let mut command = crate::tools::tools()?.cargo_in(dir, &[manifest], Some(&lock))?;
    command
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest);
    let captured = run_bounded(
        command,
        "shared resolver cargo metadata",
        MAX_METADATA_BYTES,
        MAX_DIAGNOSTIC_BYTES,
    )?;
    if !captured.status.success() {
        return Err(format!(
            "{RESOLVER_ENV} refused: cargo metadata failed: {}",
            String::from_utf8_lossy(&captured.stderr).trim()
        ));
    }
    let metadata: Value = serde_json::from_slice(&captured.stdout)
        .map_err(|err| format!("parse shared resolver metadata: {err}"))?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("shared resolver metadata has no packages")?
        .iter()
        .filter_map(|package| Some((package["id"].as_str()?, package)))
        .collect::<BTreeMap<_, _>>();
    let members = metadata["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    // A workspace member is selected by name, but a namesake whose sources
    // link outside the documented repository (another repository's `xtask`)
    // is not this repository's package: it is neither documented nor bound.
    let mut kept = Vec::new();
    let mut pending = Vec::new();
    for id in members.iter().copied() {
        let Some(package) = packages.get(id) else {
            continue;
        };
        let name = package["name"].as_str().unwrap_or_default();
        if selected.iter().any(|wanted| wanted == name) && belongs_to_repo(package, repo)? {
            kept.push(name.to_owned());
            pending.push(id);
        }
    }
    if pending.is_empty() {
        return Err(format!(
            "{RESOLVER_ENV} refused: no workspace member is a package of the documented \
             repository"
        ));
    }
    let edges = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("shared resolver metadata has no resolve graph")?
        .iter()
        .filter_map(|node| {
            let deps = node["deps"]
                .as_array()?
                .iter()
                .filter_map(|dep| dep["pkg"].as_str())
                .collect::<Vec<_>>();
            Some((node["id"].as_str()?, deps))
        })
        .collect::<BTreeMap<_, _>>();
    let mut reached = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if reached.insert(id) {
            pending.extend(edges.get(id).into_iter().flatten().copied());
        }
    }

    let mut hasher = Sha256::new();
    let mut files = 0;
    let mut bytes = 0;
    let mut path_packages = 0;
    let mut ordered = reached
        .iter()
        .filter_map(|id| packages.get(id))
        .filter(|package| package["source"].is_null())
        .collect::<Vec<_>>();
    ordered.sort_by_key(|package| {
        (
            package["name"].as_str().unwrap_or_default().to_owned(),
            package["version"].as_str().unwrap_or_default().to_owned(),
        )
    });
    for package in ordered {
        let manifest_path = package["manifest_path"]
            .as_str()
            .ok_or("shared resolver package has no manifest_path")?;
        let root = Path::new(manifest_path)
            .parent()
            .ok_or("shared resolver package manifest has no parent")?
            .canonicalize()
            .map_err(|err| format!("shared resolver package {manifest_path}: {err}"))?;
        hasher.update(package["name"].as_str().unwrap_or_default().as_bytes());
        hasher.update([0]);
        hasher.update(package["version"].as_str().unwrap_or_default().as_bytes());
        hasher.update([0]);
        for (relative, file) in package_files(&root, &mut boundary)? {
            let content = read_ordinary(&file, crate::owned::MAX_FILE_BYTES)
                .map_err(|why| format!("shared resolver source {}: {why}", file.display()))?;
            files += 1;
            bytes += content.len() as u64;
            if files > MAX_CLOSURE_FILES || bytes > MAX_CLOSURE_BYTES {
                return Err(format!(
                    "{RESOLVER_ENV} refused: the selected source closure exceeds \
                     {MAX_CLOSURE_FILES} files or {MAX_CLOSURE_BYTES} bytes"
                ));
            }
            hasher.update(relative.as_bytes());
            hasher.update([0]);
            hasher.update((content.len() as u64).to_le_bytes());
            hasher.update(&content);
        }
        path_packages += 1;
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((digest, path_packages, kept))
}

/// Whether a resolver package is this repository's. A package of the
/// repository is a generated `Cargo.toml` plus links whose every target is
/// inside the repository: no other real file or directory (a `build.rs`, a
/// `.cargo`, a `src` of its own) may sit beside them, or the farm, not the
/// repository, would supply the package's source. A package whose links all
/// lead elsewhere is another repository's namesake. One whose links lead both
/// ways is refused, and so is one with no links at all.
fn belongs_to_repo(package: &Value, repo: &Path) -> Result<bool, String> {
    let manifest = package["manifest_path"]
        .as_str()
        .ok_or("shared resolver package has no manifest_path")?;
    let dir = Path::new(manifest)
        .parent()
        .ok_or("shared resolver package manifest has no parent")?;
    let repo = repo
        .canonicalize()
        .map_err(|err| format!("{}: {err}", repo.display()))?;
    let (mut inside, mut outside) = (0, 0);
    for entry in fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("{}: {err}", dir.display()))?;
        let kind = entry
            .file_type()
            .map_err(|err| format!("{}: {err}", entry.path().display()))?;
        if !(kind.is_symlink() || kind.is_file() && entry.file_name() == "Cargo.toml") {
            return Err(format!(
                "{RESOLVER_ENV} refused: package {} has its own {} beside its links; a package \
                 of the repository is its Cargo.toml and links into the repository only",
                package["name"].as_str().unwrap_or("<unnamed>"),
                entry.file_name().to_string_lossy()
            ));
        }
        if kind.is_symlink() {
            let target = entry
                .path()
                .canonicalize()
                .map_err(|err| format!("{}: {err}", entry.path().display()))?;
            if target.starts_with(&repo) {
                inside += 1;
            } else {
                outside += 1;
            }
        }
    }
    if inside == 0 && outside == 0 {
        return Err(format!(
            "{RESOLVER_ENV} refused: package {} has no links into the documented repository; \
             a package of the repository is its Cargo.toml and links into the repository",
            package["name"].as_str().unwrap_or("<unnamed>")
        ));
    }
    if inside > 0 && outside > 0 {
        return Err(format!(
            "{RESOLVER_ENV} refused: package {} links both into and out of the documented \
             repository",
            package["name"].as_str().unwrap_or("<unnamed>")
        ));
    }
    Ok(outside == 0)
}

/// The canonical targets a resolver package's links may lead to: files of
/// the Git worktrees that are sibling checkouts of the documented repository.
struct Boundary {
    /// The directory holding the constellation's checkouts.
    checkouts: Option<PathBuf>,
    worktrees: BTreeMap<PathBuf, Worktree>,
}

impl Boundary {
    fn new(repo: &Path) -> Self {
        Self {
            checkouts: repo.parent().map(Path::to_path_buf),
            worktrees: BTreeMap::new(),
        }
    }

    /// The worktree owning the canonical `target`, opened once.
    fn worktree(&mut self, target: &Path) -> Result<&Worktree, String> {
        let refused = |why: &str| format!("{RESOLVER_ENV} refused: link target {why}");
        let checkouts = self
            .checkouts
            .as_deref()
            .ok_or_else(|| refused("has no constellation to lie in"))?;
        let name = target
            .strip_prefix(checkouts)
            .ok()
            .and_then(|rest| rest.components().next())
            .filter(|part| matches!(part, std::path::Component::Normal(_)))
            .ok_or_else(|| refused("lies outside the constellation's checkouts"))?;
        let top = checkouts.join(name);
        if !top.join(".git").exists() {
            return Err(refused("is not inside a Git checkout"));
        }
        if !self.worktrees.contains_key(&top) {
            self.worktrees.insert(top.clone(), Worktree::open(&top)?);
        }
        Ok(&self.worktrees[&top])
    }
}

/// Every ordinary file a path package's cargo build can read, in path order,
/// as `(name inside the package, file to read)`: the package's own files, and
/// the canonical files behind each link (see the module documentation).
///
/// Deny by default: every entry beneath the package, and beneath every
/// followed link target, is either bound or refuses the input. That includes
/// ignored files, untracked files, build-output directories, and links
/// nested inside a followed tree; nothing is skipped, because Cargo, rustc,
/// build scripts, and proc macros can read any of it. The one typed
/// exception is [`is_nested_lockfile`]: bound, but not required to be tracked.
fn package_files(root: &Path, boundary: &mut Boundary) -> Result<Vec<(String, PathBuf)>, String> {
    let mut files = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(|err| format!("{}: {err}", dir.display()))? {
            let entry = entry.map_err(|err| format!("{}: {err}", dir.display()))?;
            let kind = entry
                .file_type()
                .map_err(|err| format!("{}: {err}", entry.path().display()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = format!("{prefix}{name}");
            if kind.is_symlink() {
                let canonical = entry.path().canonicalize().map_err(|err| {
                    format!(
                        "{RESOLVER_ENV} refused: link {}: {err}",
                        entry.path().display()
                    )
                })?;
                let worktree = boundary.worktree(&canonical)?;
                if canonical.is_dir() {
                    owned_tree(worktree, &canonical, &format!("{relative}/"), &mut files)?;
                } else {
                    worktree
                        .owned_file(&canonical, "resolver link target")
                        .map_err(|why| format!("{RESOLVER_ENV} refused: {why}"))?;
                    files.push((relative, canonical));
                }
            } else if kind.is_dir() {
                pending.push((entry.path(), format!("{relative}/")));
            } else if kind.is_file() {
                files.push((relative, entry.path()));
            } else {
                return Err(format!(
                    "{RESOLVER_ENV} refused: {} is not an ordinary file",
                    entry.path().display()
                ));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Every file beneath the canonical directory `dir`, each of which must be an
/// ordinary file `worktree` tracks, reached through no further link. Nothing
/// else is skipped, ignored or not.
fn owned_tree(
    worktree: &Worktree,
    dir: &Path,
    prefix: &str,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("{}: {err}", dir.display()))?;
        let kind = entry
            .file_type()
            .map_err(|err| format!("{}: {err}", entry.path().display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_dir() {
            owned_tree(worktree, &entry.path(), &format!("{prefix}{name}/"), files)?;
        } else if let Err(why) = worktree.owned_file(&entry.path(), "resolver source") {
            if !is_nested_lockfile(worktree, &entry.path()) {
                return Err(format!("{RESOLVER_ENV} refused: {why}"));
            }
            // The exempt lockfile need not be tracked, but its bytes are bound.
            files.push((format!("{prefix}{name}"), entry.path()));
        } else {
            files.push((format!("{prefix}{name}"), entry.path()));
        }
    }
    Ok(())
}

/// The single typed exception to "every entry is bound": an untracked
/// `Cargo.lock` directly beside a tracked `Cargo.toml` inside a followed
/// tree. It is the resolver output of a nested crate (a focused-test crate
/// that gitignores its own lock); the enclosing package's build neither reads
/// it nor is built from it, so it need not be tracked (its bytes are bound).
fn is_nested_lockfile(worktree: &Worktree, file: &Path) -> bool {
    file.file_name().is_some_and(|name| name == "Cargo.lock")
        && worktree
            .owned_file(&file.with_file_name("Cargo.toml"), "nested manifest")
            .is_ok()
}

#[cfg(test)]
#[path = "resolver_input_tests.rs"]
mod tests;
