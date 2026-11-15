// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::super::policy_fixture::{Repo, cargo_metadata, contract_packages};

const HARNESS: &str = "cargo test --manifest-path nested/Cargo.toml";

fn declare(repo: &Repo) {
    repo.root_package(
        "app",
        &format!(
            "validation-commands = [\"{HARNESS}\"]\n\
             contract-exclusions = [{{ path = \"nested\", class = \"focused-test-harness\", \
             consumer = \"{HARNESS}\", reason = \"parent source\" }}]"
        ),
    );
}

fn harness_manifest(body: &str) -> String {
    format!(
        "[package]\nname = \"focused\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         publish = false\n\n{body}\n[workspace]\n"
    )
}

#[test]
fn a_parent_library_that_declares_tests_is_a_harness_without_a_path_dependency() {
    let repo = Repo::new("harness-parent-lib");
    declare(&repo);
    repo.write(
        "src/lib.rs",
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn kept() {}\n}\n",
    );
    repo.write(
        "nested/Cargo.toml",
        &harness_manifest("[lib]\npath = \"../src/lib.rs\"\n"),
    );

    contract_packages(&repo.root).expect("parent library is covered source");
}

#[test]
fn a_path_attribute_to_covered_source_ties_the_harness() {
    let repo = Repo::new("harness-path-attr");
    declare(&repo);
    repo.write("src/covered.rs", "pub fn covered() {}\n");
    repo.write(
        "nested/src/lib.rs",
        "#[path = \"../../src/covered.rs\"]\nmod covered;\n\n#[test]\nfn runs() {}\n",
    );
    repo.write("nested/Cargo.toml", &harness_manifest(""));

    contract_packages(&repo.root).expect("path attribute reaches covered source");
}

#[test]
fn a_commented_path_attribute_is_not_a_tie() {
    let repo = Repo::new("harness-path-comment");
    declare(&repo);
    repo.write("src/covered.rs", "pub fn covered() {}\n");
    repo.write(
        "nested/src/lib.rs",
        "// #[path = \"../../src/covered.rs\"]\n#[test]\nfn runs() {}\n",
    );
    repo.write("nested/Cargo.toml", &harness_manifest(""));

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("has no path dependency on a contract package"),
        "{err}"
    );
}

#[test]
fn a_cfg_not_test_path_attribute_is_not_a_tie() {
    let repo = Repo::new("harness-path-cfg-off");
    declare(&repo);
    repo.write("src/covered.rs", "pub fn covered() {}\n");
    repo.write(
        "nested/src/lib.rs",
        "#[cfg(not(test))]\n#[path = \"../../src/covered.rs\"]\nmod covered;\n\n#[test]\nfn runs() {}\n",
    );
    repo.write("nested/Cargo.toml", &harness_manifest(""));

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("has no path dependency on a contract package"),
        "{err}"
    );
}

#[test]
fn a_parent_path_that_leaves_the_repository_is_not_a_harness_source() {
    let repo = Repo::new("harness-parent-escapes");
    declare(&repo);
    repo.write(
        "nested/src/lib.rs",
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn kept() {}\n}\n",
    );
    repo.write(
        "nested/Cargo.toml",
        &harness_manifest(
            "[lib]\npath = \"../../outside.rs\"\n\n[dependencies]\napp = { path = \"..\" }\n",
        ),
    );

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("has no test target"), "{err}");
}
