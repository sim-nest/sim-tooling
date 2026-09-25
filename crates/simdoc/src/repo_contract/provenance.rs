// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Provenance for repo contracts: which source state the generated artifacts
//! describe, which encoder produced them, and which declared commands
//! regenerate and validate them.

use std::{fs, path::Path};

use serde_json::{Value, json};

use super::*;

pub(super) fn provenance(repo: &Path, metadata: &Value) -> Result<Value, String> {
    let preserved = preserved_provenance(repo);
    let (input_paths, workspace_hash) = measure_inputs(repo)?;
    let source_commit = preserved_source_commit(&preserved, &workspace_hash)
        .or_else(|| git_output(repo, &["rev-parse", "HEAD"]))
        .ok_or_else(|| "git rev-parse HEAD did not return a commit".to_owned())?;
    let generation_timestamp =
        generation_timestamp(repo, &preserved, &workspace_hash, &source_commit)?;
    let source_remote = public_origin(repo)?;
    let regeneration = regeneration(repo, metadata)?;
    Ok(json!({
        "schema": "sim.provenance.v1",
        "repo": repo_name(repo),
        "source_commit": source_commit,
        "source_remote": source_remote,
        "execution": execution()?,
        "regeneration_command": regeneration.regeneration_command,
        "api_docs": "target/doc/",
        "generator": GENERATOR,
        "generation_timestamp": generation_timestamp,
        "git_commit": source_commit,
        "workspace_hash": workspace_hash,
        "workspace_hash_algorithm": "fnv1a64",
        "workspace_hash_input_count": input_paths.len(),
        "workspace_hash_inputs": input_paths,
        "validation_commands": regeneration.validation_commands,
    }))
}

/// The repository-relative input paths and their stable hash: exactly what
/// the provenance workspace hash binds.
pub(super) fn measure_inputs(repo: &Path) -> Result<(Vec<String>, String), String> {
    let inputs = input_files(repo);
    let paths = inputs
        .iter()
        .map(|path| rel_path(repo, path))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((paths, stable_hash(repo, &inputs)))
}

pub(super) fn preserved_source_commit(preserved: &Value, workspace_hash: &str) -> Option<String> {
    let preserved_hash = preserved.get("workspace_hash").and_then(Value::as_str)?;
    if preserved_hash != workspace_hash {
        return None;
    }
    preserved
        .get("source_commit")
        .or_else(|| preserved.get("git_commit"))
        .and_then(Value::as_str)
        .filter(|commit| !commit.is_empty())
        .map(str::to_owned)
}

/// The generation timestamp names the source state, never the wall clock: an
/// unchanged workspace hash keeps the recorded timestamp so reruns and
/// `--check` stay byte-stable; any other state takes the committer date of the
/// source commit, so regenerated output is reproducible from that commit.
pub(super) fn generation_timestamp(
    repo: &Path,
    preserved: &Value,
    workspace_hash: &str,
    source_commit: &str,
) -> Result<String, String> {
    match preserved_generation_timestamp(preserved, workspace_hash) {
        Some(timestamp) => Ok(timestamp),
        None => commit_timestamp(repo, source_commit),
    }
}

fn preserved_generation_timestamp(preserved: &Value, workspace_hash: &str) -> Option<String> {
    let preserved_hash = preserved.get("workspace_hash").and_then(Value::as_str)?;
    if preserved_hash != workspace_hash {
        return None;
    }
    preserved
        .get("generation_timestamp")
        .and_then(Value::as_str)
        .filter(|timestamp| !timestamp.is_empty() && *timestamp != "unknown")
        .map(str::to_owned)
}

fn commit_timestamp(repo: &Path, commit: &str) -> Result<String, String> {
    if commit.is_empty() || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "source commit {commit:?} is not a hexadecimal commit id; cannot derive generation_timestamp"
        ));
    }
    git_output(repo, &["show", "-s", "--format=%cI", commit])
        .filter(|timestamp| !timestamp.is_empty())
        .ok_or_else(|| format!("git show -s --format=%cI {commit} did not return a committer date"))
}

pub(super) fn repo_name(repo: &Path) -> String {
    repo.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("REPO_NAME")
        .to_owned()
}

fn public_origin(repo: &Path) -> Result<String, String> {
    let origin = git_output(repo, &["config", "--get", "remote.origin.url"])
        .ok_or_else(|| "git remote origin url is not configured".to_owned())?;
    sanitize_origin_url(&origin)
}

pub(super) fn sanitize_origin_url(origin: &str) -> Result<String, String> {
    let ssh_github_prefix = concat!("git", "@", "github.com:");
    let mut url = if let Some(rest) = origin.strip_prefix(ssh_github_prefix) {
        format!("https://github.com/{rest}")
    } else if let Some(rest) = origin.strip_prefix("http://github.com/") {
        format!("https://github.com/{rest}")
    } else {
        origin.to_owned()
    };
    if let Some(rest) = url.strip_suffix(".git") {
        url = rest.to_owned();
    }
    if url.starts_with("https://github.com/") && !url.contains('\\') {
        Ok(url)
    } else {
        Err(format!(
            "remote origin is not a public GitHub URL: {origin}"
        ))
    }
}

fn preserved_provenance(repo: &Path) -> Value {
    let path = repo.join("docs/generated/provenance.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}
