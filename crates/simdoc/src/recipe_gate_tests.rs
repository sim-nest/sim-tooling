// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{fs, os::unix::fs::PermissionsExt};

use super::*;
use crate::test_fixture::FixtureRepo;

// conformance: simdoc validates recipe material statically and never
// executes anything a repository ships.

const RECIPE: &str = "recipes/01-basics/open/recipe.toml";

fn recipes_repo(label: &str) -> FixtureRepo {
    let repo = FixtureRepo::nested(label);
    repo.write(RECIPE, "title = \"Open\"\nsetup = \"setup.siml\"\n");
    repo.write("recipes/01-basics/open/setup.siml", "(open)\n");
    repo.commit();
    repo
}

fn validate_repo(repo: &FixtureRepo) -> Result<usize, String> {
    let scope = crate::owned::enter(repo.path()).unwrap();
    let result = validate(&repo.path().canonicalize().unwrap());
    scope.finish().unwrap();
    result
}

#[test]
fn well_formed_recipes_are_counted() {
    let repo = recipes_repo("recipe-gate-ok");
    repo.write("recipes/book.toml", "book = \"root\"\n");
    repo.write("recipes/01-basics/chapter.toml", "title = \"Basics\"\n");
    repo.commit();
    assert_eq!(validate_repo(&repo), Ok(1));
}

#[test]
fn malformed_or_dangling_recipes_are_refused_with_their_path() {
    for (label, recipe, fragment) in [
        ("broken-toml", "title = \n", RECIPE),
        (
            "untitled",
            "setup = \"setup.siml\"\n",
            "neither `title` nor `name`",
        ),
        (
            "missing-setup",
            "title = \"Open\"\nsetup = \"absent.siml\"\n",
            "not a tracked file below the recipe",
        ),
        (
            "traversal",
            "title = \"Open\"\nexpected = \"../open/setup.siml\"\n",
            "not a tracked file below the recipe",
        ),
        (
            "typed",
            "title = \"Open\"\nsetup = 3\n",
            "`setup` is not a string",
        ),
    ] {
        let repo = recipes_repo(&format!("recipe-gate-{label}"));
        repo.write(RECIPE, recipe);
        repo.commit();
        let err = validate_repo(&repo).unwrap_err();
        assert!(err.contains(fragment), "{label}: {err}");
    }
}

#[test]
fn an_absolute_reference_is_refused_even_when_it_names_a_tracked_file() {
    let repo = recipes_repo("recipe-gate-absolute");
    let absolute = repo
        .path()
        .canonicalize()
        .unwrap()
        .join("recipes/01-basics/open/setup.siml");
    repo.write(
        RECIPE,
        &format!("title = \"Open\"\nsetup = \"{}\"\n", absolute.display()),
    );
    repo.commit();
    let err = validate_repo(&repo).unwrap_err();
    assert!(err.contains("not a tracked file below the recipe"), "{err}");
}

#[test]
fn a_reference_below_the_recipe_directory_is_a_file() {
    let repo = recipes_repo("recipe-gate-below");
    repo.write(RECIPE, "title = \"Open\"\nsetup = \"src/main.rs\"\n");
    repo.write("recipes/01-basics/open/src/main.rs", "fn main() {}\n");
    repo.commit();
    assert_eq!(validate_repo(&repo), Ok(1));
}

#[test]
fn a_reference_that_is_prose_is_not_a_file() {
    let repo = recipes_repo("recipe-gate-prose");
    repo.write(RECIPE, "title = \"Open\"\nexpected = \"the door opens.\"\n");
    repo.commit();
    assert_eq!(validate_repo(&repo), Ok(1));
}

#[test]
fn an_untracked_sibling_does_not_satisfy_a_reference() {
    let repo = recipes_repo("recipe-gate-untracked");
    repo.write(RECIPE, "title = \"Open\"\nexpected = \"expected.txt\"\n");
    repo.commit();
    repo.write("recipes/01-basics/open/expected.txt", "ok\n");
    let err = validate_repo(&repo).unwrap_err();
    assert!(err.contains("not a tracked file below the recipe"), "{err}");
}

/// A repository script that would leave a marker if anything ran it.
fn plant_script(repo: &FixtureRepo, marker: &std::path::Path) {
    repo.write(
        "scripts/check-recipes.sh",
        &format!("#!/bin/sh\ntouch {}\nexit 1\n", marker.display()),
    );
    let script = repo.path().join("scripts/check-recipes.sh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn simdoc_never_executes_a_recipe_script_of_any_kind() {
    for kind in ["tracked", "ignored", "symlink"] {
        let repo = recipes_repo(&format!("recipe-gate-exec-{kind}"));
        let marker = repo.path().parent().unwrap().join("executed");
        match kind {
            "tracked" => plant_script(&repo, &marker),
            "ignored" => {
                repo.write(".gitignore", "scripts/\n");
                plant_script(&repo, &marker);
            }
            _ => {
                plant_script(&repo, &marker);
                fs::rename(
                    repo.path().join("scripts/check-recipes.sh"),
                    repo.path().parent().unwrap().join("outside.sh"),
                )
                .unwrap();
                std::os::unix::fs::symlink(
                    repo.path().parent().unwrap().join("outside.sh"),
                    repo.path().join("scripts/check-recipes.sh"),
                )
                .unwrap();
            }
        }
        repo.commit();
        let outcome = repo.run_simdoc(&[]);
        assert!(!marker.exists(), "{kind}: a repository script was executed");
        outcome.unwrap_or_else(|err| panic!("{kind}: simdoc refused: {err}"));
        repo.commit();
        repo.run_simdoc(&["--check"]).unwrap();
        assert!(!marker.exists(), "{kind}: a repository script was executed");
    }
}

/// The production entrypoint stops on malformed recipe material: removing the
/// `validate` call from `simdoc` makes this test fail (the direct tests above
/// call `validate` themselves and would not notice).
#[test]
fn a_malformed_recipe_stops_generation_through_the_public_route() {
    let repo = recipes_repo("recipe-gate-route");
    repo.write(RECIPE, "title = \n");
    repo.commit();
    let err = repo.run_simdoc(&[]).unwrap_err();
    assert!(
        err.contains("recipe validation failed") && err.contains(RECIPE),
        "{err}"
    );
    assert!(
        !repo
            .path()
            .join("docs/generated/repo-contract.json")
            .exists()
    );
    let err = repo.run_simdoc(&["--check"]).unwrap_err();
    assert!(err.contains("recipe validation failed"), "{err}");
}

/// A tracked recipe script is not evidence that recipes run: the generated
/// index claims a check only for a declared consumer whose script is a
/// standard runner (a no-op script, or one that swallows failure, is not).
#[test]
fn generated_recipe_evidence_names_only_a_declared_standard_consumer() {
    let repo = recipes_repo("recipe-gate-evidence");
    repo.write(
        "recipes/book.toml",
        "book = \"fixture\"\ntitle = \"Fixture\"\nsummary = \"Fixture recipes.\"\n",
    );
    plant_script(&repo, &repo.path().parent().unwrap().join("never-runs"));
    repo.commit();
    repo.run_simdoc(&[]).unwrap();
    let fragment = "docs/generated/sim-index-fragment.sx";
    assert!(
        !repo.read(fragment).contains("check-recipes"),
        "no consumer declared"
    );

    let manifest = repo.read("Cargo.toml").replace(
        "docs-command = \"cargo run -p xtask -- simdoc\"",
        "docs-command = \"cargo run -p xtask -- simdoc\"\n\
         validation-commands = [\"sh scripts/check-recipes.sh\"]",
    );
    repo.write("Cargo.toml", &manifest);
    repo.commit();
    for (label, script) in [
        ("no-op", "#!/bin/sh\nexit 0\n"),
        ("swallowed", "#!/bin/sh\nset -eu\ncargo test || true\n"),
        ("comment-only", "#!/bin/sh\nset -eu\n# cargo test\n"),
    ] {
        repo.write("scripts/check-recipes.sh", script);
        repo.commit();
        repo.run_simdoc(&[]).unwrap();
        assert!(
            !repo.read(fragment).contains("check-recipes"),
            "{label}: a declared no-op script was claimed as a check"
        );
    }
    repo.write(
        "scripts/check-recipes.sh",
        "#!/bin/sh\nset -eu\ncargo test --workspace --quiet\n",
    );
    repo.commit();
    repo.run_simdoc(&[]).unwrap();
    assert!(
        repo.read(fragment).contains("check-recipes"),
        "standard consumer declared"
    );
}

/// The same holds for a declared xtask consumer with an empty arm, and for a
/// workflow whose only mention of a trigger is a comment.
#[test]
fn generated_recipe_evidence_ignores_an_empty_xtask_arm_and_a_comment_trigger() {
    let repo = recipes_repo("recipe-gate-evidence-xtask");
    repo.write(
        "recipes/book.toml",
        "book = \"fixture\"\ntitle = \"Fixture\"\nsummary = \"Fixture recipes.\"\n",
    );
    repo.write(
        "xtask/src/main.rs",
        "fn main() { match std::env::args().nth(1).as_deref() { Some(\"check-recipes\") => {} _ => {} } }\n",
    );
    let manifest = repo.read("Cargo.toml").replace(
        "docs-command = \"cargo run -p xtask -- simdoc\"",
        "docs-command = \"cargo run -p xtask -- simdoc\"\n\
         validation-commands = [\"cargo run -p xtask -- check-recipes\"]",
    );
    repo.write("Cargo.toml", &manifest);
    repo.commit();
    let fragment = "docs/generated/sim-index-fragment.sx";
    repo.run_simdoc(&[]).unwrap();
    assert!(
        !repo.read(fragment).contains("check-recipes"),
        "an empty arm was claimed"
    );

    // A workflow whose trigger appears only in a comment never runs.
    repo.write(
        "Cargo.toml",
        &manifest.replace(
            "validation-commands = [\"cargo run -p xtask -- check-recipes\"]",
            "",
        ),
    );
    repo.write(
        "xtask/src/main.rs",
        "fn main() { match std::env::args().nth(1).as_deref() { Some(\"check-recipes\") => real(), _ => {} } }\nfn real() {}\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        "# on: push\nname: x\non: workflow_dispatch\njobs:\n  t:\n    steps:\n      - run: cargo run -p xtask -- check-recipes\n",
    );
    repo.commit();
    repo.run_simdoc(&[]).unwrap();
    assert!(
        !repo.read(fragment).contains("check-recipes"),
        "a comment trigger was claimed"
    );
    repo.write(
        ".github/workflows/ci.yml",
        "on: push\njobs:\n  t:\n    steps:\n      - run: cargo run -p xtask -- check-recipes\n",
    );
    repo.commit();
    repo.run_simdoc(&[]).unwrap();
    assert!(
        repo.read(fragment).contains("check-recipes"),
        "a real trigger and arm"
    );
}
