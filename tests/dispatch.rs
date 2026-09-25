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
fn every_contract_route_matches_the_engine_byte_for_byte_on_nested_packages() {
    let through_xtask = Fixture::nested("route-xtask");
    let direct = Fixture::nested("route-direct");
    let emitted_xtask = scratch_dir("route-xtask-emit");
    let emitted_direct = scratch_dir("route-direct-emit");
    let emit_args = |out: &PathBuf| {
        let mut args = Vec::new();
        for name in CONTRACT_ARTIFACTS {
            args.extend(["--emit".to_owned(), name.to_owned()]);
        }
        args.extend(["--out-dir".to_owned(), out.to_string_lossy().into_owned()]);
        args
    };

    for (command, tail) in [
        ("repo-contract", emit_args(&emitted_xtask)),
        ("validation-matrix", Vec::new()),
        ("crate-catalog", Vec::new()),
    ] {
        let mut args = vec![
            "xtask".to_owned(),
            command.to_owned(),
            "--repo".to_owned(),
            through_xtask.root.to_string_lossy().into_owned(),
        ];
        args.extend(tail);
        xtask::run(args).unwrap_or_else(|err| panic!("xtask {command}: {err}"));
    }
    for (command, tail) in [
        ("repo-contract", emit_args(&emitted_direct)),
        ("validation-matrix", Vec::new()),
        ("crate-catalog", Vec::new()),
    ] {
        let status = std::process::Command::new("cargo")
            .current_dir(source_checkout_root())
            .env_remove("RUSTUP_TOOLCHAIN")
            .args(["run", "--quiet", "--locked", "--manifest-path"])
            .arg(source_checkout_root().join("crates/simdoc/Cargo.toml"))
            .args(["--", command, "--repo"])
            .arg(&direct.root)
            .args(tail)
            .stdout(std::process::Stdio::null())
            .status()
            .expect("run simdoc directly");
        assert!(status.success(), "simdoc {command}");
    }

    for name in CONTRACT_ARTIFACTS {
        let left = fs::read(emitted_xtask.join(name)).unwrap();
        let right = fs::read(emitted_direct.join(name)).unwrap();
        assert!(!left.is_empty(), "{name} is empty");
        assert_eq!(left, right, "{name} differs between routes");
    }
    for relative in [
        "docs/generated/validation-matrix.md",
        "docs/generated/crate-catalog.json",
        "docs/generated/crate-catalog.md",
        "nested/tool/README.md",
        "nested/tool/Cargo.toml",
        "README.md",
        "Cargo.toml",
    ] {
        assert_eq!(
            through_xtask.read(relative),
            direct.read(relative),
            "{relative} differs between routes"
        );
    }
    let matrix = through_xtask.read("docs/generated/validation-matrix.md");
    assert!(
        matrix.contains("`cargo check --manifest-path nested/Cargo.toml -p tool --all-features`")
    );
    let catalog = through_xtask.read("docs/generated/crate-catalog.json");
    assert!(catalog.contains("\"nested/tool/Cargo.toml\""));
    let contract = fs::read_to_string(emitted_xtask.join("repo-contract.json")).unwrap();
    assert!(contract.contains("\"name\": \"tool\""));

    fs::remove_dir_all(emitted_xtask).unwrap();
    fs::remove_dir_all(emitted_direct).unwrap();
}

#[test]
fn xtask_carries_no_contract_generator() {
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
    let labels = [
        concat!("xtask repo-contract", " v1"),
        concat!("xtask crate-catalog", " v1"),
        concat!("xtask validation-matrix", " v1"),
        concat!("#[path = \"", "../crates/simdoc"),
    ];
    let mut pending = vec![root.join("src")];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                ![
                    "repo_contract.rs",
                    "crate_catalog.rs",
                    "validation_matrix.rs"
                ]
                .contains(&name.as_str()),
                "{} reintroduces a contract generator",
                path.display()
            );
            if path.extension().is_some_and(|ext| ext == "rs") {
                let text = fs::read_to_string(&path).unwrap();
                for label in labels {
                    assert!(
                        !text.contains(label),
                        "{} carries contract generator code ({label})",
                        path.display()
                    );
                }
            }
        }
    }
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn nested(label: &str) -> Self {
        let root = scratch_dir(label).join("sim-fixture");
        fs::create_dir_all(&root).unwrap();
        let fixture = Self { root };
        fixture.write(
            "Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             description = \"Fixture application.\"\n\n\
             [workspace]\nexclude = [\"nested\"]\n\n\
             [workspace.metadata.sim]\ncontract-workspaces = [\"nested\"]\n",
        );
        fixture.write(
            "src/lib.rs",
            "//! Fixture application.\n\npub fn app() {}\n",
        );
        fixture.write("nested/Cargo.toml", "[workspace]\nmembers = [\"tool\"]\n");
        fixture.write(
            "nested/tool/Cargo.toml",
            "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             description = \"Fixture tool.\"\npublish = false\n\n[features]\nfast = []\n",
        );
        fixture.write(
            "nested/tool/src/lib.rs",
            "//! Fixture tool.\n\npub fn tool() {}\n",
        );
        fixture.git(&["init", "--quiet"]);
        fixture.git(&[
            "remote",
            "add",
            "origin",
            "https://github.com/sim-nest/sim-fixture",
        ]);
        fixture.git(&["add", "-A"]);
        fixture.git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--no-verify",
            "-m",
            "fixture",
        ]);
        fixture
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative)).unwrap()
    }

    fn git(&self, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_AUTHOR_DATE", "2026-09-01T12:00:00+00:00")
            .env("GIT_COMMITTER_DATE", "2026-09-01T12:00:00+00:00")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(parent) = self.root.parent() {
            let _ = fs::remove_dir_all(parent);
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
