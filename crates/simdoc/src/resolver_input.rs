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
//! accepted input is recorded in provenance only by content: the SHA-256 of
//! its manifest and of its lock. It lives outside the repository (often in a
//! private, gitignored location), so neither its path nor any directory name
//! is ever recorded.

use std::{
    env, fs,
    path::{Path, PathBuf},
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
    /// Canonical path of the workspace manifest; used to run cargo, never
    /// recorded.
    pub(crate) manifest: PathBuf,
    /// SHA-256 of the workspace manifest.
    pub(crate) manifest_sha256: String,
    /// SHA-256 of the workspace's `Cargo.lock`.
    pub(crate) lock_sha256: String,
}

impl ResolverInput {
    /// The provenance projection: content digests only, no path.
    pub(crate) fn projection(&self) -> Value {
        json!({
            "kind": "shared-resolver",
            "manifest_sha256": self.manifest_sha256,
            "lock_sha256": self.lock_sha256,
        })
    }
}

/// Reads and validates the shared resolver input from the environment.
pub(crate) fn resolver_input() -> Result<Option<ResolverInput>, String> {
    match env::var_os(RESOLVER_ENV) {
        None => Ok(None),
        Some(value) => validate(Path::new(&value)).map(Some),
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
    let manifest_bytes = fs::read(&manifest).map_err(|err| refused(&err.to_string()))?;
    let table = String::from_utf8(manifest_bytes.clone())
        .map_err(|_| refused("manifest is not UTF-8"))?
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
        manifest_sha256: content_digest(&manifest_bytes),
        lock_sha256: content_digest(&lock_bytes),
    })
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
    fn a_resolver_outside_the_repository_is_recorded_by_content_only() {
        let root = temp_root("resolver-accepted");
        let meta = root.join("sim-private/.meta-workspace");
        fs::create_dir_all(&meta).unwrap();
        fs::write(meta.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        fs::write(meta.join("Cargo.lock"), "version = 4\n").unwrap();

        let input = validate(&meta.join("Cargo.toml")).unwrap();
        let projection = input.projection();
        assert_eq!(
            projection,
            json!({
                "kind": "shared-resolver",
                "manifest_sha256": content_digest(b"[workspace]\nmembers = []\n"),
                "lock_sha256": content_digest(b"version = 4\n"),
            })
        );
        let text = projection.to_string();
        for leak in ["/", "sim-private", "meta-workspace", "Cargo.toml", ".."] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
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
