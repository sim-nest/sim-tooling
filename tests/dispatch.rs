// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{fs, path::PathBuf};

#[test]
fn check_pack_reports_its_separate_checker_workspace() {
    let err = xtask::run(vec!["xtask".to_owned(), "check-pack".to_owned()])
        .expect_err("checker command must be unavailable without its feature");
    assert_eq!(
        err,
        "check-pack moved to the checker workspace; run `cargo run --manifest-path crates/Cargo.toml -p sim-check-pack-xtask -- check-pack ...`"
    );
}

#[test]
fn simdoc_reports_its_isolated_resolver_root() {
    let args = vec![
        "xtask".to_owned(),
        "simdoc".to_owned(),
        "--check".to_owned(),
    ];

    let err = xtask::run(args).expect_err("simdoc no longer runs inside xtask");
    assert_eq!(
        err,
        "simdoc moved to its isolated resolver root; run `cargo run --locked --offline --manifest-path crates/simdoc/Cargo.toml -- simdoc ...`"
    );
}

#[test]
fn generator_commands_accept_explicit_repo_root() {
    let repo = source_checkout_root().to_string_lossy().into_owned();
    for args in [
        vec![
            "xtask".to_owned(),
            "repo-contract".to_owned(),
            "--check".to_owned(),
            "--repo".to_owned(),
            repo.clone(),
        ],
        vec![
            "xtask".to_owned(),
            "validation-matrix".to_owned(),
            "--check".to_owned(),
            "--repo".to_owned(),
            repo.clone(),
        ],
        vec![
            "xtask".to_owned(),
            "crate-catalog".to_owned(),
            "--check".to_owned(),
            "--repo".to_owned(),
            repo.clone(),
        ],
    ] {
        xtask::run(args).expect("generator command should accept explicit --repo");
    }
}

fn source_checkout_root() -> PathBuf {
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest_root.join("src");
    if let Ok(target) = fs::read_link(&src) {
        let target = if target.is_absolute() {
            target
        } else {
            manifest_root.join(target)
        };
        if let Some(root) = target.parent() {
            return root.to_path_buf();
        }
    }
    manifest_root
}
