// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A committed Git repository fixture for tests that run the whole contract
//! engine without the sim-tooling checkout itself.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

/// A temporary repository with a public origin, removed on drop.
pub(crate) struct FixtureRepo {
    pub(crate) root: PathBuf,
}

impl FixtureRepo {
    /// A repository named `sim-fixture` holding a root package `app` whose
    /// workspace declares `nested` as a contract workspace with member `tool`
    /// (which has one feature).
    pub(crate) fn nested(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let parent = env::temp_dir().join(format!("{label}-{}-{stamp}", std::process::id()));
        let root = parent.join("sim-fixture");
        fs::create_dir_all(&root).unwrap();
        let repo = Self { root };
        repo.write(
            "Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             description = \"Fixture application.\"\n\n\
             [workspace]\nexclude = [\"nested\"]\n\n\
             [workspace.metadata.sim]\ncontract-workspaces = [\"nested\"]\n",
        );
        repo.write(
            "src/lib.rs",
            "//! Fixture application.\n\npub fn app() {}\n",
        );
        repo.write("nested/Cargo.toml", "[workspace]\nmembers = [\"tool\"]\n");
        repo.write(
            "nested/tool/Cargo.toml",
            "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
             description = \"Fixture tool.\"\npublish = false\n\n\
             [features]\nfast = []\n",
        );
        repo.write(
            "nested/tool/src/lib.rs",
            "//! Fixture tool.\n\npub fn tool() {}\n",
        );
        repo.git(&["init", "--quiet"]);
        repo.git(&[
            "remote",
            "add",
            "origin",
            "https://github.com/sim-nest/sim-fixture",
        ]);
        repo.commit();
        repo
    }

    /// Writes one file, creating its parents.
    pub(crate) fn write(&self, relative: &str, text: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// Commits everything with a fixed committer date.
    pub(crate) fn commit(&self) {
        self.git(&["add", "-A"]);
        self.git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--no-verify",
            "--allow-empty",
            "-m",
            "fixture",
        ]);
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_AUTHOR_DATE", "2026-09-01T12:00:00+00:00")
            .env("GIT_COMMITTER_DATE", "2026-09-01T12:00:00+00:00")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    /// Reads one repository file.
    pub(crate) fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative)).unwrap()
    }

    /// The repository root.
    pub(crate) fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for FixtureRepo {
    fn drop(&mut self) {
        if let Some(parent) = self.root.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }
}
