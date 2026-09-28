// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

// conformance: the shared resolver is validated once, binds its selected
// path-package source closure, is recorded by content only, and refuses any
// change while in use.

struct Layout {
    base: PathBuf,
}

impl Layout {
    /// `sim-private/.meta-workspace` with member `packages/app`, which
    /// path-depends on the non-member `libs/helper`; a repository `sim-app`
    /// whose root package is `app`; and a sibling repository `sim-helper`
    /// supplying `helper`'s tracked source. Both farm packages are exactly a
    /// generated manifest plus links into their owning checkout: a bare real
    /// directory or file beside a farm manifest is refused, whether the
    /// package is the documented repository's own selected member or a
    /// reached dependency (see `resolver_input::package_files`).
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = env::temp_dir().join(format!("{label}-{}-{stamp}", std::process::id()));
        let layout = Self { base };
        layout.write(
            "sim-private/.meta-workspace/Cargo.toml",
            "[workspace]\nresolver = \"3\"\nmembers = [\"packages/app\"]\n",
        );
        layout.write(
            "sim-private/.meta-workspace/packages/app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nhelper = { path = \"../../libs/helper\" }\n",
        );
        // Both farm packages' real sources live in their own repositories;
        // the farm packages only link to them.
        layout.write("sim-app/src/lib.rs", "");
        layout.write(
            "sim-private/.meta-workspace/libs/helper/Cargo.toml",
            "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        layout.write("sim-helper/src/lib.rs", "pub fn one() {}\n");
        layout.write(
            "sim-app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        for repo in ["sim-app", "sim-helper"] {
            for args in [
                vec!["init", "--quiet"],
                vec!["add", "-A"],
                vec![
                    "-c",
                    "user.name=fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "--no-verify",
                    "-mfixture",
                ],
            ] {
                assert!(
                    Command::new("git")
                        .args(&args)
                        .current_dir(layout.base.join(repo))
                        .status()
                        .unwrap()
                        .success()
                );
            }
        }
        std::os::unix::fs::symlink(
            layout.base.join("sim-app/src"),
            layout
                .base
                .join("sim-private/.meta-workspace/packages/app/src"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            layout.base.join("sim-helper/src"),
            layout
                .base
                .join("sim-private/.meta-workspace/libs/helper/src"),
        )
        .unwrap();
        let status = Command::new(env!("CARGO"))
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(layout.manifest())
            .status()
            .unwrap();
        assert!(status.success());
        layout
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.base.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn manifest(&self) -> PathBuf {
        self.base.join("sim-private/.meta-workspace/Cargo.toml")
    }

    fn repo(&self) -> PathBuf {
        self.base.join("sim-app")
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn the_selected_source_closure_is_bound_and_recorded_by_content_only() {
    let layout = Layout::new("resolver-closure");
    let input = validate(&layout.manifest(), &layout.repo()).unwrap();

    assert_eq!(input.selected, ["app"]);
    assert_eq!(input.closure_packages, 2);
    let projection = input.projection();
    assert_eq!(projection["kind"], "shared-resolver");
    let text = projection.to_string();
    for leak in [
        "/",
        "sim-private",
        "meta-workspace",
        "Cargo.toml",
        "..",
        "helper",
    ] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
    input.remeasure(&layout.repo()).unwrap();

    layout.write(
        "sim-private/.meta-workspace/libs/helper/src/lib.rs",
        "pub fn substituted() {}\n",
    );
    let changed = validate(&layout.manifest(), &layout.repo()).unwrap();
    assert_ne!(changed.closure_sha256, input.closure_sha256);
    assert_ne!(changed.cache_key(), input.cache_key());
    let err = input.remeasure(&layout.repo()).unwrap_err();
    assert!(err.contains("changed while simdoc was using it"), "{err}");
}

#[test]
fn a_manifest_only_change_moves_the_identity_and_the_cache_key() {
    let layout = Layout::new("resolver-manifest");
    let input = validate(&layout.manifest(), &layout.repo()).unwrap();
    layout.write(
        "sim-private/.meta-workspace/Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"packages/app\"]\n# reviewed\n",
    );
    let changed = validate(&layout.manifest(), &layout.repo()).unwrap();

    assert_eq!(changed.lock_sha256, input.lock_sha256);
    assert_ne!(changed.manifest_sha256, input.manifest_sha256);
    assert_ne!(changed.cache_key(), input.cache_key());
}

#[test]
fn a_symlink_whose_target_is_untracked_is_refused() {
    let layout = Layout::new("resolver-symlink");
    layout.write("sim-app/untracked.txt", "stray\n");
    // A link directly beside the farm manifest (package_files's top level),
    // not nested inside an already-followed link's subtree, so this
    // exercises the ownership check on the link's own canonical target
    // rather than the separate "no nested link" refusal below.
    std::os::unix::fs::symlink(
        layout.repo().join("untracked.txt"),
        layout
            .base
            .join("sim-private/.meta-workspace/libs/helper/extra.rs"),
    )
    .unwrap();

    // The target is a file the sibling checkout does not track.
    let err = validate(&layout.manifest(), &layout.repo()).unwrap_err();
    assert!(
        err.contains("untracked.txt") && err.contains("refused"),
        "{err}"
    );
}

#[test]
fn a_link_nested_inside_an_already_followed_link_is_refused() {
    let layout = Layout::new("resolver-nested-link");
    layout.write("sim-app/untracked.txt", "stray\n");
    // Nested inside `libs/helper/src`, which is itself a link into
    // `sim-helper/src` (see `Layout::new`): every entry beneath a followed
    // link target must be an ordinary tracked file, never another link.
    std::os::unix::fs::symlink(
        layout.repo().join("untracked.txt"),
        layout
            .base
            .join("sim-private/.meta-workspace/libs/helper/src/extra.rs"),
    )
    .unwrap();

    let err = validate(&layout.manifest(), &layout.repo()).unwrap_err();
    assert!(
        err.contains("extra.rs") && err.contains("not an ordinary file"),
        "{err}"
    );
}

#[test]
fn a_bare_real_directory_beside_a_dependency_farm_manifest_is_refused() {
    // A reached dependency package (not a selected repository member) whose
    // own directory holds a real, non-symlinked subtree instead of a link
    // into a validated sibling checkout: the farm, not a tracked worktree,
    // would supply that content, and package_files must refuse it exactly
    // as belongs_to_repo already refuses the same shape for a selected
    // member -- not silently walk and accept it.
    let layout = Layout::new("resolver-bare-dependency");
    fs::remove_file(
        layout
            .base
            .join("sim-private/.meta-workspace/libs/helper/src"),
    )
    .unwrap();
    layout.write(
        "sim-private/.meta-workspace/libs/helper/src/lib.rs",
        "pub fn one() {}\n",
    );

    let err = validate(&layout.manifest(), &layout.repo()).unwrap_err();
    assert!(
        err.contains("beside its links") && err.contains("refused"),
        "{err}"
    );
}

#[test]
fn anything_but_a_locked_workspace_manifest_is_refused() {
    let layout = Layout::new("resolver-refused");
    let root = &layout.base;
    layout.write("package/Cargo.toml", "[package]\nname = \"p\"\n");
    layout.write("package/Cargo.lock", "");
    layout.write("unlocked/Cargo.toml", "[workspace]\n");
    fs::create_dir_all(root.join("linked")).unwrap();
    std::os::unix::fs::symlink(
        root.join("unlocked/Cargo.toml"),
        root.join("linked/Cargo.toml"),
    )
    .unwrap();

    for (path, why) in [
        (PathBuf::new(), "empty path"),
        (root.join("other.toml"), "not a Cargo.toml"),
        (root.join("package/Cargo.toml"), "not a workspace manifest"),
        (root.join("unlocked/Cargo.toml"), "no Cargo.lock"),
        (root.join("linked/Cargo.toml"), "not an ordinary file"),
    ] {
        let err = validate(&path, &layout.repo()).unwrap_err();
        assert!(err.contains(why), "{}: {err}", path.display());
    }
}
