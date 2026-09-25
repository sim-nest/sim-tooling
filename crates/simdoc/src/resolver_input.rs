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
//! `[workspace]` and sits beside a `Cargo.lock`; anything else is refused. The
//! accepted input is recorded in provenance by its path relative to the
//! repository and the SHA-256 of its lock, never by an absolute path.

use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

use serde_json::{Value, json};

use crate::content_digest::content_digest;

/// Environment variable naming the shared resolver manifest.
pub(crate) const RESOLVER_ENV: &str = "SIMDOC_CARGO_MANIFEST_PATH";
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LOCK_BYTES: u64 = 64 * 1024 * 1024;

/// A validated shared resolver input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolverInput {
    /// Canonical path of the workspace manifest.
    pub(crate) manifest: PathBuf,
    /// SHA-256 of the workspace's `Cargo.lock`.
    pub(crate) lock_sha256: String,
}

impl ResolverInput {
    /// The provenance projection, relative to `repo`.
    pub(crate) fn projection(&self, repo: &Path) -> Result<Value, String> {
        let repo = repo
            .canonicalize()
            .map_err(|err| format!("{}: {err}", repo.display()))?;
        Ok(json!({
            "manifest": relative_path(&repo, &self.manifest)?,
            "lock_sha256": self.lock_sha256,
        }))
    }
}

/// Reads and validates the shared resolver input from the environment.
pub(crate) fn resolver_input() -> Result<Option<ResolverInput>, String> {
    match env::var_os(RESOLVER_ENV) {
        None => Ok(None),
        Some(value) => validate(Path::new(&value)).map(Some),
    }
}

/// The provenance projection of the environment's resolver input, or null.
pub(crate) fn resolver_projection(repo: &Path) -> Result<Value, String> {
    match resolver_input()? {
        Some(input) => input.projection(repo),
        None => Ok(Value::Null),
    }
}

/// Validates one resolver manifest path.
pub(crate) fn validate(path: &Path) -> Result<ResolverInput, String> {
    let refused = |why: &str| format!("{RESOLVER_ENV}={} refused: {why}", path.display());
    if path.as_os_str().is_empty() {
        return Err(refused("empty path"));
    }
    if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        return Err(refused("not a Cargo.toml"));
    }
    let metadata = fs::symlink_metadata(path).map_err(|err| refused(&err.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(refused("not an ordinary file"));
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(refused("manifest exceeds its size bound"));
    }
    let manifest = path
        .canonicalize()
        .map_err(|err| refused(&err.to_string()))?;
    let table = fs::read_to_string(&manifest)
        .map_err(|err| refused(&err.to_string()))?
        .parse::<toml::Table>()
        .map_err(|err| refused(&format!("invalid TOML: {err}")))?;
    if !table.get("workspace").is_some_and(toml::Value::is_table) {
        return Err(refused("not a workspace manifest"));
    }
    let lock = manifest.with_file_name("Cargo.lock");
    let lock_metadata = fs::symlink_metadata(&lock).map_err(|_| refused("no Cargo.lock"))?;
    if lock_metadata.file_type().is_symlink() || !lock_metadata.is_file() {
        return Err(refused("Cargo.lock is not an ordinary file"));
    }
    if lock_metadata.len() > MAX_LOCK_BYTES {
        return Err(refused("Cargo.lock exceeds its size bound"));
    }
    let lock_bytes = fs::read(&lock).map_err(|err| refused(&err.to_string()))?;
    Ok(ResolverInput {
        manifest,
        lock_sha256: content_digest(&lock_bytes),
    })
}

/// `to` relative to `from`; both must be canonical absolute paths.
pub(crate) fn relative_path(from: &Path, to: &Path) -> Result<String, String> {
    let from = normal_components(from)?;
    let to = normal_components(to)?;
    let shared = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = vec![".."; from.len() - shared];
    parts.extend(to[shared..].iter().map(String::as_str));
    Ok(parts.join("/"))
}

fn normal_components(path: &Path) -> Result<Vec<String>, String> {
    path.components()
        .filter(|component| !matches!(component, Component::RootDir))
        .map(|component| match component {
            Component::Normal(part) => part
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{} is not UTF-8", path.display())),
            _ => Err(format!("{} is not a canonical path", path.display())),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_locked_workspace_manifest_is_accepted_and_recorded_relatively() {
        let root = temp_root("resolver-accepted");
        let meta = root.join("sim-private/.meta-workspace");
        fs::create_dir_all(&meta).unwrap();
        fs::create_dir_all(root.join("sim-platform")).unwrap();
        fs::write(meta.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        fs::write(meta.join("Cargo.lock"), "version = 4\n").unwrap();

        let input = validate(&meta.join("Cargo.toml")).unwrap();
        assert_eq!(input.lock_sha256, content_digest(b"version = 4\n"));
        assert_eq!(
            input.projection(&root.join("sim-platform")).unwrap(),
            json!({
                "manifest": "../sim-private/.meta-workspace/Cargo.toml",
                "lock_sha256": content_digest(b"version = 4\n"),
            })
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn anything_but_a_locked_workspace_manifest_is_refused() {
        let root = temp_root("resolver-refused");
        let package = root.join("package");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
        fs::write(package.join("Cargo.lock"), "").unwrap();
        let unlocked = root.join("unlocked");
        fs::create_dir_all(&unlocked).unwrap();
        fs::write(unlocked.join("Cargo.toml"), "[workspace]\n").unwrap();
        let linked = root.join("linked");
        fs::create_dir_all(&linked).unwrap();
        std::os::unix::fs::symlink(unlocked.join("Cargo.toml"), linked.join("Cargo.toml")).unwrap();

        for (path, why) in [
            (PathBuf::new(), "empty path"),
            (root.join("other.toml"), "not a Cargo.toml"),
            (package.join("Cargo.toml"), "not a workspace manifest"),
            (unlocked.join("Cargo.toml"), "no Cargo.lock"),
            (linked.join("Cargo.toml"), "not an ordinary file"),
        ] {
            let err = validate(&path).unwrap_err();
            assert!(err.contains(why), "{}: {err}", path.display());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
