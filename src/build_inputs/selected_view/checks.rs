// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Identity and shape checks for `build-inputs select`: exact tool and input
//! identities, and bounded validation of what the selected Cargo produced.

use super::ResolverPackage;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn exact_file(path: &Path, expected: &str, role: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !canonical.is_file() || file_digest(&canonical)? != expected {
        return Err(format!("build-inputs select {role} identity differs"));
    }
    Ok(canonical)
}

pub(super) fn file_digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(super) fn tool_version(path: &Path, role: &str, expected: &str) -> Result<String, String> {
    if expected.is_empty() || expected.len() > 64 || expected.contains(char::is_whitespace) {
        return Err(format!(
            "build-inputs select expected {role} version is invalid"
        ));
    }
    let output = Command::new(path)
        .args(["--version", "--verbose"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .map_err(|error| format!("start selected {role}: {error}"))?;
    if !output.status.success() || output.stdout.len() > 4096 || !output.stderr.is_empty() {
        return Err(format!("selected {role} version observation failed"));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("selected {role} version is not UTF-8: {error}"))?;
    let first = text
        .lines()
        .next()
        .ok_or("selected tool version is empty")?;
    if first.split_whitespace().nth(1) != Some(expected) {
        return Err(format!("selected {role} version differs"));
    }
    Ok(first.into())
}

pub(super) fn validate_metadata(bytes: &[u8]) -> Result<(), String> {
    let value: Json = serde_json::from_slice(bytes)
        .map_err(|error| format!("selected Cargo metadata is invalid: {error}"))?;
    let packages = value.get("packages").and_then(Json::as_array);
    let nodes = value
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(Json::as_array);
    if packages.is_none_or(|rows| rows.is_empty() || rows.len() > 16_384)
        || nodes.is_none_or(|rows| rows.is_empty() || rows.len() > 16_384)
    {
        return Err("selected Cargo metadata is empty or unbounded".into());
    }
    Ok(())
}

pub(super) fn validate_graph(bytes: &[u8]) -> Result<(), String> {
    let value: Json = serde_json::from_slice(bytes)
        .map_err(|error| format!("selected Cargo unit graph is invalid: {error}"))?;
    let units = value.get("units").and_then(Json::as_array);
    if value.get("version").and_then(Json::as_u64) != Some(1)
        || units.is_none_or(|rows| rows.is_empty() || rows.len() > 16_384)
    {
        return Err("selected Cargo unit graph schema or unit count differs".into());
    }
    Ok(())
}

pub(super) fn validate_narrow_lock(
    original: &[u8],
    narrowed: &[u8],
    local: &[ResolverPackage],
) -> Result<(), String> {
    let original = lock_rows(original)?;
    let narrowed = lock_rows(narrowed)?;
    let local = local
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    for row in narrowed {
        let name = row
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or("selected Cargo.lock package name is invalid")?;
        let version = row
            .get("version")
            .and_then(toml::Value::as_str)
            .ok_or("selected Cargo.lock package version is invalid")?;
        let source = row.get("source").and_then(toml::Value::as_str);
        if source.is_none() {
            if !local.contains(name) {
                return Err(format!(
                    "selected Cargo.lock introduced local package {name}"
                ));
            }
            continue;
        }
        let checksum = row
            .get("checksum")
            .and_then(toml::Value::as_str)
            .ok_or("selected Cargo.lock registry checksum is invalid")?;
        let found = original.iter().any(|candidate| {
            candidate.get("name").and_then(toml::Value::as_str) == Some(name)
                && candidate.get("version").and_then(toml::Value::as_str) == Some(version)
                && candidate.get("source").and_then(toml::Value::as_str) == source
                && candidate.get("checksum").and_then(toml::Value::as_str) == Some(checksum)
        });
        if !found {
            return Err(format!(
                "selected Cargo.lock introduced registry package {name} {version}"
            ));
        }
    }
    Ok(())
}

fn lock_rows(bytes: &[u8]) -> Result<Vec<toml::Value>, String> {
    let root: toml::Value = toml::from_str(
        std::str::from_utf8(bytes).map_err(|error| format!("Cargo.lock is not UTF-8: {error}"))?,
    )
    .map_err(|error| format!("Cargo.lock is invalid: {error}"))?;
    root.get("package")
        .and_then(toml::Value::as_array)
        .cloned()
        .ok_or_else(|| "Cargo.lock has no packages".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowed_lock_refuses_a_registry_version_absent_from_retained_lock() {
        let original = b"version = 4\n\n[[package]]\nname = \"dep\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"aaa\"\n";
        let substituted = b"version = 4\n\n[[package]]\nname = \"dep\"\nversion = \"2.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"bbb\"\n";
        assert!(validate_narrow_lock(original, substituted, &[]).is_err());
    }
}
