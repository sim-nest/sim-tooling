// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::fs;

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
        (
            "comment",
            "// consume_fixture(\"tests/ui\")\n#[test]\nfn t() {}\n",
        ),
        (
            "doc",
            "///consume_fixture(\"tests/ui\")\n#[test]\nfn t() {}\n",
        ),
        (
            "unreached",
            "fn helper() { let d = consume_fixture(\"tests/ui\"); }\n#[test]\nfn t() {}\n",
        ),
        (
            "cfg-off",
            "#[cfg(any())]\n#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n#[test]\nfn u() {}\n",
        ),
        (
            "ignored",
            "#[ignore]\n#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "if-false",
            "#[test]\nfn t() { if false { let d = consume_fixture(\"tests/ui\"); } }\n",
        ),
        (
            "macro-definition",
            "#[test]\nfn t() {\n    macro_rules! m { () => { consume_fixture(\"tests/ui\") }; }\n}\n",
        ),
        (
            "macro-definition-with-a-call-shaped-body",
            "#[test]\nfn t() {\n    macro_rules! m { consume_fixture(\"tests/ui\") }\n}\n",
        ),
        (
            "attribute-value",
            "#[doc = consume_fixture(\"tests/ui\")]\n#[test]\nfn t() {}\n",
        ),
        // A no-op stand-in for the API, or a target switched off as a whole.
        (
            "noop-helper",
            "fn consume_fixture(_: &str) -> u8 { 0 }\n#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "file-level-cfg",
            "#![cfg(any())]\nfn consume_fixture(relative: &str) -> std::path::PathBuf {\n    std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "file-level-cfg-other-os",
            "#![cfg(target_os = \"windows\")]\nfn consume_fixture(relative: &str) -> std::path::PathBuf {\n    std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "file-level-cfg-feature",
            "#![cfg(feature = \"never\")]\nfn consume_fixture(relative: &str) -> std::path::PathBuf {\n    std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "cfg-macro-condition",
            "#[test]\nfn t() { if cfg!(windows) { let d = consume_fixture(\"tests/ui\"); } }\n",
        ),
        (
            "dropped-underscore-name",
            "#[test]\nfn t() { let _dir = consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "dropped-method-chain",
            "#[test]\nfn t() { consume_fixture(\"tests/ui\").to_owned(); }\n",
        ),
        (
            "dropped-drop",
            "#[test]\nfn t() { drop(consume_fixture(\"tests/ui\")); }\n",
        ),
        // A consumption whose value is dropped on the spot consumes nothing.
        (
            "dropped-statement",
            "#[test]\nfn t() { consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "dropped-let-wild",
            "#[test]\nfn t() { let _ = consume_fixture(\"tests/ui\"); }\n",
        ),
        // Code that never runs, however it is written.
        (
            "closure",
            "#[test]\nfn t() { let f = || consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "nested-fn",
            "#[test]\nfn t() {\n    fn never() { let d = consume_fixture(\"tests/ui\"); }\n}\n",
        ),
        (
            "cfg-on-statement",
            "#[test]\nfn t() {\n    #[cfg(any())]\n    let d = consume_fixture(\"tests/ui\");\n}\n",
        ),
        (
            "cfg-on-expression",
            "#[test]\nfn t() {\n    #[cfg(any())]\n    { let d = consume_fixture(\"tests/ui\"); }\n}\n",
        ),
        (
            "after-return",
            "#[test]\nfn t() {\n    return;\n    #[allow(unreachable_code)]\n    let d = consume_fixture(\"tests/ui\");\n}\n",
        ),
        (
            "if-not-true",
            "#[test]\nfn t() { if !true { let d = consume_fixture(\"tests/ui\"); } }\n",
        ),
        (
            "if-cfg-any",
            "#[test]\nfn t() { if cfg!(any()) { let d = consume_fixture(\"tests/ui\"); } }\n",
        ),
        (
            "while-false",
            "#[test]\nfn t() { while false { let d = consume_fixture(\"tests/ui\"); } }\n",
        ),
        (
            "cfg-attr-ignore",
            "#[cfg_attr(all(), ignore)]\n#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "stringify",
            "#[test]\nfn t() { let s = stringify!(consume_fixture(\"tests/ui\")); }\n",
        ),
        (
            "user-macro",
            "macro_rules! discard { ($($t:tt)*) => {}; }\n\
             #[test]\nfn t() { discard!(consume_fixture(\"tests/ui\")); }\n",
        ),
        // Rust resolves a local definition over an import.
        (
            "local-shadows-glob-import",
            "fn dir() { let d = consume_fixture(\"tests/ui\"); }\n\
             #[cfg(test)]\nmod tests {\n    use super::*;\n    fn dir() {}\n    #[test]\n    fn t() { dir(); }\n}\n",
        ),
        // Not the API: a bare literal, a differently named call, a prose value.
        ("bare-literal", "#[test]\nfn t() { run(\"tests/ui\"); }\n"),
        (
            "other-call",
            "#[test]\nfn t() { load_fixture(\"tests/ui\"); }\n",
        ),
        (
            "prose",
            "#[test]\nfn t() { let d = consume_fixture(\"see tests/ui for cases\"); }\n",
        ),
        (
            "other-path",
            "#[test]\nfn t() { let d = consume_fixture(\"tests/ui/other\"); }\n",
        ),
        (
            "unused-const",
            "const DIR: &str = \"tests/ui\";\n#[test]\nfn t() { consume_fixture(DIR); }\n",
        ),
        // A same-named function in an unrelated module does not lend its body.
        (
            "namesake-in-other-module",
            "mod unrelated { pub fn dir() { let d = consume_fixture(\"tests/ui\"); } }\n\
             fn dir() {}\n#[test]\nfn t() { dir(); }\n",
        ),
        (
            "namesake-in-sibling-module",
            "mod a { pub fn dir() { let d = consume_fixture(\"tests/ui\"); } }\n\
             mod b { pub fn dir() {} #[test] fn t() { dir(); } }\n",
        ),
        (
            "not-a-test",
            "mod helpers { pub fn dir() { let d = consume_fixture(\"tests/ui\"); } }\nfn main_like() {}\n",
        ),
        // A same-named decoy elsewhere in the file, reached by its own
        // qualified path, does not lend the reviewed helper's proof: the
        // reviewed helper exists (so the file-wide check alone would wrongly
        // pass) but is never actually called.
        (
            "decoy-consume-fixture-reached-by-qualified-path",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             mod fake {\n    pub fn consume_fixture(_: &str) -> u8 { 0 }\n}\n\
             #[test]\nfn t() { let d = fake::consume_fixture(\"tests/ui\"); }\n",
        ),
        // A nested fn-local `consume_fixture` shadows the outer, reviewed
        // one for every bare reference inside its own body, exactly as real
        // Rust scoping requires: the call inside `t` binds to the local
        // no-op, never to the outer helper, regardless of the outer one's
        // own reviewed shape.
        (
            "locally-shadowed-no-op-helper",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             #[test]\nfn t() {\n    fn consume_fixture(_: &str) -> u8 { 0 }\n    \
             let d = consume_fixture(\"tests/ui\");\n}\n",
        ),
        // Same shadowing rule, a `let` binding instead of a nested fn.
        (
            "locally-shadowed-via-let-binding",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             #[test]\nfn t() {\n    let consume_fixture = |_: &str| 0u8;\n    \
             let d = consume_fixture(\"tests/ui\");\n}\n",
        ),
        // Same shadowing rule, a function parameter instead of a nested fn.
        (
            "locally-shadowed-via-parameter",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             fn dir(consume_fixture: u8) {\n    let d = consume_fixture(\"tests/ui\");\n}\n\
             #[test]\nfn t() { dir(0); }\n",
        ),
        // Same shadowing rule, bound through a destructuring pattern.
        (
            "locally-shadowed-via-destructuring",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             #[test]\nfn t() {\n    let (consume_fixture,) = (|_: &str| 0u8,);\n    \
             let d = consume_fixture(\"tests/ui\");\n}\n",
        ),
        // Same shadowing rule, a block-local `use` renaming onto the name.
        (
            "locally-shadowed-via-use-rename",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             mod fake {\n    pub fn other(_: &str) -> u8 { 0 }\n}\n\
             #[test]\nfn t() {\n    use fake::other as consume_fixture;\n    \
             let d = consume_fixture(\"tests/ui\");\n}\n",
        ),
        // Same shadowing rule, a local tuple-struct constructor: its own
        // value-namespace item is callable exactly like a function.
        (
            "locally-shadowed-via-tuple-struct",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             #[test]\nfn t() {\n    struct consume_fixture(&'static str);\n    \
             let d = consume_fixture(\"tests/ui\");\n}\n",
        ),
        // Same shadowing rule, a `for` loop's own pattern.
        (
            "locally-shadowed-via-for-loop-pattern",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             fn dir() {\n    for consume_fixture in [\"x\"] {\n        \
             let d = consume_fixture(\"tests/ui\");\n    }\n}\n\
             #[test]\nfn t() { dir(); }\n",
        ),
        // Same shadowing rule, a `match` arm's own pattern.
        (
            "locally-shadowed-via-match-arm-pattern",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             fn dir(x: u8) {\n    match x {\n        consume_fixture => {\n            \
             let d = consume_fixture(\"tests/ui\");\n        }\n    }\n}\n\
             #[test]\nfn t() { dir(0); }\n",
        ),
        // Same shadowing rule, an `if let` pattern.
        (
            "locally-shadowed-via-if-let-pattern",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             fn dir(x: Option<u8>) {\n    if let Some(consume_fixture) = x {\n        \
             let d = consume_fixture(\"tests/ui\");\n    }\n}\n\
             #[test]\nfn t() { dir(Some(0)); }\n",
        ),
        // Same shadowing rule, a `while let` pattern.
        (
            "locally-shadowed-via-while-let-pattern",
            "fn consume_fixture(relative: &str) -> std::path::PathBuf {\n    \
             std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n\
             fn dir(mut it: std::vec::IntoIter<u8>) {\n    \
             while let Some(consume_fixture) = it.next() {\n        \
             let d = consume_fixture(\"tests/ui\");\n    }\n}\n\
             #[test]\nfn t() { dir(vec![0].into_iter()); }\n",
        ),
        // A doc attribute (or any other attribute) is not the item's body: it
        // must never be able to forge the reviewed shape by merely quoting
        // the three required substrings.
        (
            "doc-attribute-forged-reviewed-shape",
            "#[doc = \"CARGO_MANIFEST_DIR join ->\"]\n\
             fn consume_fixture(_: &str) -> u8 { 0 }\n\
             #[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
        ),
        // A path used as a value -- coerced to a function pointer, passed
        // around, stored -- is never itself an invocation: `consumer`'s own
        // body must never be credited unless something actually calls it.
        (
            "referenced-but-never-called",
            "fn consumer() { let d = consume_fixture(\"tests/ui\"); }\n\
             #[test]\nfn t() { let _f: fn() = consumer; }\n",
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
        (
            "file-level-cfg-this-host",
            "#![cfg(all(target_os = \"linux\", target_arch = \"x86_64\", target_env = \"gnu\"))]\nfn consume_fixture(relative: &str) -> std::path::PathBuf {\n    std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\")).join(relative)\n}\n#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "direct",
            "#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
        ),
        (
            "helper",
            "fn dir() { let d = consume_fixture(\"tests/ui\"); }\n#[test]\nfn t() { dir(); }\n",
        ),
        (
            "qualified-module-path",
            // `dir` itself reaches the outer, reviewed `consume_fixture`
            // through `super::`, exactly as real Rust name resolution
            // requires from inside a child module (a bare, unqualified
            // reference here would not compile, and does not resolve).
            "mod h { pub fn dir() { let d = super::consume_fixture(\"tests/ui\"); } }\n\
             #[test]\nfn t() { h::dir(); }\n",
        ),
        (
            "glob-import-of-parent",
            "fn dir() { let d = consume_fixture(\"tests/ui\"); }\n\
             #[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn t() { dir(); }\n}\n",
        ),
        (
            "named-import",
            "mod h { pub fn dir() { let d = super::consume_fixture(\"tests/ui\"); } }\n\
             use h::dir;\n#[test]\nfn t() { dir(); }\n",
        ),
        (
            "macro-arguments",
            "#[test]\nfn t() { assert!(consume_fixture(\"./tests/ui/\")); }\n",
        ),
        (
            "test-module",
            // The standard `use super::*;` idiom every nested test module
            // needs to see the outer scope at all; a bare reference here
            // would not otherwise compile, and does not resolve.
            "#[cfg(test)]\nmod inner {\n    use super::*;\n    #[test]\n    fn t() { let d = consume_fixture(\"tests/ui\"); }\n}\n",
        ),
        (
            "if-true",
            "#[test]\nfn t() { if true { let d = consume_fixture(\"tests/ui\"); } }\n",
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
fn a_test_target_switched_off_is_not_a_consumer() {
    let repo = fixture_repo("witness-test-false", "test-fixture", "tests/ui.rs", "");
    repo.write(
        "tests/ui.rs",
        "#[test]\nfn t() { let d = consume_fixture(\"tests/ui\"); }\n",
    );
    cargo_metadata(&repo.root).unwrap();
    let manifest = fs::read_to_string(repo.root.join("Cargo.toml")).unwrap();
    fs::write(
        repo.root.join("Cargo.toml"),
        manifest.replace(
            "[workspace]",
            "[[test]]\nname = \"ui\"\ntest = false\n\n[workspace]",
        ),
    )
    .unwrap();
    assert!(refused(&repo).contains("not a test target of a contract package"));
}

#[test]
fn a_test_consumer_that_does_not_parse_is_refused() {
    let repo = fixture_repo("witness-unparsed", "test-fixture", "tests/ui.rs", "");
    repo.write("tests/ui.rs", "fn (\n");
    assert!(refused(&repo).contains("does not parse as Rust"));
}

/// A repository whose fixture `recipes/fixture/case` is excluded.
fn recipe_repo(label: &str, recipe: &str) -> Repo {
    let repo = Repo::new(label);
    repo.nested_workspace("recipes/fixture", &["case"]);
    repo.package("recipes/fixture/case", "case", "publish = false\n");
    repo.write("recipes/use/recipe.toml", recipe);
    repo.write("recipes/book.toml", "fixtures = [\"recipes/fixture\"]\n");
    repo.root_package(
        "app",
        "contract-exclusions = [{ path = \"recipes/fixture\", class = \"recipe-fixture\", \
         consumer = \"recipes/use/recipe.toml\", reason = \"cases\" }]",
    );
    repo
}

#[test]
fn only_the_typed_fixtures_field_of_a_recipe_consumes_a_recipe_fixture() {
    for (label, recipe) in [
        ("comment", "# recipes/fixture\ntitle = \"x\"\n"),
        ("note", "title = \"x\"\nnote = \"recipes/fixture\"\n"),
        (
            "prose",
            "title = \"x\"\nsummary = \"uses recipes/fixture heavily\"\n",
        ),
        ("key", "title = \"x\"\n\"recipes/fixture\" = 1\n"),
        ("table", "title = \"x\"\n[\"recipes/fixture\"]\nk = 1\n"),
        (
            "other-field",
            "title = \"x\"\nrequires = [\"recipes/fixture\"]\n",
        ),
        (
            "nested-fixtures",
            "title = \"x\"\n[extra]\nfixtures = [\"recipes/fixture\"]\n",
        ),
        (
            "unrelated",
            "title = \"x\"\nfixtures = [\"recipes/other\"]\n",
        ),
        ("empty", "title = \"x\"\nfixtures = []\n"),
    ] {
        let repo = recipe_repo(&format!("witness-recipe-dead-{label}"), recipe);
        let err = refused(&repo);
        assert!(
            err.contains("is not consumed by recipes/use/recipe.toml"),
            "{label}: {err}"
        );
    }
    for (label, recipe) in [
        (
            "repo-relative",
            "title = \"x\"\nfixtures = [\"recipes/fixture\"]\n",
        ),
        (
            "trailing-slash",
            "title = \"x\"\nfixtures = [\"a\", \"recipes/fixture/\"]\n",
        ),
        (
            "consumer-relative",
            "title = \"x\"\nfixtures = [\"../fixture\"]\n",
        ),
    ] {
        let repo = recipe_repo(&format!("witness-recipe-live-{label}"), recipe);
        if label == "consumer-relative" {
            // `..` is not a plain relative path: refused, never normalized.
            assert!(refused(&repo).contains("is not consumed"), "{label}");
        } else {
            cargo_metadata(&repo.root).unwrap_or_else(|err| panic!("{label}: {err}"));
        }
    }
    let repo = recipe_repo(
        "witness-recipe-typed",
        "title = \"x\"\nfixtures = \"recipes/fixture\"\n",
    );
    assert!(refused(&repo).contains("`fixtures` must be an array"));
    let repo = recipe_repo(
        "witness-recipe-typed-entry",
        "title = \"x\"\nfixtures = [3]\n",
    );
    assert!(refused(&repo).contains("must be a path string"));
    let repo = recipe_repo("witness-recipe-broken", "title = \n");
    assert!(refused(&repo).contains("does not parse as TOML"));
    // A book manifest is not a recipe: it cannot consume a fixture.
    let repo = recipe_repo(
        "witness-recipe-book",
        "title = \"x\"\nfixtures = [\"recipes/fixture\"]\n",
    );
    repo.root_package(
        "app",
        "contract-exclusions = [{ path = \"recipes/fixture\", class = \"recipe-fixture\", \
         consumer = \"recipes/book.toml\", reason = \"cases\" }]",
    );
    assert!(refused(&repo).contains("not a `recipe.toml`"));
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
fn a_harness_needs_a_declared_command_that_runs_it_and_a_target_cargo_discovers() {
    let repo = harness_repo("witness-harness-ok", HARNESS, &[HARNESS], true);
    cargo_metadata(&repo.root).unwrap();

    let undeclared = harness_repo("witness-harness-undeclared", HARNESS, &["cargo test"], true);
    assert!(refused(&undeclared).contains("not one of the root manifest's `validation-commands`"));

    // Commands that do not execute the harness, or that filter what runs.
    for (label, extra) in [
        ("no-run", " --no-run"),
        ("filter", " some_test"),
        ("target-select", " --test run"),
        ("lib-only", " --lib"),
        ("package", " -p case"),
        ("program-args", " -- --ignored"),
        ("list", " -- --list"),
        ("chained", " && true"),
        (
            "duplicate-manifest",
            " --manifest-path tests/ui/case/Cargo.toml",
        ),
    ] {
        let command = format!("{HARNESS}{extra}");
        let repo = harness_repo(
            &format!("witness-harness-{label}"),
            &command,
            &[&command],
            true,
        );
        assert!(
            refused(&repo).contains("not a plain `cargo test` or `cargo run`"),
            "{label}"
        );
    }
    for accepted in [" --locked", " --offline --locked", " --release", " -q"] {
        let command = format!("{HARNESS}{accepted}");
        let repo = harness_repo("witness-harness-flags", &command, &[&command], true);
        cargo_metadata(&repo.root).unwrap_or_else(|err| panic!("{accepted}: {err}"));
    }

    let other = "cargo test --manifest-path tests/other/Cargo.toml";
    let repo = harness_repo("witness-harness-other", other, &[other], true);
    assert!(refused(&repo).contains("not a manifest of the excluded harness"));

    let lib_only = harness_repo("witness-harness-lib-only", HARNESS, &[HARNESS], false);
    assert!(refused(&lib_only).contains("has no test target"));

    // Only the exact layout counts: a nested `.rs` is no test target, and
    // `autotests = false` turns the automatic layout off.
    let nested = harness_repo("witness-harness-nested", HARNESS, &[HARNESS], false);
    nested.write("tests/ui/case/tests/helpers/util.rs", "pub fn util() {}\n");
    assert!(refused(&nested).contains("has no test target"));
    let no_auto = harness_repo("witness-harness-noauto", HARNESS, &[HARNESS], true);
    no_auto.package(
        "tests/ui/case",
        "case",
        "publish = false\nautotests = false\n\n[dependencies]\napp = { path = \"../../..\" }\n",
    );
    assert!(refused(&no_auto).contains("has no test target"));
    no_auto.package(
        "tests/ui/case",
        "case",
        "publish = false\nautotests = false\n\n[[test]]\nname = \"run\"\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    cargo_metadata(&no_auto.root).unwrap();
    // A declared target that the plain command does not run, or that replaces
    // the automatic one of its name, is not a target.
    for (label, extra) in [
        (
            "required-features",
            "[features]\nnever = []\n\n[[test]]\nname = \"run\"\nrequired-features = [\"never\"]\n",
        ),
        ("test-false", "[[test]]\nname = \"run\"\ntest = false\n"),
    ] {
        let repo = harness_repo(
            &format!("witness-harness-{label}"),
            HARNESS,
            &[HARNESS],
            true,
        );
        repo.package(
            "tests/ui/case",
            "case",
            &format!(
                "publish = false\n\n[dependencies]\napp = {{ path = \"../../..\" }}\n\n{extra}"
            ),
        );
        assert!(refused(&repo).contains("has no test target"), "{label}");
    }
    let subdir = harness_repo("witness-harness-subdir", HARNESS, &[HARNESS], false);
    subdir.write("tests/ui/case/tests/run/main.rs", "#[test]\nfn run() {}\n");
    cargo_metadata(&subdir.root).unwrap();

    let run = "cargo run --manifest-path tests/ui/case/Cargo.toml";
    // Several binaries and no `default-run`: Cargo refuses to pick one.
    let two = harness_repo("witness-harness-two-bins", run, &[run], false);
    two.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    two.write("tests/ui/case/src/bin/other.rs", "fn main() {}\n");
    assert!(refused(&two).contains("has no binary target"));
    let tests_only = harness_repo("witness-harness-run-tests", run, &[run], true);
    assert!(refused(&tests_only).contains("has no binary target"));
    let binary = harness_repo("witness-harness-run-bin", run, &[run], false);
    binary.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    cargo_metadata(&binary.root).unwrap();

    // `default-run` names a required-features-gated binary; a plain
    // `cargo run` would try to run exactly that one and fail, even though
    // the package's other binary is fine on its own.
    let gated_default = harness_repo("witness-harness-gated-default-run", run, &[run], false);
    gated_default.package(
        "tests/ui/case",
        "case",
        "publish = false\ndefault-run = \"case\"\n\n\
         [features]\nnever = []\n\n\
         [[bin]]\nname = \"case\"\npath = \"src/main.rs\"\nrequired-features = [\"never\"]\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    gated_default.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    gated_default.write("tests/ui/case/src/bin/other.rs", "fn main() {}\n");
    assert!(refused(&gated_default).contains("has no binary target"));

    // `default-run` names the package's only runnable binary: accepted, even
    // though it is declared with an explicit (ungated) `[[bin]]` entry.
    let runnable_default = harness_repo("witness-harness-runnable-default-run", run, &[run], false);
    runnable_default.package(
        "tests/ui/case",
        "case",
        "publish = false\ndefault-run = \"case\"\n\n\
         [[bin]]\nname = \"case\"\npath = \"src/main.rs\"\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    runnable_default.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    runnable_default.write("tests/ui/case/src/bin/other.rs", "fn main() {}\n");
    cargo_metadata(&runnable_default.root).unwrap();

    // A declared [[bin]] replacing the automatic one of the same name is
    // one binary, not two: no default-run needed.
    let replaced = harness_repo("witness-harness-replaced-automatic", run, &[run], false);
    replaced.package(
        "tests/ui/case",
        "case",
        "publish = false\n\n[[bin]]\nname = \"case\"\npath = \"src/main.rs\"\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    replaced.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    cargo_metadata(&replaced.root).unwrap();

    // autobins = false with only a declared binary: no default-run needed,
    // even though there is also an (ignored) automatic-layout file.
    let no_autobins = harness_repo("witness-harness-no-autobins", run, &[run], false);
    no_autobins.package(
        "tests/ui/case",
        "case",
        "publish = false\nautobins = false\n\n\
         [[bin]]\nname = \"case\"\npath = \"src/main.rs\"\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    no_autobins.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    no_autobins.write("tests/ui/case/src/bin/other.rs", "fn main() {}\n");
    cargo_metadata(&no_autobins.root).unwrap();

    // Two declared [[bin]] entries of the same name: real Cargo refuses
    // this manifest outright, so the witness must too, not silently
    // deduplicate them into one name and undercount.
    let duplicate_bin = harness_repo("witness-harness-duplicate-bin-name", run, &[run], false);
    duplicate_bin.package(
        "tests/ui/case",
        "case",
        "publish = false\ndefault-run = \"case\"\n\n\
         [[bin]]\nname = \"case\"\npath = \"src/main.rs\"\n\n\
         [[bin]]\nname = \"case\"\npath = \"src/bin/other.rs\"\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    duplicate_bin.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    duplicate_bin.write("tests/ui/case/src/bin/other.rs", "fn main() {}\n");
    assert!(refused(&duplicate_bin).contains("has no binary target"));

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
        cargo_invocation("cargo run --manifest-path=a/Cargo.toml"),
        Some(("run", "a/Cargo.toml".to_owned()))
    );
    for command in [
        "cargo test",
        "cargo test --no-run --manifest-path a/Cargo.toml",
        "cargo test --manifest-path a/Cargo.toml filter",
        "cargo test --manifest-path a/Cargo.toml -- --ignored",
        "cargo test --manifest-path a/Cargo.toml --manifest-path b/Cargo.toml",
        "cargo doc --manifest-path a/Cargo.toml",
        "sh cargo test --manifest-path a/Cargo.toml",
        "cargo test --manifest-path a/Cargo.toml; rm x",
        "cargo test --manifest-path \"a/Cargo.toml\"",
        "cargo test --manifest-path $X/Cargo.toml",
    ] {
        assert_eq!(cargo_invocation(command), None, "{command}");
    }
}
