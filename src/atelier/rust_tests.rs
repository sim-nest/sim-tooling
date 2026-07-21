use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::json;

use super::{
    index_manifest::{RepoEntry, RepoStatus},
    index_units::SourceUnit,
    rust::{build_rust_intelligence, remap_meta_workspace_path},
};

#[test]
fn ide_object_ids_survive_item_renames() {
    let fixture = RustFixture::new("rename");
    fixture.write_cargo_at("repo", "sim-alpha", &[]);
    let repo = fixture.repo("sim-alpha-repo", "repo", &["sim-alpha"], &["."]);

    let first = source_unit(
        "one",
        "sim-alpha-repo",
        "sim-alpha",
        "src/lib.rs",
        7,
        "alpha",
    );
    let second = source_unit(
        "two",
        "sim-alpha-repo",
        "sim-alpha",
        "src/lib.rs",
        7,
        "beta",
    );
    let first_index = build_rust_intelligence(std::slice::from_ref(&repo), &[first]);
    let second_index = build_rust_intelligence(&[repo], &[second]);

    assert_eq!(
        first_index.fact_for_unit("one").unwrap().json()["ide_object_id"],
        second_index.fact_for_unit("two").unwrap().json()["ide_object_id"]
    );
}

#[test]
fn duplicate_crate_names_are_repo_qualified() {
    let fixture = RustFixture::new("duplicate");
    fixture.write_cargo_at("left", "sim-shared", &[]);
    fixture.write_cargo_at("right", "sim-shared", &[]);
    let repos = [
        fixture.repo("left-repo", "left", &["sim-shared"], &["."]),
        fixture.repo("right-repo", "right", &["sim-shared"], &["."]),
    ];
    let units = [
        source_unit("left", "left-repo", "sim-shared", "src/lib.rs", 3, "alpha"),
        source_unit(
            "right",
            "right-repo",
            "sim-shared",
            "src/lib.rs",
            3,
            "alpha",
        ),
    ];

    let index = build_rust_intelligence(&repos, &units);
    let left = index.fact_for_unit("left").unwrap().json();
    let right = index.fact_for_unit("right").unwrap().json();

    assert_ne!(left["ide_object_id"], right["ide_object_id"]);
    assert_eq!(left["repo"], "left-repo");
    assert_eq!(right["repo"], "right-repo");
}

#[test]
fn rustdoc_json_supplies_feature_gates_and_docs() {
    let fixture = RustFixture::new("rustdoc");
    fixture.write_cargo_at("repo", "sim-alpha", &["extra"]);
    fixture.write_rustdoc_json(
        "repo",
        "sim_alpha",
        json!({
            "index": {
                "0:0": {
                    "name": "alpha",
                    "attrs": ["#[cfg(feature = \"extra\")]"],
                    "docs": "Alpha docs from rustdoc.",
                    "span": {
                        "filename": "src/lib.rs",
                        "begin": [9, 1]
                    },
                    "inner": {
                        "function": {}
                    }
                }
            }
        }),
    );
    let repo = fixture.repo("sim-alpha-repo", "repo", &["sim-alpha"], &["."]);
    let unit = source_unit(
        "alpha",
        "sim-alpha-repo",
        "sim-alpha",
        "src/lib.rs",
        7,
        "alpha",
    );

    let index = build_rust_intelligence(&[repo], &[unit]);
    let fact = index.fact_for_unit("alpha").unwrap().json();

    assert_eq!(fact["docs_summary"], "Alpha docs from rustdoc.");
    assert_eq!(fact["source"]["line"], 9);
    assert_eq!(fact["feature_gates"], json!(["extra"]));
    assert!(
        fact["crate_features"]
            .as_array()
            .unwrap()
            .contains(&json!("extra"))
    );
}

#[test]
fn meta_workspace_paths_remap_to_owner_repo() {
    let fixture = RustFixture::new("remap");
    fixture.write_cargo_at("sdk/crates/sim-conformance", "sim-conformance", &[]);
    let repos = [fixture.repo(
        "sim-sdk",
        "sdk",
        &["sim", "sim-conformance"],
        &["crates/sim-conformance"],
    )];

    let remapped = remap_meta_workspace_path(
        &repos,
        Path::new(".meta-workspace/packages/sim-conformance/tests/rust_intelligence.rs"),
    )
    .unwrap();

    assert_eq!(remapped.repo, "sim-sdk");
    assert_eq!(
        remapped.file,
        "crates/sim-conformance/tests/rust_intelligence.rs"
    );
}

fn source_unit(
    id: &str,
    repo: &str,
    crate_name: &str,
    path: &str,
    line: usize,
    name: &str,
) -> SourceUnit {
    SourceUnit {
        id: id.to_owned(),
        repo: repo.to_owned(),
        crate_name: Some(crate_name.to_owned()),
        kind: "rust-fn".to_owned(),
        path: path.to_owned(),
        line,
        text: format!("signature: fn {name}\ndocs:\n{name} docs.\n"),
        graph_id: None,
        related_ids: Vec::new(),
        panels: Vec::new(),
    }
}

struct RustFixture {
    root: PathBuf,
}

impl RustFixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sim-tooling-rust-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write_cargo_at(&self, path: &str, name: &str, features: &[&str]) {
        let root = self.root.join(path);
        fs::create_dir_all(root.join("src")).unwrap();
        let feature_table = if features.is_empty() {
            String::new()
        } else {
            format!(
                "\n[features]\ndefault = []\n{}",
                features
                    .iter()
                    .map(|feature| format!("{feature} = []\n"))
                    .collect::<String>()
            )
        };
        fs::write(
            root.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{feature_table}"
            ),
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
    }

    fn write_rustdoc_json(&self, repo: &str, crate_file: &str, value: serde_json::Value) {
        let doc_dir = self.root.join(repo).join("target/doc");
        fs::create_dir_all(&doc_dir).unwrap();
        fs::write(
            doc_dir.join(format!("{crate_file}.json")),
            serde_json::to_string(&value).unwrap(),
        )
        .unwrap();
    }

    fn repo(&self, name: &str, path: &str, crates: &[&str], sources: &[&str]) -> RepoEntry {
        RepoEntry {
            name: name.to_owned(),
            kind: "code".to_owned(),
            local_path: path.to_owned(),
            checkout_path: self.root.join(path),
            contains_code: true,
            crate_names: crates.iter().map(|value| (*value).to_owned()).collect(),
            source_paths: sources.iter().map(|value| (*value).to_owned()).collect(),
            validation_command: "cargo test".to_owned(),
            docs_command: "cargo run -p xtask -- simdoc --check".to_owned(),
            pin: "aaaa".to_owned(),
            publish_to_github: false,
            status: RepoStatus::Clean,
        }
    }
}

impl Drop for RustFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
