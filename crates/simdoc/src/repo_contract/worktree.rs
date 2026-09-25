// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Git worktree that owns a repository contract.
//!
//! A contract may only describe files its own worktree owns: ordinary files
//! that `git ls-files --cached --others --exclude-standard` reports for the
//! worktree rooted exactly at the repository. Ignored files, files inside a
//! submodule (gitlink) or a nested repository, symlinks, and anything outside
//! the repository are refused, because the source commit and workspace hash
//! recorded in provenance would not bind them.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::bounded_process::run_bounded;

/// Ceiling for `git ls-files` output.
const MAX_LISTING_BYTES: usize = 64 * 1024 * 1024;
/// Ceiling for Git diagnostics.
const MAX_DIAGNOSTIC_BYTES: usize = 1024 * 1024;
/// Ceiling for the number of files a worktree may list.
const MAX_VISIBLE_FILES: usize = 1_000_000;

/// The files owned by the Git worktree rooted at a repository.
#[derive(Debug)]
pub(crate) struct Worktree {
    root: PathBuf,
    visible: BTreeSet<String>,
}

impl Worktree {
    /// Opens the worktree rooted exactly at `repo` (canonical), refusing a
    /// repository that is only a subdirectory of some other worktree.
    pub(crate) fn open(repo: &Path) -> Result<Self, String> {
        let root = repo
            .canonicalize()
            .map_err(|err| format!("{}: {err}", repo.display()))?;
        let toplevel = git(&root, &["rev-parse", "--show-toplevel"], "git rev-parse")?;
        let toplevel = String::from_utf8(toplevel)
            .map_err(|_| "git rev-parse returned a non-UTF-8 path".to_owned())?;
        let toplevel = Path::new(toplevel.trim())
            .canonicalize()
            .map_err(|err| format!("git worktree root: {err}"))?;
        if toplevel != root {
            return Err(format!(
                "{} is not the root of its Git worktree; cannot classify Cargo manifests",
                root.display()
            ));
        }
        let listing = git(
            &root,
            &[
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ],
            "git ls-files",
        )?;
        let listing = String::from_utf8(listing)
            .map_err(|_| "git ls-files returned a non-UTF-8 path".to_owned())?;
        let visible = listing
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        if visible.len() > MAX_VISIBLE_FILES {
            return Err(format!(
                "git worktree lists more than {MAX_VISIBLE_FILES} files; refusing an unbounded contract"
            ));
        }
        Ok(Self { root, visible })
    }

    /// Canonical repository root.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Repository-relative paths of every visible `Cargo.toml`.
    pub(crate) fn manifests(&self) -> BTreeSet<String> {
        self.visible
            .iter()
            .filter(|path| {
                Path::new(path.as_str())
                    .file_name()
                    .is_some_and(|name| name == "Cargo.toml")
            })
            .filter(|path| self.root.join(path.as_str()).is_file())
            .cloned()
            .collect()
    }

    /// Requires `path` to be an ordinary file owned by this worktree and
    /// returns its repository-relative path.
    pub(crate) fn owned_file(&self, path: &Path, role: &str) -> Result<String, String> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|err| format!("{role} {}: {err}", path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!("{role} {} is not an ordinary file", path.display()));
        }
        let canonical = path
            .canonicalize()
            .map_err(|err| format!("{role} {}: {err}", path.display()))?;
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| format!("{role} {} resolves outside the repository", path.display()))?
            .to_str()
            .ok_or_else(|| format!("{role} {} is not UTF-8", path.display()))?
            .replace(std::path::MAIN_SEPARATOR, "/");
        if !self.visible.contains(&relative) {
            return Err(format!(
                "{role} {relative} is not a file of this Git worktree \
                 (ignored, inside a submodule or nested repository, or outside Git)"
            ));
        }
        Ok(relative)
    }
}

fn git(repo: &Path, args: &[&str], role: &str) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    let captured = run_bounded(command, role, MAX_LISTING_BYTES, MAX_DIAGNOSTIC_BYTES)?;
    if !captured.status.success() {
        return Err(format!(
            "{role} failed in {}; cannot classify Cargo manifests: {}",
            repo.display(),
            String::from_utf8_lossy(&captured.stderr).trim()
        ));
    }
    Ok(captured.stdout)
}
