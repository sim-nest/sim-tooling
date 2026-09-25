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

const CONTRACT_ARTIFACTS: [&str; 11] = [
    "card-index.json",
    "card-index.md",
    "feature-map.json",
    "feature-map.md",
    "provenance.json",
    "repo-contract.json",
    "repo-contract.md",
    "rustdoc-index.json",
    "rustdoc-index.md",
    "sim-index-fragment.claims.sx",
    "sim-index-fragment.sx",
];

#[test]
fn repo_contract_routes_emit_identical_bytes() {
    let repo = source_checkout_root();
    let through_xtask = scratch_dir("xtask-route");
    let direct = scratch_dir("simdoc-route");
    let emit_args = |out: &PathBuf| {
        let mut args = vec!["--repo".to_owned(), repo.to_string_lossy().into_owned()];
        for name in CONTRACT_ARTIFACTS {
            args.extend(["--emit".to_owned(), name.to_owned()]);
        }
        args.extend(["--out-dir".to_owned(), out.to_string_lossy().into_owned()]);
        args
    };

    let mut xtask_args = vec!["xtask".to_owned(), "repo-contract".to_owned()];
    xtask_args.extend(emit_args(&through_xtask));
    xtask::run(xtask_args).expect("xtask repo-contract --emit");

    let status = std::process::Command::new(env!("CARGO"))
        .args(["run", "--quiet", "--locked", "--manifest-path"])
        .arg(repo.join("crates/simdoc/Cargo.toml"))
        .args(["--", "repo-contract"])
        .args(emit_args(&direct))
        .stdout(std::process::Stdio::null())
        .status()
        .expect("run simdoc directly");
    assert!(status.success());

    for name in CONTRACT_ARTIFACTS {
        let left = fs::read(through_xtask.join(name)).unwrap();
        let right = fs::read(direct.join(name)).unwrap();
        assert!(!left.is_empty(), "{name} is empty");
        assert_eq!(left, right, "{name} differs between routes");
    }
    fs::remove_dir_all(through_xtask).unwrap();
    fs::remove_dir_all(direct).unwrap();
}

#[test]
fn xtask_compiles_no_contract_engine_source() {
    let root = source_checkout_root();
    let manifest: toml::Table = fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(dependencies) = manifest.get(table).and_then(toml::Value::as_table) else {
            continue;
        };
        for (name, spec) in dependencies {
            let path = spec.get("path").and_then(toml::Value::as_str).unwrap_or("");
            assert!(
                name != "simdoc" && !path.contains("simdoc"),
                "xtask {table} names the simdoc engine"
            );
        }
    }
    let mut pending = vec![root.join("src")];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = fs::read_to_string(&path).unwrap();
                assert!(
                    !text.contains(concat!("#[path = \"", "../crates/simdoc")),
                    "{} compiles simdoc source",
                    path.display()
                );
            }
        }
    }
}

fn scratch_dir(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}
