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

/// `checkouts/sim-app` (the documented repository), `checkouts/sim-app` (a
/// sibling checkout holding the package sources), and
/// `checkouts/sim-private/.meta-workspace/packages/lib`, a real package
/// directory whose `src`, `recipes`, and `README.md` are links into sim-app.
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
            "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [workspace]\nmembers = [\"xtask\"]\n",
        );
        farm.write(
            "sim-app/xtask/Cargo.toml",
            "[package]\nname = \"xtask\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        farm.write("sim-app/xtask/src/main.rs", "fn main() {}\n");
        farm.write(
            "sim-other/crates/xtask/src/lib.rs",
            "pub fn stranger() {}\n",
        );
        farm.write("sim-app/crates/lib/src/lib.rs", "pub fn one() {}\n");
        farm.write("sim-app/crates/lib/src/nested/mod.rs", "pub fn two() {}\n");
        farm.write("sim-app/crates/lib/recipes/r.toml", "title = \"r\"\n");
        farm.write("sim-app/crates/lib/README.md", "# lib\n");
        for repo in ["sim-app", "sim-other"] {
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
            "[workspace]\nresolver = \"3\"\nmembers = [\"packages/lib\", \"packages/xtask\"]\n",
        );
        farm.write(
            "sim-private/.meta-workspace/packages/xtask/Cargo.toml",
            "[package]\nname = \"xtask\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        symlink(
            farm.base.join("sim-other/crates/xtask/src"),
            farm.base
                .join("sim-private/.meta-workspace/packages/xtask/src"),
        )
        .unwrap();
        farm.write(
            &format!("{package}/Cargo.toml"),
            "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        for name in ["src", "recipes", "README.md"] {
            symlink(
                farm.base.join("sim-app/crates/lib").join(name),
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
    for leak in ["/", "checkouts", "sim-app", "sim-private", "src", "recipes"] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
    input.remeasure(&farm.repo()).unwrap();

    // A change to a tracked target file behind a link moves the identity...
    farm.write("sim-app/crates/lib/src/nested/mod.rs", "pub fn TWO() {}\n");
    let changed = farm.validate().unwrap();
    assert_ne!(changed.closure_sha256, input.closure_sha256);
    assert_ne!(changed.cache_key(), input.cache_key());
    assert!(input.remeasure(&farm.repo()).is_err());
    // ...and so does a change behind a file link.
    farm.write("sim-app/crates/lib/README.md", "# LIB\n");
    assert_ne!(
        farm.validate().unwrap().closure_sha256,
        changed.closure_sha256
    );
}

#[test]
fn a_namesake_package_of_another_repository_is_not_selected_or_bound() {
    let farm = Farm::new("farm-namesake");
    let input = farm.validate().unwrap();
    // The farm's `xtask` is sim-other's: the repository's own `xtask` is not
    // in the farm, so only `lib` is documented and bound.
    assert_eq!(input.selected, ["lib"]);
    assert_eq!(input.closure_packages, 1);
    farm.write("sim-other/crates/xtask/src/lib.rs", "pub fn changed() {}\n");
    farm.write("sim-other/crates/xtask/src/untracked.rs", "stray\n");
    assert_eq!(farm.validate().unwrap(), input);
}

/// Rewrites the farm package manifest with extra target tables and reads the
/// resolver through the real `cargo metadata`.
fn validate_with_manifest(farm: &Farm, extra: &str) -> Result<ResolverInput, String> {
    let package = farm.base.join("sim-private/.meta-workspace/packages/lib");
    fs::write(
        package.join("Cargo.toml"),
        format!("[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n{extra}"),
    )
    .unwrap();
    farm.validate()
}

#[test]
fn every_cargo_target_path_must_be_in_the_bound_closure() {
    let farm = Farm::new("farm-target-paths");
    farm.write("sim-app/crates/lib/outside.rs", "fn main() {}\n");
    farm.git("sim-app", &["add", "-A"]);
    let evil = farm.base.parent().unwrap().join("evil.rs");
    fs::write(&evil, "fn main() {}\n").unwrap();
    let inside = "../../../../sim-app/crates/lib/src/lib.rs";
    // A linked file, however it is spelled, is bound.
    validate_with_manifest(&farm, &format!("[lib]\npath = \"{inside}\"\n")).unwrap();
    validate_with_manifest(&farm, "[lib]\npath = \"src/lib.rs\"\n").unwrap();

    for (label, extra) in [
        (
            "absolute-lib",
            format!("[lib]\npath = \"{}\"\n", evil.display()),
        ),
        (
            "absolute-bin",
            format!("[[bin]]\nname = \"x\"\npath = \"{}\"\n", evil.display()),
        ),
        (
            "absolute-build",
            format!(
                "build = \"{}\"\n[lib]\npath = \"src/lib.rs\"\n",
                evil.display()
            ),
        ),
        (
            "tracked-but-unlinked",
            "[lib]\npath = \"../../../../sim-app/crates/lib/outside.rs\"\n".to_owned(),
        ),
        (
            "test-target",
            format!(
                "[lib]\npath = \"src/lib.rs\"\n[[test]]\nname = \"t\"\npath = \"{}\"\n",
                evil.display()
            ),
        ),
        (
            "example-target",
            format!(
                "[lib]\npath = \"src/lib.rs\"\n[[example]]\nname = \"e\"\npath = \"{}\"\n",
                evil.display()
            ),
        ),
    ] {
        let err = validate_with_manifest(&farm, &extra).expect_err(label);
        assert!(
            err.contains("not part of the package's bound source closure"),
            "{label}: {err}"
        );
    }
}

#[test]
fn a_package_of_the_repository_is_its_manifest_and_links_only() {
    for name in ["build.rs", "extra.rs"] {
        let farm = Farm::new("farm-real-file");
        farm.write(
            &format!("sim-private/.meta-workspace/packages/lib/{name}"),
            "fn main() {}\n",
        );
        let err = farm.validate().unwrap_err();
        assert!(
            err.contains("has its own") && err.contains(name),
            "{name}: {err}"
        );
    }
    let farm = Farm::new("farm-real-dir");
    farm.write(
        "sim-private/.meta-workspace/packages/lib/.cargo/config.toml",
        "",
    );
    assert!(farm.validate().unwrap_err().contains("has its own .cargo"));
}

#[test]
fn a_package_with_no_links_into_the_repository_is_refused() {
    let farm = Farm::new("farm-zero-links");
    let package = farm.base.join("sim-private/.meta-workspace/packages/lib");
    for name in ["src", "recipes", "README.md"] {
        fs::remove_file(package.join(name)).unwrap();
    }
    // The manifest reaches the repository's source by a plain relative path.
    fs::write(
        package.join("Cargo.toml"),
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [lib]\npath = \"../../../../sim-app/crates/lib/src/lib.rs\"\n",
    )
    .unwrap();
    let err = farm.validate().unwrap_err();
    assert!(
        err.contains("no links into the documented repository"),
        "{err}"
    );
}

#[test]
fn a_resolver_that_selects_none_of_the_repository_is_refused() {
    let farm = Farm::new("farm-none");
    fs::write(
        farm.base.join("sim-app/Cargo.toml"),
        "[package]\nname = \"unrelated\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    assert!(farm.validate().unwrap_err().contains("no workspace member"));
}

#[test]
fn every_ignored_or_untracked_entry_in_a_followed_tree_is_refused() {
    // An ignored source the build can read (include_bytes, a build script) is
    // as much an input as a tracked one, so it refuses the run.
    for (label, ignore, path, contents) in [
        (
            "generated",
            "/crates/lib/src/generated.rs\n",
            "src/generated.rs",
            "first\n",
        ),
        (
            "blob",
            "/crates/lib/src/blob.bin\n",
            "src/blob.bin",
            "bytes\n",
        ),
        (
            "target-dir",
            "/crates/lib/src/target/\n",
            "src/target/debug/junk",
            "junk\n",
        ),
        ("stray", "", "src/stray.rs", "pub fn stray() {}\n"),
    ] {
        let farm = Farm::new(&format!("farm-entry-{label}"));
        farm.write("sim-app/.gitignore", ignore);
        farm.write(&format!("sim-app/crates/lib/{path}"), contents);
        let err = farm.validate().unwrap_err();
        assert!(
            err.contains(path.rsplit('/').next().unwrap()) && err.contains("refused"),
            "{label}: {err}"
        );
    }
    // Ignored links are refused too, not followed and not skipped.
    let farm = Farm::new("farm-ignored-link");
    farm.write("sim-app/.gitignore", "/crates/lib/src/alias.rs\n");
    symlink(
        farm.base.join("sim-app/Cargo.toml"),
        farm.base.join("sim-app/crates/lib/src/alias.rs"),
    )
    .unwrap();
    assert!(farm.validate().unwrap_err().contains("alias.rs"));
}

#[test]
fn only_a_nested_crates_untracked_lockfile_is_exempt() {
    let farm = Farm::new("farm-nested-lock");
    // sim-app/crates/lib/src/focused/{Cargo.toml (tracked), Cargo.lock (ignored)}.
    farm.write(
        "sim-app/crates/lib/src/focused/Cargo.toml",
        "[package]\nname = \"focused\"\n",
    );
    farm.git("sim-app", &["add", "-A"]);
    farm.write("sim-app/.gitignore", "Cargo.lock\n");
    farm.write("sim-app/crates/lib/src/focused/Cargo.lock", "# nested\n");
    let before = farm.validate().unwrap();
    // The exempt lock need not be tracked, but its bytes are bound.
    farm.write(
        "sim-app/crates/lib/src/focused/Cargo.lock",
        "# changed, and longer\n",
    );
    assert_ne!(
        farm.validate().unwrap().closure_sha256,
        before.closure_sha256
    );
    // A lockfile with no tracked manifest beside it is an ordinary stray.
    farm.write("sim-app/crates/lib/src/Cargo.lock", "# stray\n");
    let err = farm.validate().unwrap_err();
    assert!(
        err.contains("Cargo.lock") && err.contains("refused"),
        "{err}"
    );
}

#[test]
fn a_link_to_an_untracked_file_is_refused() {
    let farm = Farm::new("farm-untracked");
    farm.write("sim-app/crates/lib/src/extra.rs", "pub fn stray() {}\n");
    let err = farm.validate().unwrap_err();
    assert!(
        err.contains("extra.rs") && err.contains("untracked"),
        "{err}"
    );

    let farm = Farm::new("farm-untracked-file-link");
    fs::remove_file(farm.base.join("sim-app/crates/lib/README.md")).unwrap();
    farm.write("sim-app/crates/lib/README.md", "# lib, rewritten\n");
    farm.git(
        "sim-app",
        &["rm", "--cached", "--quiet", "crates/lib/README.md"],
    );
    let err = farm.validate().unwrap_err();
    assert!(
        err.contains("README.md") && err.contains("untracked"),
        "{err}"
    );
}

#[test]
fn a_package_linking_both_into_and_out_of_the_repository_is_refused() {
    let farm = Farm::new("farm-mixed");
    let outside = farm.base.parent().unwrap().join("elsewhere");
    fs::create_dir_all(&outside).unwrap();
    let package = farm.base.join("sim-private/.meta-workspace/packages/lib");
    symlink(&outside, package.join("extra")).unwrap();
    let err = farm.validate().unwrap_err();
    assert!(err.contains("both into and out of"), "{err}");
}

#[test]
fn a_dependency_may_link_only_into_git_checkouts_beside_the_repository() {
    for (label, target, why) in [
        (
            "outside",
            "../elsewhere",
            "outside the constellation's checkouts",
        ),
        ("plain", "plain", "is not inside a Git checkout"),
    ] {
        let farm = Farm::new(&format!("farm-dependency-{label}"));
        let target = farm.base.join(target);
        fs::create_dir_all(target.join("src")).unwrap();
        fs::write(target.join("src/lib.rs"), "").unwrap();
        let workspace = farm.base.join("sim-private/.meta-workspace");
        fs::write(
            workspace.join("packages/lib/Cargo.toml"),
            "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\ndep = { path = \"../dep\" }\n",
        )
        .unwrap();
        farm.write(
            "sim-private/.meta-workspace/packages/dep/Cargo.toml",
            "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        symlink(
            target.canonicalize().unwrap().join("src"),
            workspace.join("packages/dep/src"),
        )
        .unwrap();
        fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nresolver = \"3\"\nmembers = [\"packages/lib\", \"packages/xtask\"]\n",
        )
        .unwrap();
        let status = std::process::Command::new(env!("CARGO"))
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(farm.manifest())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let err = farm.validate().unwrap_err();
        assert!(err.contains(why), "{label}: {err}");
    }
}

#[test]
fn a_link_inside_a_followed_tree_is_refused() {
    let farm = Farm::new("farm-nested-link");
    symlink(
        farm.base.join("sim-app/Cargo.toml"),
        farm.base.join("sim-app/crates/lib/src/nested/alias.rs"),
    )
    .unwrap();
    farm.git("sim-app", &["add", "-A"]);
    let err = farm.validate().unwrap_err();
    assert!(err.contains("alias.rs is not an ordinary file"), "{err}");
}
