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
    path::{Component, Path, PathBuf},
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

    /// Requires `path` to be an ordinary file owned by this worktree, reached
    /// from the repository root without passing through any symlink, and
    /// returns its repository-relative path.
    ///
    /// Each original component is checked before `..` cancels it. A symlink
    /// followed by `..` is refused, because a later open uses the original
    /// path. A `..` that cancels only an ordinary component and stays inside
    /// the root names that file.
    pub(crate) fn owned_file(&self, path: &Path, role: &str) -> Result<String, String> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| outside(role, path))?;
        let original = relative.display().to_string();
        let mut parts = Vec::new();
        let mut current = self.root.clone();
        let components = relative.components().collect::<Vec<_>>();
        for (index, component) in components.iter().copied().enumerate() {
            match component {
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => return Err(outside(role, path)),
                Component::ParentDir => {
                    if parts.pop().is_none() {
                        return Err(outside(role, path));
                    }
                    current.pop();
                }
                Component::Normal(part) => {
                    let Some(part) = part.to_str() else {
                        return Err(outside(role, path));
                    };
                    current.push(part);
                    let metadata = fs::symlink_metadata(&current)
                        .map_err(|err| format!("{role} {original}: {err}"))?;
                    let continues = components[index + 1..]
                        .iter()
                        .any(|next| matches!(next, Component::Normal(_) | Component::ParentDir));
                    if metadata.file_type().is_symlink() {
                        if continues {
                            return Err(format!(
                                "{role} {original} is reached through a symlink and resolves outside the repository's own files"
                            ));
                        }
                        return Err(format!(
                            "{role} {} is not an ordinary file",
                            shown(&parts, part)
                        ));
                    }
                    parts.push(part);
                }
            }
        }
        let relative = parts.join("/");
        if relative.is_empty() {
            return Err(outside(role, path));
        }
        let metadata =
            fs::symlink_metadata(&current).map_err(|err| format!("{role} {relative}: {err}"))?;
        if !metadata.is_file() {
            return Err(format!("{role} {relative} is not an ordinary file"));
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

fn outside(role: &str, path: &Path) -> String {
    format!("{role} {} resolves outside the repository", path.display())
}

fn shown(parts: &[&str], part: &str) -> String {
    if parts.is_empty() {
        part.to_owned()
    } else {
        format!("{}/{part}", parts.join("/"))
    }
}

impl Worktree {
    /// `path` relative to the root without following anything.
    fn lexical_relative(&self, path: &Path) -> Option<String> {
        lexical_repo_relative(&self.root, path)
    }
}

/// Repository-relative form of `path` without following the filesystem.
///
/// `.` is dropped. `..` cancels the preceding component. The result is `None`
/// when `path` is not lexically beneath `root`, or when `..` would leave
/// `root`. This function does not inspect symlinks. [`Worktree::owned_file`]
/// checks each original component before cancelling `..` and refuses a
/// symlink on that traversal.
pub(crate) fn lexical_repo_relative(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.join("/"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_segments_inside_the_root_normalize() {
        let root = Path::new("/repo");
        assert_eq!(
            lexical_repo_relative(root, Path::new("/repo/crates/focused/../src/lib.rs")).as_deref(),
            Some("crates/src/lib.rs")
        );
        assert_eq!(
            lexical_repo_relative(root, Path::new("/repo")).as_deref(),
            Some("")
        );
    }

    #[test]
    fn parent_segments_that_leave_the_root_are_outside() {
        let root = Path::new("/repo");
        assert_eq!(
            lexical_repo_relative(root, Path::new("/repo/../outside.rs")),
            None
        );
        assert_eq!(
            lexical_repo_relative(root, Path::new("/repo/a/../../other.rs")),
            None
        );
        assert_eq!(
            lexical_repo_relative(root, Path::new("/other/src/lib.rs")),
            None
        );
    }
}
