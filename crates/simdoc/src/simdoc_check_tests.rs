// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::fs;

use crate::{
    cardspine_state::{CardSpineState, lane_digest},
    test_fixture::FixtureRepo,
};

// conformance: `--check` compares every generated lane and never lets a
// local cache decide that a lane need not be compared.

fn validation_matrix(repo: &FixtureRepo) -> String {
    let args = ["simdoc", "validation-matrix", "--repo"]
        .map(str::to_owned)
        .into_iter()
        .chain([repo.path().to_string_lossy().into_owned()])
        .collect();
    crate::run(args).unwrap();
    repo.read("docs/generated/validation-matrix.md")
}

#[test]
fn a_file_that_is_not_tracked_does_not_exist_for_generation() {
    // The fuzz manifest adds a matrix row only when the repository owns it.
    let repo = FixtureRepo::nested("exists-untracked");
    // Untracked (and undeclared): the manifest does not exist for
    // generation, so it adds no row.
    repo.write(
        "fuzz/Cargo.toml",
        "[package]\nname = \"fuzz\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
         description = \"Fuzz.\"\npublish = false\n\n[workspace]\n",
    );
    repo.write("fuzz/src/lib.rs", "//! Fuzz.\n");
    assert!(!validation_matrix(&repo).contains("fuzz compile"));
    // Tracked and declared: it is the repository's, and it does.
    let root = repo
        .read("Cargo.toml")
        .replace("exclude = [\"nested\"]", "exclude = [\"nested\", \"fuzz\"]")
        .replace(
            "contract-workspaces = [\"nested\"]",
            "contract-workspaces = [\"nested\", \"fuzz\"]",
        );
    repo.write("Cargo.toml", &root);
    repo.commit();
    assert!(validation_matrix(&repo).contains("fuzz compile"));

    // Likewise the recipe harness a generated index names.
    assert!(!crate::owned::is_owned_file(
        repo.path().join("scripts/check-recipes.sh")
    ));
    repo.write("scripts/check-recipes.sh", "#!/bin/sh\n");
    let scope = crate::owned::enter(repo.path()).unwrap();
    assert!(!crate::owned::is_owned_file(
        repo.path()
            .canonicalize()
            .unwrap()
            .join("scripts/check-recipes.sh")
    ));
    scope.finish().unwrap();
    repo.commit();
    let scope = crate::owned::enter(repo.path()).unwrap();
    assert!(crate::owned::is_owned_file(
        repo.path()
            .canonicalize()
            .unwrap()
            .join("scripts/check-recipes.sh")
    ));
    scope.finish().unwrap();
}

/// Generation through the production entrypoint refuses a package whose
/// (tracked, committed) description smuggles in a machine-local path.
#[test]
fn a_local_path_in_tracked_text_never_reaches_a_generated_file() {
    for secret in [
        "/tmp/customer-secret",
        "/var/runner-4711/work",
        "/etc/private-job.conf",
    ] {
        let repo = FixtureRepo::nested("publish-secret");
        let manifest = repo
            .read("Cargo.toml")
            .replace("Fixture application.", &format!("Reads {secret} at start."));
        repo.write("Cargo.toml", &manifest);
        repo.commit();
        let err = repo.run_simdoc(&[]).unwrap_err();
        assert!(
            err.contains("refused to publish") && err.contains(secret),
            "{secret}: {err}"
        );
        assert!(
            !repo
                .path()
                .join("docs/generated/repo-contract.json")
                .exists()
        );
    }
}

#[test]
fn check_never_trusts_the_local_cache_to_skip_a_lane() {
    let repo = FixtureRepo::nested("check-cache");
    repo.run_simdoc(&[]).unwrap();
    repo.commit();
    repo.run_simdoc(&["--check"]).unwrap();

    // Tamper with a committed lane, then make the local cache vouch for the
    // tampered bytes.
    let lane = "docs/agents/cards.jsonl";
    let tampered = format!("{}{{\"tampered\":true}}\n", repo.read(lane));
    fs::write(repo.path().join(lane), &tampered).unwrap();
    let mut state = CardSpineState::read(repo.path()).unwrap().unwrap();
    state
        .lane_digests
        .insert(lane.to_owned(), lane_digest(&tampered));
    state.write(repo.path()).unwrap();

    let err = repo.run_simdoc(&["--check"]).unwrap_err();
    assert!(err.contains("stale generated doc artifacts"), "{err}");
    assert!(err.contains(lane), "{err}");
    // Generation may reuse the cache to save work; the check stays exact, so a
    // poisoned cache can only ever make a check fail, never pass.
    repo.run_simdoc(&[]).unwrap();
    assert!(repo.run_simdoc(&["--check"]).is_err());
}

/// Cargo's writes must not follow a link: `cargo doc` writes into `target`.
#[test]
fn a_target_that_is_a_symlink_refuses_the_docs_build() {
    let repo = FixtureRepo::nested("target-link");
    let elsewhere = repo.path().parent().unwrap().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, repo.path().join("target")).unwrap();
    let err = repo.run_simdoc(&["--rustdoc", "force"]).unwrap_err();
    assert!(
        err.contains("is a symlink") && err.contains("cargo doc"),
        "{err}"
    );
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

/// `cargo doc` builds into a private target directory and reads only a
/// tracked lock: a `target` left in the tree is never reused or written, and
/// an untracked `Cargo.lock` never decides which registry crates are built.
#[test]
fn the_docs_build_never_uses_the_trees_target_or_an_untracked_lock() {
    let repo = FixtureRepo::nested("docs-private-target");
    let status = std::process::Command::new(env!("CARGO"))
        .args(["generate-lockfile", "--offline", "--manifest-path"])
        .arg(repo.path().join("Cargo.toml"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    repo.write("target/debug/planted", "attacker\n");
    repo.commit();
    repo.run_simdoc(&["--rustdoc", "force"]).unwrap();
    assert!(
        !repo.path().join("target/doc").exists(),
        "cargo doc wrote into the tree's target"
    );
    assert_eq!(repo.read("target/debug/planted"), "attacker\n");

    // Untrack the lock (leaving the file): it may not decide the build.
    repo.git_untrack("Cargo.lock");
    let err = repo.run_simdoc(&["--rustdoc", "force"]).unwrap_err();
    assert!(
        err.contains("does not own") && err.contains("Cargo.lock"),
        "{err}"
    );
}
