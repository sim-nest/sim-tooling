// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    env, fs,
    os::unix::fs::symlink,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;

use super::*;
use crate::repo_contract::{cargo_metadata, workspace_package_names};

// conformance: a repo contract covers every first-party Cargo manifest exactly
// once, and nothing it covers resolves outside the repository.

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
    for (declared, expected) in [
        (json!("crates"), "must be an array"),
        (json!([1]), "entries must be strings"),
        (json!([""]), "plain repository-relative"),
        (json!(["../other"]), "plain repository-relative"),
        (json!(["/abs"]), "plain repository-relative"),
        (json!(["./crates"]), "plain repository-relative"),
        (json!(["crates", "crates"]), "declared twice"),
    ] {
        let metadata = json!({"metadata": {"sim": {"contract-workspaces": declared}}});
        let err = declared_contract_workspaces(&metadata).unwrap_err();
        assert!(err.contains(expected), "{declared}: {err}");
    }
}

#[test]
fn contract_exclusions_require_a_plain_path_and_a_reason() {
    let metadata = json!({"metadata": {"sim": {"contract-exclusions": [
        {"path": "tests/ui", "reason": "compile-fail fixtures"}
    ]}}});
    assert_eq!(
        contract_exclusions(&metadata).unwrap(),
        [ContractExclusion {
            path: "tests/ui".to_owned(),
            reason: "compile-fail fixtures".to_owned(),
        }]
    );
    for (declared, expected) in [
        (json!({"path": "tests"}), "must be an array"),
        (json!([{"reason": "why"}]), "needs a string `path`"),
        (json!([{"path": "tests"}]), "non-empty reason"),
        (
            json!([{"path": "tests", "reason": "  "}]),
            "non-empty reason",
        ),
        (
            json!([{"path": "../x", "reason": "why"}]),
            "plain repository-relative",
        ),
        (
            json!([{"path": "t", "reason": "a"}, {"path": "t", "reason": "b"}]),
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
    repo.package("nested/tool", "tool");

    let names = workspace_package_names(&cargo_metadata(&repo.root).unwrap()).unwrap();
    assert_eq!(names.into_iter().collect::<Vec<_>>(), ["app", "tool"]);
}

#[test]
fn contract_workspaces_refuse_a_package_in_two_workspaces() {
    let repo = Repo::new("policy-duplicate");
    repo.root_package("dup", r#"contract-workspaces = ["nested"]"#);
    repo.nested_workspace("nested", &["dup"]);
    repo.package("nested/dup", "dup");

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("more than one contract workspace"), "{err}");
}

#[test]
fn declared_member_directory_is_not_a_workspace_root() {
    let repo = Repo::new("policy-member-root");
    repo.root_package("app", r#"contract-workspaces = ["nested/tool"]"#);
    repo.nested_workspace("nested", &["tool"]);
    repo.package("nested/tool", "tool");

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(err.contains("is not a Cargo workspace root"), "{err}");
}

#[test]
fn every_visible_manifest_is_covered_or_excluded_with_a_reason() {
    let repo = Repo::new("policy-complete");
    repo.root_package("app", "");
    repo.nested_workspace("fixtures/ui", &["case"]);
    repo.package("fixtures/ui/case", "case");

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains(
            "unclassified Cargo manifests: fixtures/ui/Cargo.toml, fixtures/ui/case/Cargo.toml"
        ),
        "{err}"
    );

    repo.root_package(
        "app",
        r#"contract-exclusions = [{ path = "fixtures", reason = "compile-fail fixtures" }]"#,
    );
    let names = workspace_package_names(&cargo_metadata(&repo.root).unwrap()).unwrap();
    assert_eq!(names.into_iter().collect::<Vec<_>>(), ["app"]);

    repo.root_package(
        "app",
        r#"contract-exclusions = [
    { path = "fixtures", reason = "compile-fail fixtures" },
    { path = "gone", reason = "stale" },
]"#,
    );
    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("contract exclusion gone matches no Cargo manifest"),
        "{err}"
    );
}

#[test]
fn an_exclusion_may_not_hide_a_contract_manifest() {
    let repo = Repo::new("policy-overlap");
    repo.root_package(
        "app",
        r#"contract-workspaces = ["nested"]
contract-exclusions = [{ path = "nested", reason = "overlaps" }]"#,
    );
    repo.nested_workspace("nested", &["tool"]);
    repo.package("nested/tool", "tool");

    let err = cargo_metadata(&repo.root).unwrap_err();
    assert!(
        err.contains("contract exclusion nested covers contract manifest nested/Cargo.toml"),
        "{err}"
    );
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
    write_package(&outside.join("tool"), "tool");
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
    write_package(&outside, "tool");
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
fn a_repository_inside_another_workspace_is_refused() {
    let outer = temp_root("policy-outer");
    fs::write(
        outer.join("Cargo.toml"),
        "[workspace]\nmembers = [\"inner\"]\n",
    )
    .unwrap();
    write_package(&outer.join("inner"), "inner");
    git(&outer.join("inner"), &["init", "--quiet"]);

    let err = cargo_metadata(&outer.join("inner")).unwrap_err();
    assert!(
        err.contains("is not the root of its Cargo workspace"),
        "{err}"
    );

    fs::remove_dir_all(outer).unwrap();
}

#[test]
fn classification_requires_git_visibility() {
    let root = temp_root("policy-no-git");
    write_package(&root, "app");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();

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
        self.write("src/lib.rs", "");
        self.write(
            "Cargo.toml",
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [workspace]\nexclude = [\"nested\", \"fixtures\", \"linked\"]\n\n\
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

    fn package(&self, relative: &str, name: &str) {
        write_package(&self.root.join(relative), name);
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_package(dir: &Path, name: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), "").unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
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
