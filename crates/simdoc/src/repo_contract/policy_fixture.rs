// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Repositories on disk for the contract-policy tests.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub(in crate::repo_contract) use crate::repo_contract::cargo_metadata as raw_cargo_metadata;
use crate::repo_contract::contract_packages as raw_contract_packages;

/// Tracks everything Git would add (never an ignored file), so a fixture's
/// files are owned the way committed files are. Fixtures that need a file to
/// stay unowned say so and call the raw functions.
pub(in crate::repo_contract) fn stage(root: &Path) {
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub(in crate::repo_contract) fn cargo_metadata(root: &Path) -> Result<serde_json::Value, String> {
    stage(root);
    raw_cargo_metadata(root)
}

pub(in crate::repo_contract) fn contract_packages(
    root: &Path,
) -> Result<crate::repo_contract::ContractPackages, String> {
    stage(root);
    raw_contract_packages(root)
}

pub(in crate::repo_contract) struct Repo {
    pub(in crate::repo_contract) root: PathBuf,
}

impl Repo {
    pub(in crate::repo_contract) fn new(name: &str) -> Self {
        let root = temp_root(name);
        git(&root, &["init", "--quiet"]);
        Self { root }
    }

    pub(in crate::repo_contract) fn write(&self, relative: &str, text: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, crate::test_fixture::with_helper(text)).unwrap();
    }

    pub(in crate::repo_contract) fn root_package(&self, name: &str, sim_metadata: &str) {
        self.root_package_with(name, sim_metadata, "");
    }

    pub(in crate::repo_contract) fn root_package_with(
        &self,
        name: &str,
        sim_metadata: &str,
        extra: &str,
    ) {
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

    pub(in crate::repo_contract) fn nested_workspace(&self, relative: &str, members: &[&str]) {
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

    pub(in crate::repo_contract) fn package(&self, relative: &str, name: &str, extra: &str) {
        write_package(&self.root.join(relative), name, extra);
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(in crate::repo_contract) fn write_package(dir: &Path, name: &str, extra: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), "").unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{extra}"),
    )
    .unwrap();
}

pub(in crate::repo_contract) fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

pub(in crate::repo_contract) fn temp_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
