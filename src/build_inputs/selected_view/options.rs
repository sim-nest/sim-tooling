// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Command-line options for `build-inputs select`: every path, identity,
//! expected digest, and hard bound the selection is allowed to use.

use super::TreeLimit;
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf};

const MAXIMUM_VIEW_ENTRIES: usize = 4096;
const MAXIMUM_VIEW_BYTES: usize = 256 * 1024 * 1024;

pub(super) struct Options {
    pub(super) metadata: PathBuf,
    pub(super) unit_graph: PathBuf,
    pub(super) workspace: PathBuf,
    pub(super) lock: PathBuf,
    pub(super) config: PathBuf,
    pub(super) vendor_root: PathBuf,
    pub(super) cargo_home: PathBuf,
    pub(super) cargo: PathBuf,
    pub(super) rustc: PathBuf,
    pub(super) destination: PathBuf,
    pub(super) owner_roots: Vec<PathBuf>,
    pub(super) package: String,
    pub(super) binary: String,
    pub(super) target: String,
    pub(super) profile: Profile,
    pub(super) expected: BTreeMap<&'static str, String>,
    pub(super) expected_cargo_version: String,
    pub(super) expected_rustc_version: String,
    pub(super) expected_units: usize,
    pub(super) expected_local_packages: usize,
    pub(super) expected_registry_packages: usize,
    pub(super) view_limit: TreeLimit,
}

#[derive(Clone, Copy, Serialize)]
pub(super) enum Profile {
    Development,
    Release,
}

pub(super) fn parse(args: &[String]) -> Result<Options, String> {
    let mut values = BTreeMap::new();
    let mut owner_roots = Vec::new();
    let mut index = 3;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        if flag == "--owner-root" {
            owner_roots.push(PathBuf::from(value));
        } else if values.insert(flag, value.clone()).is_some() {
            return Err(format!("build-inputs select repeats {flag}"));
        }
        index += 2;
    }
    let required = |name: &'static str| {
        values
            .get(name)
            .cloned()
            .ok_or_else(|| format!("build-inputs select requires {name}"))
    };
    if owner_roots.is_empty() {
        return Err("build-inputs select requires at least one --owner-root".into());
    }
    let identifier = |role: &str, value: String| -> Result<String, String> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(format!("build-inputs select {role} is invalid"));
        }
        Ok(value)
    };
    let target = required("--target")?;
    if target.is_empty()
        || target.len() > 128
        || !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("build-inputs select target is invalid".into());
    }
    let profile = match required("--profile")?.as_str() {
        "development" => Profile::Development,
        "release" => Profile::Release,
        _ => return Err("build-inputs select profile is invalid".into()),
    };
    let number = |name: &'static str, hard: usize| -> Result<usize, String> {
        let value = required(name)?
            .parse::<usize>()
            .map_err(|_| format!("{name} is not a canonical positive integer"))?;
        if value == 0 || value > hard {
            return Err(format!("{name} exceeds its hard bound"));
        }
        Ok(value)
    };
    let mut expected = BTreeMap::new();
    for (key, flag) in [
        ("metadata", "--expected-metadata-sha256"),
        ("unit-graph", "--expected-unit-graph-sha256"),
        ("workspace-manifest", "--expected-workspace-manifest-sha256"),
        ("lock", "--expected-lock-sha256"),
        ("config", "--expected-config-sha256"),
        ("cargo", "--expected-cargo-sha256"),
        ("rustc", "--expected-rustc-sha256"),
    ] {
        expected.insert(key, expected_digest(&required(flag)?)?);
    }
    Ok(Options {
        metadata: PathBuf::from(required("--metadata")?),
        unit_graph: PathBuf::from(required("--unit-graph")?),
        workspace: PathBuf::from(required("--workspace")?),
        lock: PathBuf::from(required("--lock")?),
        config: PathBuf::from(required("--config")?),
        vendor_root: PathBuf::from(required("--vendor-root")?),
        cargo_home: PathBuf::from(required("--cargo-home")?),
        cargo: PathBuf::from(required("--cargo")?),
        rustc: PathBuf::from(required("--rustc")?),
        destination: PathBuf::from(required("--destination")?),
        owner_roots,
        package: identifier("package", required("--package")?)?,
        binary: identifier("binary", required("--bin")?)?,
        target,
        profile,
        expected,
        expected_cargo_version: required("--expected-cargo-version")?,
        expected_rustc_version: required("--expected-rustc-version")?,
        expected_units: number("--expected-units", 16_384)?,
        expected_local_packages: number("--expected-local-packages", 4096)?,
        expected_registry_packages: number("--expected-registry-packages", 4096)?,
        view_limit: TreeLimit {
            entries: number("--max-view-entries", MAXIMUM_VIEW_ENTRIES)?,
            bytes: number("--max-view-bytes", MAXIMUM_VIEW_BYTES)?,
        },
    })
}

fn expected_digest(value: &str) -> Result<String, String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("build-inputs select expected digest is invalid".into());
    }
    Ok(value.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_view_limits_never_exceed_native_tree_limits() {
        let args = [
            "xtask",
            "build-inputs",
            "select",
            "--metadata",
            "/metadata",
            "--unit-graph",
            "/graph",
            "--workspace",
            "/workspace",
            "--lock",
            "/lock",
            "--config",
            "/config",
            "--vendor-root",
            "/cargo/registry/src/index",
            "--cargo-home",
            "/cargo",
            "--cargo",
            "/bin/cargo",
            "--rustc",
            "/bin/rustc",
            "--destination",
            "/out",
            "--owner-root",
            "/owner",
            "--package",
            "app",
            "--bin",
            "app",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--profile",
            "release",
            "--expected-cargo-version",
            "1.96.0",
            "--expected-rustc-version",
            "1.96.0",
            "--expected-units",
            "156",
            "--expected-local-packages",
            "42",
            "--expected-registry-packages",
            "49",
            "--max-view-entries",
            "4097",
            "--max-view-bytes",
            "268435456",
            "--expected-metadata-sha256",
            &"a".repeat(64),
            "--expected-unit-graph-sha256",
            &"b".repeat(64),
            "--expected-workspace-manifest-sha256",
            &"c".repeat(64),
            "--expected-lock-sha256",
            &"d".repeat(64),
            "--expected-config-sha256",
            &"e".repeat(64),
            "--expected-cargo-sha256",
            &"f".repeat(64),
            "--expected-rustc-sha256",
            &"0".repeat(64),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        assert!(parse(&args).is_err());
    }
}
