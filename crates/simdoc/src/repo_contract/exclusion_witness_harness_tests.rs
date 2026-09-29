// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;

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
    // A declared `[[test]]` naming a source file that does not exist is a
    // manifest real Cargo refuses outright, even though an automatic test
    // target elsewhere in the SAME package is perfectly valid on its own:
    // that other target's existence must never prove the package runs.
    let missing_source = harness_repo(
        "witness-harness-missing-test-source",
        HARNESS,
        &[HARNESS],
        true,
    );
    missing_source.package(
        "tests/ui/case",
        "case",
        "publish = false\n\n[[test]]\nname = \"missing\"\npath = \"tests/missing.rs\"\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    assert!(refused(&missing_source).contains("has no test target"));

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

    // A missing declared-target source refuses the manifest for EVERY verb,
    // not only the one whose own target kind it belongs to: a valid binary
    // does not excuse a missing test source when proving `cargo run`.
    let missing_test_for_run =
        harness_repo("witness-harness-missing-test-for-run", run, &[run], false);
    missing_test_for_run.package(
        "tests/ui/case",
        "case",
        "publish = false\n\n[[test]]\nname = \"missing\"\npath = \"tests/missing.rs\"\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    missing_test_for_run.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    assert!(refused(&missing_test_for_run).contains("has no binary target"));

    // The same is true of every OTHER declared target kind Cargo itself
    // recognizes: a missing `[[example]]`, `[[bench]]`, or `[lib]` source
    // refuses the manifest too, not only bin/test.
    for (label, extra) in [
        (
            "example",
            "[[example]]\nname = \"missing\"\npath = \"examples/missing.rs\"\n",
        ),
        (
            "bench",
            "[[bench]]\nname = \"missing\"\npath = \"benches/missing.rs\"\n",
        ),
        ("lib", "[lib]\npath = \"src/missing_lib.rs\"\n"),
    ] {
        let repo = harness_repo(
            &format!("witness-harness-missing-{label}-source"),
            run,
            &[run],
            false,
        );
        repo.package(
            "tests/ui/case",
            "case",
            &format!(
                "publish = false\n\n{extra}\n[dependencies]\napp = {{ path = \"../../..\" }}\n"
            ),
        );
        repo.write("tests/ui/case/src/main.rs", "fn main() {}\n");
        assert!(refused(&repo).contains("has no binary target"), "{label}");
    }

    // A declared target's `path` present but not a string is exactly as
    // missing as one that is absent and matches no convention: Cargo
    // requires it to be a string, and never falls back to the conventional
    // path when it is present but wrong.
    let non_string_path = harness_repo("witness-harness-non-string-path", run, &[run], false);
    non_string_path.package(
        "tests/ui/case",
        "case",
        "publish = false\n\n[[bin]]\nname = \"case\"\npath = 1\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    non_string_path.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    assert!(refused(&non_string_path).contains("has no binary target"));

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

    // An invalid `default-run` (naming a target that does not exist) is a
    // manifest error Cargo refuses regardless of how many binaries the
    // package actually has -- even a single, otherwise-unambiguous one.
    let bad_default_one_bin = harness_repo(
        "witness-harness-bad-default-run-one-bin",
        run,
        &[run],
        false,
    );
    bad_default_one_bin.package(
        "tests/ui/case",
        "case",
        "publish = false\ndefault-run = \"nonexistent\"\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    bad_default_one_bin.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    assert!(refused(&bad_default_one_bin).contains("has no binary target"));

    // `default-run` present but not a string is a manifest error Cargo
    // refuses too, never silently treated the same as the key being
    // absent (which, with one otherwise-valid binary, would wrongly
    // accept).
    let non_string_default =
        harness_repo("witness-harness-non-string-default-run", run, &[run], false);
    non_string_default.package(
        "tests/ui/case",
        "case",
        "publish = false\ndefault-run = 1\n\n\
         [dependencies]\napp = { path = \"../../..\" }\n",
    );
    non_string_default.write("tests/ui/case/src/main.rs", "fn main() {}\n");
    assert!(refused(&non_string_default).contains("has no binary target"));

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
