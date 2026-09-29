// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;

// conformance: the workflow reader builds the tree the YAML means or refuses.

fn scalar(text: &str) -> Yaml {
    Yaml::Scalar(text.to_owned())
}

#[test]
fn a_real_workflow_shape_parses_to_its_tree() {
    let yaml = "name: CI\non:\n  push:\n    branches: [\"**\"]\n  pull_request:\n\
                jobs:\n  t:\n    runs-on: ubuntu-latest\n    needs: [scope, heavy]\n    steps:\n\
                \x20     - uses: actions/checkout@v4\n        with:\n          path: x\n\
                \x20     - run: |\n          echo one\n          echo two\n\
                \x20     - run: cargo test # trailing comment\n        env:\n          A: ${{ github.workspace }}/b\n";
    let tree = parse(yaml).unwrap();
    assert_eq!(tree.get("name"), Some(&scalar("CI")));
    let on = tree.get("on").unwrap();
    assert!(on.get("push").is_some() && on.get("pull_request").is_some());
    let steps = tree.get("jobs").unwrap().get("t").unwrap().get("steps");
    let Some(Yaml::Seq(steps)) = steps else {
        panic!("{steps:?}")
    };
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[1].get("run"), Some(&scalar("echo one\necho two")));
    assert_eq!(steps[2].get("run"), Some(&scalar("cargo test")));
    assert_eq!(
        steps[2].get("env").and_then(|env| env.get("A")),
        Some(&scalar("${{ github.workspace }}/b"))
    );
    assert_eq!(
        tree.get("jobs").unwrap().get("t").unwrap().get("needs"),
        Some(&Yaml::Seq(vec![scalar("scope"), scalar("heavy")]))
    );
}

#[test]
fn comments_are_lexed_away_never_searched() {
    let tree = parse("# on: push\nname: x # on: push\nkey: \"a # b\"\nrun: 'x # y'\n").unwrap();
    assert!(tree.get("on").is_none());
    assert_eq!(tree.get("name"), Some(&scalar("x")));
    assert_eq!(tree.get("key"), Some(&scalar("a # b")));
    assert_eq!(tree.get("run"), Some(&scalar("x # y")));
}

#[test]
fn anything_unsupported_is_refused() {
    for (label, source) in [
        ("tab", "a:\n\tb: 1\n"),
        ("anchor", "a: &x 1\n"),
        ("alias", "a: *x\n"),
        ("tag", "a: !!str 1\n"),
        ("flow-map", "a: {b: 1}\n"),
        ("second-document", "a: 1\n---\nb: 2\n"),
        ("duplicate-key", "a: 1\na: 2\n"),
        ("bad-indent", "a:\n  b: 1\n c: 2\n"),
        ("complex-key", "? a\n: b\n"),
        ("unterminated-quote", "a: \"x\n"),
        ("empty", "# only a comment\n"),
    ] {
        assert!(parse(source).is_err(), "{label}");
    }
}

#[test]
fn a_sequence_may_sit_at_its_keys_indent() {
    let tree = parse("steps:\n- run: a\n- run: b\nother: 1\n").unwrap();
    let Some(Yaml::Seq(steps)) = tree.get("steps") else {
        panic!()
    };
    assert_eq!(steps.len(), 2);
    assert_eq!(tree.get("other"), Some(&scalar("1")));
}
