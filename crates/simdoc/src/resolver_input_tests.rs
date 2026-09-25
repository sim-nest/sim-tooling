// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

// conformance: the shared resolver is validated once, binds its selected
// path-package source closure, is recorded by content only, and refuses any
// change while in use.

struct Layout {
    base: PathBuf,
}

impl Layout {
    /// `sim-private/.meta-workspace` with member `packages/app`, which
    /// path-depends on the non-member `libs/helper`; and a repository
    /// `sim-app` whose root package is `app`.
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
        layout.write("sim-private/.meta-workspace/packages/app/src/lib.rs", "");
        layout.write(
            "sim-private/.meta-workspace/libs/helper/Cargo.toml",
            "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        layout.write(
            "sim-private/.meta-workspace/libs/helper/src/lib.rs",
            "pub fn one() {}\n",
        );
        layout.write(
            "sim-app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
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
fn a_symlink_inside_a_path_package_is_refused() {
    let layout = Layout::new("resolver-symlink");
    std::os::unix::fs::symlink(
        layout.repo().join("Cargo.toml"),
        layout
            .base
            .join("sim-private/.meta-workspace/libs/helper/src/extra.rs"),
    )
    .unwrap();

    let err = validate(&layout.manifest(), &layout.repo()).unwrap_err();
    assert!(err.contains("is a symlink"), "{err}");
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
