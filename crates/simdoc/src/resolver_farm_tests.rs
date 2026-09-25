// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    env, fs,
    os::unix::fs::symlink,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::resolver_input::{ResolverInput, validate};

// conformance: a meta-workspace symlink farm is read through canonical
// targets that are tracked ordinary files of sibling checkouts, and nothing
// else, without the identity recording any path.

/// `checkouts/sim-app` (the documented repository), `checkouts/sim-lib` (a
/// sibling checkout holding the package sources), and
/// `checkouts/sim-private/.meta-workspace/packages/lib`, a real package
/// directory whose `src`, `recipes`, and `README.md` are links into sim-lib.
struct Farm {
    base: PathBuf,
}

impl Farm {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = env::temp_dir()
            .join(format!("{label}-{}-{stamp}", std::process::id()))
            .join("checkouts");
        let farm = Self { base };
        farm.write(
            "sim-app/Cargo.toml",
            "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        farm.write("sim-lib/crates/lib/src/lib.rs", "pub fn one() {}\n");
        farm.write("sim-lib/crates/lib/src/nested/mod.rs", "pub fn two() {}\n");
        farm.write("sim-lib/crates/lib/recipes/r.toml", "title = \"r\"\n");
        farm.write("sim-lib/crates/lib/README.md", "# lib\n");
        for repo in ["sim-app", "sim-lib"] {
            farm.git(repo, &["init", "--quiet"]);
            farm.git(repo, &["add", "-A"]);
            farm.git(
                repo,
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
                    "-mfixture",
                ],
            );
        }
        let package = "sim-private/.meta-workspace/packages/lib";
        farm.write(
            "sim-private/.meta-workspace/Cargo.toml",
            "[workspace]\nresolver = \"3\"\nmembers = [\"packages/lib\"]\n",
        );
        farm.write(
            &format!("{package}/Cargo.toml"),
            "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        for name in ["src", "recipes", "README.md"] {
            symlink(
                farm.base.join("sim-lib/crates/lib").join(name),
                farm.base.join(package).join(name),
            )
            .unwrap();
        }
        let status = Command::new(env!("CARGO"))
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(farm.manifest())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        farm
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.base.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn git(&self, repo: &str, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(self.base.join(repo))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn manifest(&self) -> PathBuf {
        self.base.join("sim-private/.meta-workspace/Cargo.toml")
    }

    fn repo(&self) -> PathBuf {
        self.base.join("sim-app").canonicalize().unwrap()
    }

    fn validate(&self) -> Result<ResolverInput, String> {
        validate(&self.manifest(), &self.repo())
    }
}

impl Drop for Farm {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.base.parent().unwrap());
    }
}

#[test]
fn a_symlink_farm_is_bound_by_the_content_of_its_canonical_targets() {
    let farm = Farm::new("farm-accepted");
    let input = farm.validate().unwrap();

    assert_eq!(input.selected, ["lib"]);
    assert_eq!(input.closure_packages, 1);
    let text = input.projection().to_string();
    for leak in ["/", "checkouts", "sim-lib", "sim-private", "src", "recipes"] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
    input.remeasure(&farm.repo()).unwrap();

    // A change to a tracked target file behind a link moves the identity...
    farm.write("sim-lib/crates/lib/src/nested/mod.rs", "pub fn TWO() {}\n");
    let changed = farm.validate().unwrap();
    assert_ne!(changed.closure_sha256, input.closure_sha256);
    assert_ne!(changed.cache_key(), input.cache_key());
    assert!(input.remeasure(&farm.repo()).is_err());
    // ...and so does a change behind a file link.
    farm.write("sim-lib/crates/lib/README.md", "# LIB\n");
    assert_ne!(
        farm.validate().unwrap().closure_sha256,
        changed.closure_sha256
    );
}

#[test]
fn ignored_build_output_inside_a_linked_tree_is_not_read() {
    let farm = Farm::new("farm-target");
    let before = farm.validate().unwrap();
    farm.write("sim-lib/crates/lib/src/target/debug/junk", "junk");
    assert_eq!(farm.validate().unwrap(), before);
}

#[test]
fn an_ignored_file_in_a_linked_tree_is_neither_read_nor_bound() {
    let farm = Farm::new("farm-ignored");
    farm.write("sim-lib/.gitignore", "/crates/lib/src/generated.rs\n");
    farm.write("sim-lib/crates/lib/src/generated.rs", "first\n");
    let before = farm.validate().unwrap();
    farm.write(
        "sim-lib/crates/lib/src/generated.rs",
        "second, and longer\n",
    );
    assert_eq!(farm.validate().unwrap(), before);
    // A file that is untracked and not ignored is still refused.
    farm.write("sim-lib/crates/lib/src/stray.rs", "pub fn stray() {}\n");
    let err = farm.validate().unwrap_err();
    assert!(
        err.contains("stray.rs") && err.contains("untracked"),
        "{err}"
    );
}

#[test]
fn a_link_to_an_untracked_file_is_refused() {
    let farm = Farm::new("farm-untracked");
    farm.write("sim-lib/crates/lib/src/extra.rs", "pub fn stray() {}\n");
    let err = farm.validate().unwrap_err();
    assert!(
        err.contains("extra.rs") && err.contains("untracked"),
        "{err}"
    );

    let farm = Farm::new("farm-untracked-file-link");
    fs::remove_file(farm.base.join("sim-lib/crates/lib/README.md")).unwrap();
    farm.write("sim-lib/crates/lib/README.md", "# lib, rewritten\n");
    farm.git(
        "sim-lib",
        &["rm", "--cached", "--quiet", "crates/lib/README.md"],
    );
    let err = farm.validate().unwrap_err();
    assert!(
        err.contains("README.md") && err.contains("untracked"),
        "{err}"
    );
}

#[test]
fn a_link_outside_the_checkouts_or_into_a_plain_directory_is_refused() {
    let farm = Farm::new("farm-outside");
    let outside = farm.base.parent().unwrap().join("elsewhere");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("stray.rs"), "").unwrap();
    let package = farm.base.join("sim-private/.meta-workspace/packages/lib");
    symlink(&outside, package.join("extra")).unwrap();
    let err = farm.validate().unwrap_err();
    assert!(
        err.contains("outside the constellation's checkouts"),
        "{err}"
    );
    fs::remove_file(package.join("extra")).unwrap();

    let plain = farm.base.join("plain");
    fs::create_dir_all(&plain).unwrap();
    symlink(&plain, package.join("extra")).unwrap();
    let err = farm.validate().unwrap_err();
    assert!(err.contains("is not inside a Git checkout"), "{err}");
}

#[test]
fn a_link_inside_a_followed_tree_is_refused() {
    let farm = Farm::new("farm-nested-link");
    symlink(
        farm.base.join("sim-app/Cargo.toml"),
        farm.base.join("sim-lib/crates/lib/src/nested/alias.rs"),
    )
    .unwrap();
    farm.git("sim-lib", &["add", "-A"]);
    let err = farm.validate().unwrap_err();
    assert!(err.contains("alias.rs is not an ordinary file"), "{err}");
}
