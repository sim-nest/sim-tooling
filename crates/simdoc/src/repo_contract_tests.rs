// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use sim_codec_index::{IndexCodec, IndexForm};

use super::*;

// conformance: generated repository contracts are canonical, bounded, and side-effect free.

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
fn simdoc_generated_contracts_list_controlled_tooling_target() {
    let root = crate::tooling_checkout_root();
    let artifacts = contract_artifacts(&root).unwrap();

    assert_eq!(artifacts.package_count, 5);

    let feature_map = generated_json(&artifacts, "feature-map.json");
    let provenance = generated_json(&artifacts, "provenance.json");
    let rustdoc_index = generated_json(&artifacts, "rustdoc-index.json");
    let repo_contract = generated_json(&artifacts, "repo-contract.json");
    let index_fragment = IndexCodec
        .decode_fragment(
            IndexForm::Sx,
            artifacts.files.get("sim-index-fragment.sx").unwrap(),
        )
        .unwrap();

    let package_names = feature_map["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|package| package["package"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        package_names,
        [
            "sim-check-pack",
            "sim-check-pack-ubuntu-pc",
            "sim-check-pack-xtask",
            "simdoc",
            "xtask"
        ]
    );
    let check_pack = repo_contract["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "sim-check-pack")
        .unwrap();
    assert_eq!(check_pack["manifest"], "crates/sim-check-pack/Cargo.toml");
    assert_eq!(provenance["schema"], "sim.provenance.v1");
    assert_eq!(provenance["repo"], "sim-tooling");
    assert_eq!(
        provenance["regeneration_command"],
        "cargo run --locked --offline --manifest-path crates/simdoc/Cargo.toml -- simdoc"
    );
    assert!(provenance.get("generated_by").is_none());
    assert_eq!(provenance["execution"]["operation"], CONTRACT_OPERATION);
    assert_eq!(provenance["execution"]["executable"], executable_identity());
    assert_eq!(provenance["execution"]["toolchain"], toolchain_identity());
    assert_eq!(repo_contract["contract_exclusions"], json!([]));
    let validation = provenance["validation_commands"].as_array().unwrap();
    assert_eq!(validation.len(), 12);
    assert!(validation.contains(&json!(
        "cargo run --locked --manifest-path crates/simdoc/Cargo.toml -- simdoc --check"
    )));
    assert_eq!(provenance["api_docs"], "target/doc/");
    assert!(provenance["source_commit"].as_str().is_some());
    assert!(
        provenance["source_remote"]
            .as_str()
            .is_some_and(|remote| remote.starts_with("https://github.com/"))
    );
    assert_eq!(provenance["git_commit"], provenance["source_commit"]);
    assert!(
        rustdoc_index["packages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|package| package["package"] == "xtask")
    );
    let root_package = repo_contract["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["root"] == "")
        .unwrap();
    assert_eq!(root_package["name"], "xtask");
    assert_eq!(root_package["manifest"], "Cargo.toml");
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
            .any(|subject| subject.id.as_str() == "crate/sim-check-pack")
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

#[test]
fn generation_timestamp_survives_unchanged_workspace_hash() {
    let preserved = json!({
        "workspace_hash": "same-hash",
        "generation_timestamp": "2026-08-21T10:00:00+02:00"
    });
    let no_git = temp_root("sim-tooling-timestamp-preserved");

    assert_eq!(
        generation_timestamp(&no_git, &preserved, "same-hash", "not-read").unwrap(),
        "2026-08-21T10:00:00+02:00"
    );

    fs::remove_dir_all(no_git).unwrap();
}

#[test]
fn generation_timestamp_takes_commit_date_after_workspace_change() {
    let (repo, commit) =
        committed_repo("sim-tooling-timestamp-changed", "2026-09-20T08:30:00+02:00");
    let preserved = json!({
        "workspace_hash": "old-hash",
        "generation_timestamp": "2026-08-21T10:00:00+02:00"
    });

    assert_eq!(
        generation_timestamp(&repo, &preserved, "new-hash", &commit).unwrap(),
        "2026-09-20T08:30:00+02:00"
    );
    assert_eq!(
        generation_timestamp(&repo, &json!({}), "new-hash", &commit).unwrap(),
        "2026-09-20T08:30:00+02:00"
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn generation_timestamp_refuses_unreadable_commit_date() {
    let (repo, _) = committed_repo("sim-tooling-timestamp-missing", "2026-09-20T08:30:00+02:00");
    let missing = "0".repeat(40);

    let err = generation_timestamp(&repo, &json!({}), "new-hash", &missing).unwrap_err();
    assert!(err.contains("did not return a committer date"), "{err}");
    let err = generation_timestamp(&repo, &json!({}), "new-hash", "--output=x").unwrap_err();
    assert!(err.contains("not a hexadecimal commit id"), "{err}");
    let legacy = json!({"workspace_hash": "same-hash", "generation_timestamp": "unknown"});
    assert!(generation_timestamp(&repo, &legacy, "same-hash", &missing).is_err());

    fs::remove_dir_all(repo).unwrap();
}

fn committed_repo(name: &str, committer_date: &str) -> (PathBuf, String) {
    let repo = temp_root(name);
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args([
                "-c",
                "user.name=simdoc",
                "-c",
                "user.email=simdoc@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&repo)
            .env("GIT_AUTHOR_DATE", committer_date)
            .env("GIT_COMMITTER_DATE", committer_date)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?} failed: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "--quiet"]);
    git(&[
        "commit",
        "--quiet",
        "--no-verify",
        "--allow-empty",
        "-m",
        "fixture",
    ]);
    let commit = git(&["rev-parse", "HEAD"]);
    (repo, commit)
}

fn generated_json(artifacts: &ContractArtifacts, name: &'static str) -> Value {
    serde_json::from_str(artifacts.files.get(name).unwrap()).unwrap()
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

#[test]
fn a_fixture_contract_is_generated_from_one_measured_snapshot() {
    let repo = crate::test_fixture::FixtureRepo::nested("contract-snapshot");

    let artifacts = contract_artifacts(repo.path()).unwrap();
    assert_eq!(artifacts.package_count, 2);
    let provenance = generated_json(&artifacts, "provenance.json");
    assert_eq!(provenance["repo"], "sim-fixture");
    assert_eq!(provenance["generation_timestamp"], "2026-09-01T12:00:00Z");
    assert_eq!(provenance["execution"]["operation"], CONTRACT_OPERATION);

    let contract = generated_json(&artifacts, "repo-contract.json");
    assert_eq!(contract["contract_exclusions"], json!([]));

    repo.write(
        "Cargo.toml",
        &repo.read("Cargo.toml").replace(
            "contract-workspaces = [\"nested\"]\n",
            "contract-workspaces = [\"nested\"]\ncontract-exclusions = [{ path = \"tests/ui\", class = \"test-fixture\", reason = \"compile-fail cases\" }]\n",
        ),
    );
    repo.write(
        "tests/ui/case/Cargo.toml",
        "[package]\nname = \"case\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[workspace]\n",
    );
    repo.write("tests/ui/case/src/lib.rs", "");
    repo.commit();
    let artifacts = contract_artifacts(repo.path()).unwrap();
    let contract = generated_json(&artifacts, "repo-contract.json");
    assert_eq!(
        contract["contract_exclusions"],
        json!([{
            "path": "tests/ui",
            "class": "test-fixture",
            "reason": "compile-fail cases",
            "manifests": ["tests/ui/case/Cargo.toml"],
        }])
    );
    assert!(artifacts.files["repo-contract.md"].contains("| `tests/ui` | `test-fixture` | 1 |"));

    let err = contract_artifacts_observed(repo.path(), &|| {
        repo.write(
            "src/lib.rs",
            "//! Fixture application.\n\npub fn changed() {}\n",
        );
    })
    .err()
    .unwrap();
    assert!(err.contains("changed during generation"), "{err}");
}

#[test]
fn a_contract_that_would_record_an_outside_path_is_refused() {
    let repo = crate::test_fixture::FixtureRepo::nested("contract-outside-path");
    let outside = repo.path().parent().unwrap().to_string_lossy().into_owned();
    repo.write(
        "nested/tool/Cargo.toml",
        &repo
            .read("nested/tool/Cargo.toml")
            .replace("Fixture tool.", &format!("Built from {outside}.")),
    );
    repo.commit();

    let err = contract_artifacts(repo.path()).err().unwrap();
    assert!(err.contains("nothing outside the repository"), "{err}");
}
