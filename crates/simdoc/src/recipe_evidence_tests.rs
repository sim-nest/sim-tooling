// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;

const STANDARD: &str = "#!/bin/sh\nset -eu\ncargo test --workspace --quiet\nprintf 'ok\\n'\n";

// conformance: a repository's recipes have a DECLARED consumer only by an
// exact declaration of a tracked file; nothing claims the consumer validates.

#[test]
fn recipes_have_a_declared_consumer_only_by_an_exact_tracked_declaration() {
    let repo = crate::test_fixture::FixtureRepo::nested("harness-consumer");
    let harness = |repo: &crate::test_fixture::FixtureRepo| {
        let root = repo.path().canonicalize().unwrap();
        let scope = crate::owned::enter(&root).unwrap();
        let found = declared_consumer(&root);
        scope.finish().unwrap();
        found
    };
    // A tracked script (and an xtask that names the command) that nothing
    // declared runs proves nothing.
    repo.write("scripts/check-recipes.sh", STANDARD);
    repo.write(
        "xtask/src/main.rs",
        "fn main() { match std::env::args().nth(1).as_deref() { Some(\"check-recipes\") => run(), _ => {} } }\nfn run() {}\n",
    );
    repo.commit();
    assert_eq!(harness(&repo), None);

    let manifest = repo.read("Cargo.toml");
    let declare = |repo: &crate::test_fixture::FixtureRepo, command: &str| {
        repo.write(
            "Cargo.toml",
            &manifest.replace(
                "docs-command = \"cargo run -p xtask -- simdoc\"",
                &format!(
                    "docs-command = \"cargo run -p xtask -- simdoc\"\n\
                     validation-commands = [\"{command}\"]"
                ),
            ),
        );
        repo.commit();
    };
    // Near misses are not consumers.
    for command in [
        "cargo run -p xtask -- check-recipes --extra",
        "sh scripts/check-recipes.sh --extra",
        "sh scripts/check-recipes.sh; true",
        "bash scripts/check-recipes.sh",
    ] {
        declare(&repo, command);
        assert_eq!(harness(&repo), None, "{command}");
    }
    declare(&repo, "sh scripts/check-recipes.sh");
    assert_eq!(
        harness(&repo).as_deref(),
        Some("sh scripts/check-recipes.sh")
    );
    declare(&repo, "cargo run -p xtask -- check-recipes");
    assert_eq!(harness(&repo).as_deref(), Some("xtask check-recipes"));

    // A CI workflow `run:` step is a consumer; a name, a comment, or an
    // untracked workflow is not.
    declare(&repo, "cargo test");
    let decoy = |body: &str| {
        repo.write(".github/workflows/ci.yml", body);
        repo.commit();
    };
    let run = "sh scripts/check-recipes.sh";
    for (label, body) in [
        (
            "name-and-comment",
            format!(
                "name: {run}\n# - run: {run}\non: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo test\n"
            ),
        ),
        (
            "with-script",
            format!(
                "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: x/y@v1\n        with:\n          script: |\n            run: {run}\n"
            ),
        ),
        (
            "job-if",
            format!(
                "on: push\njobs:\n  t:\n    runs-on: x\n    if: false\n    steps:\n      - run: {run}\n"
            ),
        ),
        (
            "step-if",
            format!(
                "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: {run}\n        if: false\n"
            ),
        ),
        (
            "step-if-first",
            format!(
                "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - if: false\n        run: {run}\n"
            ),
        ),
        (
            "continue-on-error",
            format!(
                "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: {run}\n        continue-on-error: true\n"
            ),
        ),
        (
            "dispatch-only",
            format!(
                "on: workflow_dispatch\njobs:\n  t:\n    runs-on: x\n    steps:\n      - run: {run}\n"
            ),
        ),
        (
            "job-if-after-steps",
            format!(
                "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: {run}\n    if: false\n"
            ),
        ),
        (
            "job-defaults",
            format!(
                "on: push\njobs:\n  t:\n    runs-on: x\n    defaults:\n      run:\n        working-directory: x\n    steps:\n      - run: {run}\n"
            ),
        ),
        (
            "workflow-defaults",
            format!(
                "on: push\ndefaults:\n  run:\n    working-directory: x\njobs:\n  t:\n    steps:\n      - run: {run}\n"
            ),
        ),
        (
            "shell-noop",
            format!(
                "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: {run}\n        shell: \"true {{0}}\"\n"
            ),
        ),
        (
            "working-directory",
            format!(
                "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: {run}\n        working-directory: other\n"
            ),
        ),
        (
            "no-trigger",
            format!("jobs:\n  t:\n    runs-on: x\n    steps:\n      - run: {run}\n"),
        ),
        (
            "no-runs-on",
            format!("on: push\njobs:\n  t:\n    steps:\n      - run: {run}\n"),
        ),
    ] {
        decoy(&body);
        assert_eq!(harness(&repo), None, "{label}");
    }
    // Only a workflow GitHub reads: not one in a subdirectory.
    repo.write(
        ".github/workflows/nested/ci.yml",
        &format!(
            "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: {run}\n"
        ),
    );
    repo.commit();
    assert_eq!(harness(&repo), None, "nested workflow");
    repo.write(
        ".github/workflows/recipes.yml",
        "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: sh scripts/check-recipes.sh\n",
    );
    assert_eq!(
        harness(&repo),
        None,
        "an untracked workflow is not a consumer"
    );
    repo.commit();
    assert_eq!(
        harness(&repo).as_deref(),
        Some("sh scripts/check-recipes.sh")
    );
    repo.git_remove(".github/workflows/recipes.yml");
    // The xtask consumer needs the tracked xtask entry point, and nothing
    // more: what its arm does is not asserted.
    declare(&repo, "cargo run -p xtask -- check-recipes");
    repo.write(
        ".github/workflows/ci.yml",
        "on: push\njobs:\n  t:\n    runs-on: x\n    steps:\n      - run: cargo test\n",
    );
    repo.write("xtask/src/main.rs", "fn main() {}\n");
    repo.commit();
    assert_eq!(harness(&repo).as_deref(), Some("xtask check-recipes"));
    repo.git_remove("xtask/src/main.rs");
    assert_eq!(harness(&repo), None, "no tracked xtask entry point");
    repo.write(
        ".github/workflows/ci.yml",
        "on: push\njobs:\n  t:\n    runs-on: x\n    steps:\n      - run: sh scripts/check-recipes.sh\n",
    );
    repo.commit();
    // The script itself must still be the repository's.
    declare(&repo, "sh scripts/check-recipes.sh");
    repo.git_remove("scripts/check-recipes.sh");
    assert_eq!(harness(&repo), None);
}

/// The generated wording never claims validation.
#[test]
fn declared_consumer_wording_never_claims_a_check() {
    let text = declared_consumer_evidence("xtask check-recipes");
    assert!(text.contains("declared consumer"), "{text}");
    assert!(text.contains("not validated"), "{text}");
    assert!(!text.contains("checked"), "{text}");
}
