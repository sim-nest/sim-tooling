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
