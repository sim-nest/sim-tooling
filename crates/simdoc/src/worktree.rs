// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Git worktree that owns a repository contract.
//!
//! A contract may only describe files its own worktree owns: ordinary files
//! that `git ls-files --cached` reports, that is, files Git tracks, for the
//! worktree rooted exactly at the repository. Untracked files (whether or not
//! they are ignored), files inside a submodule (gitlink) or a nested
//! repository, symlinks, and anything outside the repository are refused:
//! the source commit recorded in provenance binds only what is tracked, and
//! a file nobody committed is content nobody reviewed.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
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
        let listing = git(&root, &["ls-files", "--cached", "-z"], "git ls-files")?;
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

    /// Repository-relative paths of every listed `Cargo.toml` that exists;
    /// each still has to pass [`Worktree::owned_file`] before it is used.
    pub(crate) fn manifests(&self) -> BTreeSet<String> {
        self.visible
            .iter()
            .filter(|path| {
                Path::new(path.as_str())
                    .file_name()
                    .is_some_and(|name| name == "Cargo.toml")
            })
            .filter(|path| fs::symlink_metadata(self.root.join(path.as_str())).is_ok())
            .cloned()
            .collect()
    }

    /// Every listed ordinary file beneath `dir`, in path order. Symlinks,
    /// gitlinks, nested repositories, and ignored files are never listed, so
    /// nothing is reached through them.
    pub(crate) fn files_under(&self, dir: &Path) -> Vec<PathBuf> {
        let Some(prefix) = self.lexical_relative(dir) else {
            return Vec::new();
        };
        self.visible
            .iter()
            .filter(|path| {
                prefix.is_empty()
                    || path
                        .strip_prefix(prefix.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .map(|path| self.root.join(path))
            .filter(|path| self.owned_file(path, "file").is_ok())
            .collect()
    }

    /// Whether Git's ignore rules ignore `path` (an untracked, generated or
    /// local file such as a nested crate's `Cargo.lock`). Asked of Git itself
    /// so every ignore file and global rule applies; never true for a tracked
    /// file.
    pub(crate) fn is_ignored(&self, path: &Path) -> Result<bool, String> {
        let relative = self
            .lexical_relative(path)
            .filter(|relative| !relative.is_empty())
            .ok_or_else(|| format!("{} resolves outside the repository", path.display()))?;
        let mut command = crate::tools::tools()?.git();
        command
            .arg("-C")
            .arg(&self.root)
            .args(["check-ignore", "-q", "--"])
            .arg(&relative);
        let captured = run_bounded(command, "git check-ignore", 1024, MAX_DIAGNOSTIC_BYTES)?;
        match captured.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(format!(
                "git check-ignore failed for {relative}: {}",
                String::from_utf8_lossy(&captured.stderr).trim()
            )),
        }
    }

    /// Requires `path` to be an ordinary file owned by this worktree, reached
    /// from the repository root without passing through any symlink, and
    /// returns its repository-relative path.
    pub(crate) fn owned_file(&self, path: &Path, role: &str) -> Result<String, String> {
        let relative = self
            .lexical_relative(path)
            .filter(|relative| !relative.is_empty())
            .ok_or_else(|| format!("{role} {} resolves outside the repository", path.display()))?;
        let mut current = self.root.clone();
        let components = Path::new(&relative).components().collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            current.push(component);
            let metadata = fs::symlink_metadata(&current)
                .map_err(|err| format!("{role} {relative}: {err}"))?;
            let last = index + 1 == components.len();
            if last && (metadata.file_type().is_symlink() || !metadata.is_file()) {
                return Err(format!("{role} {relative} is not an ordinary file"));
            }
            if !last && metadata.file_type().is_symlink() {
                return Err(format!(
                    "{role} {relative} is reached through a symlink and resolves outside the repository's own files"
                ));
            }
        }
        if !self.visible.contains(&relative) {
            return Err(format!(
                "{role} {relative} is not a file of this Git worktree \
                 (untracked, ignored, inside a submodule or nested repository, or outside Git)"
            ));
        }
        Ok(relative)
    }
}

impl Worktree {
    /// `path` relative to the root without following anything: `None` when it
    /// is not lexically beneath the root or contains `..`.
    fn lexical_relative(&self, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(&self.root).ok()?;
        if !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return None;
        }
        Some(relative.to_str()?.replace(std::path::MAIN_SEPARATOR, "/"))
    }
}

fn git(repo: &Path, args: &[&str], role: &str) -> Result<Vec<u8>, String> {
    let mut command = crate::tools::tools()?.git();
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
