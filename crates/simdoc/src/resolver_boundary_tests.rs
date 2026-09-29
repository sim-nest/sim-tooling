// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Regression checks for simdoc's isolated Cargo resolver boundary.

use std::{collections::BTreeSet, path::PathBuf, process::Command};

use serde_json::Value;

const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

const DIRECT_ENCODER_IDENTITIES: &[(&str, &str)] = &[
    ("proc-macro2", "1.0.107"),
    ("quote", "1.0.47"),
    ("serde_json", "1.0.151"),
    ("sha2", "0.10.9"),
    ("sim-codec-index", "0.3.1"),
    ("sim-codec-json", "0.4.0"),
    ("sim-cookbook", "0.4.0"),
    ("sim-index-core", "0.4.0"),
    ("sim-index-vault-core", "0.2.0"),
    ("sim-kernel", "0.4.0"),
    ("sim-lib-net-core", "0.4.0"),
    ("syn", "2.0.119"),
    ("toml", "0.8.23"),
];

const UNRELATED_TOOLING_PACKAGES: &[&str] = &[
    "sim-check-pack",
    "sim-check-pack-ubuntu-pc",
    "sim-check-pack-xtask",
    "sim-codec-index-vault",
    "sim-lib-numbers-special",
    "sim-lib-numbers-stats",
    "xtask",
];

const REQUIRED_TRANSITIVE_CODEC_PACKAGES: &[&str] = &["sim-table-core"];

#[test]
fn resolver_is_locked_offline_and_contains_only_encoder_roots() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = manifest_dir.join("Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(&manifest)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .expect("run isolated locked offline Cargo metadata");
    assert!(
        output.status.success(),
        "isolated metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value = serde_json::from_slice(&output.stdout).expect("parse Cargo metadata");
    assert_eq!(
        metadata["workspace_root"].as_str(),
        manifest_dir.to_str(),
        "simdoc must retain its own workspace root"
    );

    let root_id = metadata["resolve"]["root"]
        .as_str()
        .expect("isolated metadata has resolve root");
    let root_package = metadata["packages"]
        .as_array()
        .expect("isolated metadata has packages")
        .iter()
        .find(|package| package["id"].as_str() == Some(root_id))
        .expect("isolated resolve root identifies simdoc package");
    assert_eq!(root_package["name"].as_str(), Some("simdoc"));
    assert_eq!(root_package["manifest_path"].as_str(), manifest.to_str());

    let resolved_root = metadata["resolve"]["nodes"]
        .as_array()
        .expect("isolated metadata has resolve nodes")
        .iter()
        .find(|node| node["id"].as_str() == Some(root_id))
        .expect("isolated resolve root has a node");
    let packages = metadata["packages"]
        .as_array()
        .expect("isolated metadata has packages")
        .iter()
        .filter_map(|package| {
            Some((
                package["id"].as_str()?.to_owned(),
                package["name"].as_str()?.to_owned(),
                package["version"].as_str()?.to_owned(),
                package["manifest_path"].as_str()?.to_owned(),
                package["source"].as_str().map(ToOwned::to_owned),
            ))
        })
        .map(|(id, name, version, manifest_path, source)| {
            (id, (name, version, manifest_path, source))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .expect("isolated metadata has resolve nodes")
        .iter()
        .filter_map(|node| {
            Some((
                node["id"].as_str()?.to_owned(),
                node["deps"]
                    .as_array()?
                    .iter()
                    .filter_map(|dependency| dependency["pkg"].as_str().map(ToOwned::to_owned))
                    .collect::<Vec<_>>(),
            ))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let direct_identities = resolved_root["deps"]
        .as_array()
        .expect("isolated resolve root has dependencies")
        .iter()
        .filter_map(|dependency| dependency["pkg"].as_str())
        .map(|package_id| {
            let (name, version, _, source) = &packages[package_id];
            (name.clone(), version.clone(), source.clone())
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        direct_identities,
        DIRECT_ENCODER_IDENTITIES
            .iter()
            .map(|(name, version)| {
                (
                    (*name).to_owned(),
                    (*version).to_owned(),
                    Some(REGISTRY.to_owned()),
                )
            })
            .collect(),
        "simdoc may directly resolve only its reviewed document-generation identities"
    );

    let mut reachable = BTreeSet::from([root_id.to_owned()]);
    let mut pending = vec![root_id.to_owned()];
    while let Some(package_id) = pending.pop() {
        for dependency in &nodes[&package_id] {
            if reachable.insert(dependency.clone()) {
                pending.push(dependency.clone());
            }
        }
    }
    assert_eq!(
        reachable.len(),
        packages.len(),
        "isolated metadata must contain no package outside simdoc's reachable graph"
    );

    for package_id in reachable {
        let (name, _, manifest_path, source) = &packages[&package_id];
        assert!(
            !UNRELATED_TOOLING_PACKAGES.contains(&name.as_str()),
            "unrelated tooling package resolved through simdoc: {name}"
        );
        if package_id == root_id {
            assert_eq!(
                manifest_path,
                manifest.to_str().expect("UTF-8 manifest path")
            );
            assert!(
                source.is_none(),
                "simdoc root must be the only local package"
            );
        } else {
            assert!(
                source.as_deref() == Some(REGISTRY),
                "simdoc may resolve only registry dependencies after its root; {name} has source {source:?}"
            );
        }
    }
    for package in REQUIRED_TRANSITIVE_CODEC_PACKAGES {
        assert!(
            packages.values().any(|(name, _, _, _)| name == package),
            "simdoc's reviewed codec closure must retain required transitive package {package}"
        );
    }
}
