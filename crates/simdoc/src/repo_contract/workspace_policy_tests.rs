// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    env, fs,
    os::unix::fs::symlink,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;

use super::*;
use crate::repo_contract::{cargo_metadata, contract_packages, workspace_package_names};

// conformance: a repo contract covers every Cargo manifest its Git worktree
// owns exactly once; exclusions are typed, proved, and never hide first-party
// code; and nothing covered is outside the worktree's ordinary files.

#[test]
fn contract_workspaces_default_to_the_root_workspace() {
    assert!(declared_contract_workspaces(&json!({})).unwrap().is_empty());
    assert!(contract_exclusions(&json!({})).unwrap().is_empty());
    assert_eq!(
        declared_contract_workspaces(&json!({
            "metadata": {"sim": {"contract-workspaces": ["crates", "tools/pack"]}}
        }))
        .unwrap(),
        ["crates", "tools/pack"]
    );
}

#[test]
fn contract_workspaces_refuse_invalid_declarations() {
    let too_many = (0..=MAX_CONTRACT_WORKSPACES)
        .map(|index| format!("w{index}"))
        .collect::<Vec<_>>();
    for (declared, expected) in [
        (json!("crates"), "must be an array"),
        (json!([1]), "entries must be strings"),
        (json!([""]), "plain repository-relative"),
        (json!(["../other"]), "plain repository-relative"),
        (json!(["/abs"]), "plain repository-relative"),
        (json!(["./crates"]), "plain repository-relative"),
        (json!(["crates", "crates"]), "declared twice"),
        (json!(too_many), "contract workspaces are declared"),
    ] {
        let metadata = json!({"metadata": {"sim": {"contract-workspaces": declared}}});
        let err = declared_contract_workspaces(&metadata).unwrap_err();
        assert!(err.contains(expected), "{declared}: {err}");
    }
}

#[test]
fn contract_exclusions_are_typed_and_reasoned() {
    let metadata = json!({"metadata": {"sim": {"contract-exclusions": [
        {"path": "tests/ui", "class": "test-fixture", "reason": "compile-fail fixtures"}
    ]}}});
    assert_eq!(
        contract_exclusions(&metadata).unwrap(),
        [ContractExclusion {
            path: "tests/ui".to_owned(),
            class: ExclusionClass::TestFixture,
            reason: "compile-fail fixtures".to_owned(),
        }]
    );
    let entry = |path: &str, class: &str, reason: &str| json!({"path": path, "class": class, "reason": reason});
    for (declared, expected) in [
        (json!({"path": "tests"}), "must be an array"),
        (json!(["tests"]), "must be a table"),
        (
            json!([{"path": "t", "class": "test-fixture", "reason": "r", "extra": 1}]),
            "unexpected key `extra`",
        ),
        (
            json!([{"class": "test-fixture", "reason": "r"}]),
            "needs a string `path`",
        ),
        (
            json!([entry("../x", "test-fixture", "r")]),
            "plain repository-relative",
        ),
        (json!([{"path": "tests", "reason": "r"}]), "needs a class"),
        (json!([entry("tests", "fixture", "r")]), "needs a class"),
        (
            json!([entry("tests", "test-fixture", "  ")]),
            "non-empty reason",
        ),
        (
            json!([
                entry("t", "test-fixture", "a"),
                entry("t", "test-fixture", "b")
            ]),
            "declared twice",
        ),
    ] {
        let metadata = json!({"metadata": {"sim": {"contract-exclusions": declared}}});
        let err = contract_exclusions(&metadata).unwrap_err();
        assert!(err.contains(expected), "{declared}: {err}");
    }
}

#[test]
fn declared_nested_workspace_joins_the_contract() {
    let repo = Repo::new("policy-nested");
    repo.root_package("app", r#"contract-workspaces = ["nested"]"#);
    repo.nested_workspace("nested", &["tool"]);
    repo.package("nested/tool", "tool", "");

    let contract = contract_packages(&repo.root).unwrap();
    let names = workspace_package_names(&contract.metadata).unwrap();
    assert_eq!(names.into_iter().collect::<Vec<_>>(), ["app", "tool"]);
    let owner = |name: &str| {
        let package = contract.metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == name)
            .unwrap();
        contract.workspace_of(package).map(str::to_owned)
    };
    assert_eq!(owner("app"), None);
    assert_eq!(owner("tool").as_deref(), Some("nested"));
    assert!(contract.exclusions.is_empty());
}

#[test]
fn contract_workspaces_refuse_a_package_in_two_workspaces() {
    let repo = Repo::new("policy-duplicate");
    repo.root_package("dup", r#"contract-workspaces = ["nested"]"#);
    repo.nested_workspace("nested", &["dup"]);
    repo.package("nested/dup", "dup", "");

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("more than one contract workspace"), "{err}");
}

#[test]
fn declared_member_directory_is_not_a_workspace_root() {
    let repo = Repo::new("policy-member-root");
    repo.root_package("app", r#"contract-workspaces = ["nested/tool"]"#);
    repo.nested_workspace("nested", &["tool"]);
    repo.package("nested/tool", "tool", "");

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("is not a Cargo workspace root"), "{err}");
}

#[test]
fn every_owned_manifest_is_covered_or_excluded_and_projected() {
    let repo = Repo::new("policy-complete");
    repo.root_package("app", "");
    repo.nested_workspace("tests/ui", &["case"]);
    repo.package("tests/ui/case", "case", "publish = false\n");

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("unclassified Cargo manifests: tests/ui/Cargo.toml, tests/ui/case/Cargo.toml"),
        "{err}"
    );

    repo.root_package(
        "app",
        r#"contract-exclusions = [{ path = "tests/ui", class = "test-fixture", reason = "compile-fail cases" }]"#,
    );
    let contract = contract_packages(&repo.root).unwrap();
    let names = workspace_package_names(&contract.metadata).unwrap();
    assert_eq!(names.into_iter().collect::<Vec<_>>(), ["app"]);
    assert_eq!(
        contract
            .exclusions
            .iter()
            .map(ExcludedManifests::projection)
            .collect::<Vec<_>>(),
        [json!({
            "path": "tests/ui",
            "class": "test-fixture",
            "reason": "compile-fail cases",
            "manifests": ["tests/ui/Cargo.toml", "tests/ui/case/Cargo.toml"],
        })]
    );

    repo.root_package(
        "app",
        r#"contract-exclusions = [
    { path = "tests/ui", class = "test-fixture", reason = "compile-fail cases" },
    { path = "tests/gone", class = "test-fixture", reason = "stale" },
]"#,
    );
    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("contract exclusion tests/gone matches no Cargo manifest"),
        "{err}"
    );
}

#[test]
fn an_exclusion_may_not_hide_a_contract_manifest() {
    let repo = Repo::new("policy-overlap");
    repo.root_package(
        "app",
        r#"contract-workspaces = ["nested"]
contract-exclusions = [{ path = "nested", class = "focused-test-harness", reason = "overlaps" }]"#,
    );
    repo.nested_workspace("nested", &["tool"]);
    repo.package("nested/tool", "tool", "");

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("contract exclusion nested covers contract manifest nested/Cargo.toml"),
        "{err}"
    );
}

#[test]
fn an_exclusion_class_needs_its_native_evidence() {
    let repo = Repo::new("policy-class-evidence");
    repo.nested_workspace("fixtures/ui", &["case"]);
    repo.package("fixtures/ui/case", "case", "publish = false\n");
    repo.root_package(
        "app",
        r#"contract-exclusions = [{ path = "fixtures", class = "test-fixture", reason = "cases" }]"#,
    );
    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("has no `tests` path component"), "{err}");

    repo.root_package(
        "app",
        r#"contract-exclusions = [{ path = "fixtures", class = "recipe-fixture", reason = "cases" }]"#,
    );
    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("has no `recipes` path component"), "{err}");

    repo.root_package(
        "app",
        r#"contract-exclusions = [{ path = "fixtures", class = "focused-test-harness", reason = "cases" }]"#,
    );
    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("has no path dependency on a contract package"),
        "{err}"
    );

    repo.package(
        "fixtures/ui/case",
        "case",
        "publish = false\n\n[dependencies]\napp = { path = \"../../..\" }\n",
    );
    let contract = contract_packages(&repo.root).unwrap();
    assert_eq!(
        contract.exclusions[0].exclusion.class,
        ExclusionClass::FocusedTestHarness
    );
}

#[test]
fn a_publishable_package_can_never_be_excluded() {
    let repo = Repo::new("policy-publishable");
    repo.nested_workspace("tests/real", &["real"]);
    repo.package("tests/real/real", "real", "");
    repo.root_package(
        "app",
        r#"contract-exclusions = [{ path = "tests/real", class = "test-fixture", reason = "fixture" }]"#,
    );

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("tests/real/real/Cargo.toml is publishable"),
        "{err}"
    );
}

#[test]
fn a_contract_package_may_not_depend_on_excluded_code() {
    let repo = Repo::new("policy-first-party");
    repo.package("tests/helper", "helper", "publish = false\n");
    repo.root_package_with(
        "app",
        r#"contract-exclusions = [{ path = "tests/helper", class = "test-fixture", reason = "helper" }]"#,
        "[dependencies]\nhelper = { path = \"tests/helper\" }\n",
    );

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("first-party code cannot be excluded"), "{err}");
}

#[test]
fn a_declared_workspace_reached_through_a_symlink_is_refused() {
    let repo = Repo::new("policy-symlink-root");
    let outside = temp_root("policy-symlink-outside");
    fs::write(
        outside.join("Cargo.toml"),
        "[workspace]\nmembers = [\"tool\"]\n",
    )
    .unwrap();
    write_package(&outside.join("tool"), "tool", "");
    symlink(&outside, repo.root.join("linked")).unwrap();
    repo.root_package("app", r#"contract-workspaces = ["linked"]"#);

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("contract workspace linked is reached through a symlink"),
        "{err}"
    );

    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn a_member_or_target_outside_the_repository_is_refused() {
    let repo = Repo::new("policy-member-outside");
    let outside = temp_root("policy-member-outside-src");
    write_package(&outside, "tool", "");
    repo.root_package("app", r#"contract-workspaces = ["nested"]"#);
    repo.nested_workspace("nested", &["tool"]);
    symlink(&outside, repo.root.join("nested/tool")).unwrap();

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("package manifest"), "{err}");
    assert!(err.contains("resolves outside the repository"), "{err}");

    let repo = Repo::new("policy-target-outside");
    fs::write(outside.join("external.rs"), "").unwrap();
    repo.write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [lib]\npath = \"{}\"\n\n[workspace]\n",
            outside.join("external.rs").display()
        ),
    );
    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("package target source"), "{err}");
    assert!(err.contains("resolves outside the repository"), "{err}");

    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn an_ignored_workspace_or_source_is_not_owned() {
    let repo = Repo::new("policy-ignored");
    repo.root_package("app", r#"contract-workspaces = ["nested"]"#);
    repo.nested_workspace("nested", &["tool"]);
    repo.package("nested/tool", "tool", "");
    repo.write(".gitignore", "/nested/\n");

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains(
            "contract workspace manifest nested/Cargo.toml is not a file of this Git worktree"
        ),
        "{err}"
    );

    let repo = Repo::new("policy-ignored-source");
    repo.root_package("app", "");
    repo.write(".gitignore", "/src/lib.rs\n");
    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("package target source src/lib.rs is not a file of this Git worktree"),
        "{err}"
    );
}

#[test]
fn a_symlinked_source_inside_the_repository_is_not_an_ordinary_file() {
    let repo = Repo::new("policy-symlinked-source");
    repo.root_package("app", "");
    repo.write("src/real.rs", "");
    fs::remove_file(repo.root.join("src/lib.rs")).unwrap();
    symlink("real.rs", repo.root.join("src/lib.rs")).unwrap();

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("src/lib.rs is not an ordinary file"), "{err}");
}

#[test]
fn a_nested_repository_or_submodule_is_not_owned() {
    for as_gitlink in [false, true] {
        let repo = Repo::new("policy-nested-repo");
        repo.root_package("app", r#"contract-workspaces = ["nested"]"#);
        repo.nested_workspace("nested", &["tool"]);
        repo.package("nested/tool", "tool", "");
        let nested = repo.root.join("nested");
        git(&nested, &["init", "--quiet"]);
        if as_gitlink {
            git(&nested, &["add", "-A"]);
            git(
                &nested,
                &[
                    "-c",
                    "user.name=fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "--no-verify",
                    "-m",
                    "fixture",
                ],
            );
            git(&repo.root, &["add", "nested"]);
        }

        let err = cargo_metadata(&repo.root).unwrap_err();
        assert!(
            err.contains("nested/Cargo.toml is not a file of this Git worktree"),
            "gitlink {as_gitlink}: {err}"
        );
    }
}

#[test]
fn a_repository_inside_another_worktree_or_workspace_is_refused() {
    let outer = temp_root("policy-outer");
    fs::write(
        outer.join("Cargo.toml"),
        "[workspace]\nmembers = [\"inner\"]\n",
    )
    .unwrap();
    write_package(&outer.join("inner"), "inner", "");
    git(&outer.join("inner"), &["init", "--quiet"]);
    let err = cargo_metadata(&outer.join("inner")).unwrap_err();
    assert!(
        err.contains("is not the root of its Cargo workspace"),
        "{err}"
    );
    fs::remove_dir_all(outer).unwrap();

    let outer = Repo::new("policy-outer-worktree");
    write_package(&outer.root.join("inner"), "inner", "");
    let err = cargo_metadata(&outer.root.join("inner")).unwrap_err();
    assert!(err.contains("is not the root of its Git worktree"), "{err}");
}

#[test]
fn classification_requires_a_git_worktree() {
    let root = temp_root("policy-no-git");
    write_package(&root, "app", "");

    let err = cargo_metadata(&root).unwrap_err();
    assert!(err.contains("cannot classify Cargo manifests"), "{err}");

    fs::remove_dir_all(root).unwrap();
}

struct Repo {
    root: PathBuf,
}

impl Repo {
    fn new(name: &str) -> Self {
        let root = temp_root(name);
        git(&root, &["init", "--quiet"]);
        Self { root }
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn root_package(&self, name: &str, sim_metadata: &str) {
        self.root_package_with(name, sim_metadata, "");
    }

    fn root_package_with(&self, name: &str, sim_metadata: &str, extra: &str) {
        self.write("src/lib.rs", "");
        self.write(
            "Cargo.toml",
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n{extra}\n\
                 [workspace]\nexclude = [\"nested\", \"fixtures\", \"linked\", \"tests\"]\n\n\
                 [workspace.metadata.sim]\n{sim_metadata}\n"
            ),
        );
    }

    fn nested_workspace(&self, relative: &str, members: &[&str]) {
        let members = members
            .iter()
            .map(|member| format!("\"{member}\""))
            .collect::<Vec<_>>()
            .join(", ");
        self.write(
            &format!("{relative}/Cargo.toml"),
            &format!("[workspace]\nmembers = [{members}]\n"),
        );
    }

    fn package(&self, relative: &str, name: &str, extra: &str) {
        write_package(&self.root.join(relative), name, extra);
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_package(dir: &Path, name: &str, extra: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), "").unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{extra}"),
    )
    .unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
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
