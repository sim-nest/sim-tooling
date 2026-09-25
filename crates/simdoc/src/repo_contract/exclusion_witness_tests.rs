// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::super::policy_fixture::{Repo, cargo_metadata};
use super::*;

// conformance: an exclusion is tied to a native consumer by a parsed,
// checked association; text that merely mentions the fixture proves nothing.

/// A repository whose fixture `tests/ui/case` is excluded with `consumer`.
fn fixture_repo(label: &str, class: &str, consumer: &str, extra: &str) -> Repo {
    let repo = Repo::new(label);
    repo.nested_workspace("tests/ui", &["case"]);
    repo.package(
        "tests/ui/case",
        "case",
        "publish = false\n\n[dependencies]\napp = { path = \"../../..\" }\n",
    );
    repo.root_package(
        "app",
        &format!(
            "{extra}\ncontract-exclusions = [{{ path = \"tests/ui\", class = \"{class}\", \
             consumer = \"{consumer}\", reason = \"cases\" }}]"
        ),
    );
    repo
}

fn refused(repo: &Repo) -> String {
    cargo_metadata(&repo.root).unwrap_err()
}

#[test]
fn only_a_live_test_that_reaches_the_fixture_consumes_it() {
    let dead = [
        ("comment", "// tests/ui\n#[test]\nfn t() {}\n"),
        ("doc", "///tests/ui\n#[test]\nfn t() {}\n"),
        (
            "unreached",
            "fn helper() { run(\"tests/ui\"); }\n#[test]\nfn t() {}\n",
        ),
        (
            "cfg-off",
            "#[cfg(any())]\n#[test]\nfn t() { run(\"tests/ui\"); }\n#[test]\nfn u() {}\n",
        ),
        (
            "ignored",
            "#[ignore]\n#[test]\nfn t() { run(\"tests/ui\"); }\n",
        ),
        (
            "if-false",
            "#[test]\nfn t() { if false { run(\"tests/ui\"); } }\n",
        ),
        (
            "macro-definition",
            "#[test]\nfn t() {\n    macro_rules! m { () => { run(\"tests/ui\") }; }\n}\n",
        ),
        (
            "prose",
            "#[test]\nfn t() { run(\"see tests/ui for cases\"); }\n",
        ),
        (
            "other-path",
            "#[test]\nfn t() { run(\"tests/ui/other\"); }\n",
        ),
        (
            "unused-const",
            "const DIR: &str = \"tests/ui\";\n#[test]\nfn t() {}\n",
        ),
        (
            "not-a-test",
            "mod helpers { pub fn dir() -> &'static str { \"tests/ui\" } }\nfn main_like() {}\n",
        ),
    ];
    for (label, source) in dead {
        let repo = fixture_repo(
            &format!("witness-dead-{label}"),
            "test-fixture",
            "tests/ui.rs",
            "",
        );
        repo.write("tests/ui.rs", source);
        let err = refused(&repo);
        assert!(
            err.contains("is not consumed by tests/ui.rs"),
            "{label}: {err}"
        );
    }
    let live = [
        ("direct", "#[test]\nfn t() { run(\"tests/ui\"); }\n"),
        (
            "helper",
            "fn dir() -> &'static str { \"tests/ui\" }\n#[test]\nfn t() { run(dir()); }\n",
        ),
        (
            "const",
            "const DIR: &str = \"tests/ui\";\n#[test]\nfn t() { run(DIR); }\n",
        ),
        (
            "macro-arguments",
            "#[test]\nfn t() { assert!(run(\"./tests/ui/Cargo.toml\")); }\n",
        ),
        (
            "test-module",
            "#[cfg(test)]\nmod inner {\n#[test]\nfn t() { run(\"tests/ui\"); }\n}\n",
        ),
        (
            "if-true",
            "#[test]\nfn t() { if true { run(\"tests/ui\"); } }\n",
        ),
    ];
    for (label, source) in live {
        let repo = fixture_repo(
            &format!("witness-live-{label}"),
            "test-fixture",
            "tests/ui.rs",
            "",
        );
        repo.write("tests/ui.rs", source);
        cargo_metadata(&repo.root).unwrap_or_else(|err| panic!("{label}: {err}"));
    }
}

#[test]
fn a_test_consumer_that_does_not_parse_is_refused() {
    let repo = fixture_repo("witness-unparsed", "test-fixture", "tests/ui.rs", "");
    repo.write("tests/ui.rs", "fn (\n");
    assert!(refused(&repo).contains("does not parse as Rust"));
}

/// A repository whose fixture `recipes/fixture/case` is excluded.
fn recipe_repo(label: &str, book: &str) -> Repo {
    let repo = Repo::new(label);
    repo.nested_workspace("recipes/fixture", &["case"]);
    repo.package("recipes/fixture/case", "case", "publish = false\n");
    repo.write("recipes/book.toml", book);
    repo.root_package(
        "app",
        "contract-exclusions = [{ path = \"recipes/fixture\", class = \"recipe-fixture\", \
         consumer = \"recipes/book.toml\", reason = \"cases\" }]",
    );
    repo
}

#[test]
fn only_a_parsed_value_of_a_recipe_manifest_consumes_a_recipe_fixture() {
    for (label, book) in [
        ("comment", "# recipes/fixture\nbook = \"x\"\n"),
        ("prose", "summary = \"uses recipes/fixture heavily\"\n"),
        ("key", "\"recipes/fixture\" = 1\n"),
        ("table", "[\"recipes/fixture\"]\nbook = \"x\"\n"),
        ("unrelated", "book = \"x\"\n"),
    ] {
        let repo = recipe_repo(&format!("witness-recipe-dead-{label}"), book);
        let err = refused(&repo);
        assert!(
            err.contains("is not consumed by recipes/book.toml"),
            "{label}: {err}"
        );
    }
    for (label, book) in [
        ("repo-relative", "fixture = \"recipes/fixture\"\n"),
        ("array", "fixtures = [\"recipes/fixture/\"]\n"),
        ("nested", "[[uses]]\npath = \"fixture\"\n"),
    ] {
        let repo = recipe_repo(&format!("witness-recipe-live-{label}"), book);
        cargo_metadata(&repo.root).unwrap_or_else(|err| panic!("{label}: {err}"));
    }
    let repo = recipe_repo("witness-recipe-broken", "book = \n");
    assert!(refused(&repo).contains("does not parse as TOML"));
}

const HARNESS: &str = "cargo test --manifest-path tests/ui/case/Cargo.toml";

fn harness_repo(label: &str, consumer: &str, commands: &[&str], with_test: bool) -> Repo {
    let list = commands
        .iter()
        .map(|command| format!("\"{command}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let repo = fixture_repo(
        label,
        "focused-test-harness",
        consumer,
        &format!("validation-commands = [{list}]"),
    );
    if with_test {
        repo.write("tests/ui/case/tests/run.rs", "#[test]\nfn run() {}\n");
    }
    repo
}

#[test]
fn a_harness_needs_a_declared_command_that_runs_it_and_a_matching_target() {
    let repo = harness_repo("witness-harness-ok", HARNESS, &[HARNESS], true);
    cargo_metadata(&repo.root).unwrap();

    let undeclared = harness_repo("witness-harness-undeclared", HARNESS, &["cargo test"], true);
    assert!(refused(&undeclared).contains("not one of the root manifest's `validation-commands`"));

    let chained = "cargo test --manifest-path tests/ui/case/Cargo.toml && true";
    let repo = harness_repo("witness-harness-chained", chained, &[chained], true);
    assert!(refused(&repo).contains("not a plain `cargo test` or `cargo run`"));

    let other = "cargo test --manifest-path tests/other/Cargo.toml";
    let repo = harness_repo("witness-harness-other", other, &[other], true);
    assert!(refused(&repo).contains("not a manifest of the excluded harness"));

    let lib_only = harness_repo("witness-harness-lib-only", HARNESS, &[HARNESS], false);
    assert!(refused(&lib_only).contains("has no test target"));

    let run = "cargo run --manifest-path tests/ui/case/Cargo.toml";
    let tests_only = harness_repo("witness-harness-run-tests", run, &[run], true);
    assert!(refused(&tests_only).contains("has no binary target"));
    let binary = harness_repo("witness-harness-run-bin", run, &[run], false);
    binary.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    cargo_metadata(&binary.root).unwrap();

    let verb = "cargo build --manifest-path tests/ui/case/Cargo.toml";
    let repo = harness_repo("witness-harness-verb", verb, &[verb], true);
    assert!(refused(&repo).contains("not a plain `cargo test` or `cargo run`"));
}

#[test]
fn cargo_invocations_are_read_as_tokens() {
    assert_eq!(
        cargo_invocation("cargo test --locked --manifest-path ./a/Cargo.toml"),
        Some(("test", "a/Cargo.toml".to_owned()))
    );
    assert_eq!(
        cargo_invocation("cargo run --manifest-path=a/Cargo.toml -- x"),
        Some(("run", "a/Cargo.toml".to_owned()))
    );
    for command in [
        "cargo test",
        "cargo doc --manifest-path a/Cargo.toml",
        "sh cargo test --manifest-path a/Cargo.toml",
        "cargo test --manifest-path a/Cargo.toml; rm x",
        "cargo test --manifest-path \"a/Cargo.toml\"",
        "cargo test --manifest-path $X/Cargo.toml",
    ] {
        assert_eq!(cargo_invocation(command), None, "{command}");
    }
}
