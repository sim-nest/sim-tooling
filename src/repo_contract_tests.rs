use std::{
    env, fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use sim_codec_index::{IndexCodec, IndexForm};

use super::*;

#[test]
fn stable_hash_uses_repo_relative_paths() {
    let left = temp_root("sim-tooling-hash-left");
    let right = temp_root("sim-tooling-hash-right");
    fs::create_dir_all(left.join("src")).unwrap();
    fs::create_dir_all(right.join("src")).unwrap();
    fs::write(left.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
    fs::write(right.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();

    let left_hash = stable_hash(&left, &[left.join("src/lib.rs")]);
    let right_hash = stable_hash(&right, &[right.join("src/lib.rs")]);

    assert_eq!(left_hash, right_hash);

    fs::remove_dir_all(left).unwrap();
    fs::remove_dir_all(right).unwrap();
}

#[test]
fn simdoc_generated_contracts_list_root_package() {
    let root = source_checkout_root();
    let artifacts = contract_artifacts(&root).unwrap();

    assert_eq!(artifacts.package_count, 1);

    let feature_map = generated_json(&artifacts, "feature-map.json");
    let provenance = generated_json(&artifacts, "provenance.json");
    let rustdoc_index = generated_json(&artifacts, "rustdoc-index.json");
    let repo_contract = generated_json(&artifacts, "repo-contract.json");
    let index_fragment = IndexCodec
        .decode(
            IndexForm::Sx,
            artifacts.files.get("sim-index-fragment.sx").unwrap(),
        )
        .unwrap();

    assert_eq!(feature_map["packages"][0]["package"], "xtask");
    assert_eq!(provenance["schema"], "sim.provenance.v1");
    assert_eq!(provenance["repo"], "sim-tooling");
    assert_eq!(provenance["generated_by"], "cargo run -p xtask -- simdoc");
    assert_eq!(provenance["api_docs"], "target/doc/");
    assert!(provenance["source_commit"].as_str().is_some());
    assert!(
        provenance["source_remote"]
            .as_str()
            .is_some_and(|remote| remote.starts_with("https://github.com/"))
    );
    assert_eq!(provenance["git_commit"], provenance["source_commit"]);
    assert_eq!(rustdoc_index["packages"][0]["package"], "xtask");
    assert_eq!(repo_contract["packages"][0]["name"], "xtask");
    assert_eq!(repo_contract["packages"][0]["manifest"], "Cargo.toml");
    assert_eq!(repo_contract["packages"][0]["root"], "");
    assert!(
        index_fragment
            .subjects
            .iter()
            .any(|subject| subject.id.as_str() == "repo/sim-tooling")
    );
    assert!(
        index_fragment
            .subjects
            .iter()
            .any(|subject| subject.id.as_str() == "crate/xtask")
    );
    assert!(
        index_fragment
            .subjects
            .iter()
            .any(|subject| subject.id.as_str() == "doc-set/sim-tooling/generated")
    );
    assert!(index_fragment.edges.iter().any(|edge| {
        edge.from == "repo/sim-tooling" && edge.rel == "contains" && edge.to == "crate/xtask"
    }));
}

#[test]
fn origin_sanitizer_emits_public_github_url() {
    let ssh_github_origin = concat!("git", "@", "github.com:sim-nest/sim-tooling.git");
    assert_eq!(
        sanitize_origin_url(ssh_github_origin).unwrap(),
        "https://github.com/sim-nest/sim-tooling"
    );
    assert_eq!(
        sanitize_origin_url("https://github.com/sim-nest/sim-tooling.git").unwrap(),
        "https://github.com/sim-nest/sim-tooling"
    );
    assert!(sanitize_origin_url("/tmp/sim-tooling").is_err());
}

#[test]
fn preserved_source_commit_survives_generated_doc_commit() {
    let preserved = json!({
        "workspace_hash": "same-hash",
        "source_commit": "source-commit",
        "git_commit": "legacy-commit"
    });

    assert_eq!(
        preserved_source_commit(&preserved, "same-hash").as_deref(),
        Some("source-commit")
    );
}

#[test]
fn preserved_source_commit_accepts_legacy_git_commit() {
    let preserved = json!({
        "workspace_hash": "same-hash",
        "git_commit": "legacy-commit"
    });

    assert_eq!(
        preserved_source_commit(&preserved, "same-hash").as_deref(),
        Some("legacy-commit")
    );
}

#[test]
fn preserved_source_commit_ignores_changed_workspace_hash() {
    let preserved = json!({
        "workspace_hash": "old-hash",
        "source_commit": "source-commit"
    });

    assert!(preserved_source_commit(&preserved, "new-hash").is_none());
}

fn generated_json(artifacts: &ContractArtifacts, name: &'static str) -> Value {
    serde_json::from_str(artifacts.files.get(name).unwrap()).unwrap()
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

fn temp_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
