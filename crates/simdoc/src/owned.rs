// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Owned reads: the only way simdoc consumes repository files.
//!
//! Every generator runs inside a scope bound to the repository's Git
//! worktree. Inside it, a file may be read or listed only when it is an
//! ordinary file the worktree lists, reached from the root without passing
//! through a symlink. Walks enumerate the worktree listing rather than the
//! directory tree, so symlinked directories, nested repositories, submodules,
//! and ignored files are never followed. An attempt to read a file that
//! exists but is not owned is recorded, and the scope refuses the run when it
//! finishes, even if the caller tolerated the failed read.
//!
//! Outside a scope, production builds refuse every read; only unit tests may
//! read their own temporary fixtures directly.

use std::{
    cell::RefCell,
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::worktree::Worktree;

/// Ceiling for one consumed repository file.
pub(crate) const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

struct Scope {
    worktree: Worktree,
    violations: RefCell<Vec<String>>,
    /// Paths embedded in consumed content; publication may repeat them.
    consumed_paths: RefCell<BTreeSet<String>>,
}

thread_local! {
    static SCOPE: RefCell<Option<Rc<Scope>>> = const { RefCell::new(None) };
}

/// An active owned-read scope; dropping it restores the previous scope.
#[must_use = "the scope ends when the guard is dropped; call finish to check it"]
pub(crate) struct ScopeGuard {
    previous: Option<Rc<Scope>>,
    nested: bool,
}

impl ScopeGuard {
    /// Ends the scope, refusing the run if any non-owned file was touched.
    pub(crate) fn finish(self) -> Result<(), String> {
        if self.nested {
            return Ok(());
        }
        let violations = current().map(|scope| scope.violations.borrow().clone());
        match violations {
            Some(violations) if !violations.is_empty() => Err(format!(
                "refused: generation touched files the repository's Git worktree does not own: {}",
                violations.join("; ")
            )),
            _ => Ok(()),
        }
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        if !self.nested {
            SCOPE.with(|scope| *scope.borrow_mut() = previous);
        }
    }
}

/// Enters the owned-read scope of the worktree rooted at `repo`. Entering the
/// scope that is already active nests into it.
pub(crate) fn enter(repo: &Path) -> Result<ScopeGuard, String> {
    let root = repo
        .canonicalize()
        .map_err(|err| format!("{}: {err}", repo.display()))?;
    if let Some(active) = current()
        && active.worktree.root() == root
    {
        return Ok(ScopeGuard {
            previous: Some(active),
            nested: true,
        });
    }
    let scope = Rc::new(Scope {
        worktree: Worktree::open(&root)?,
        violations: RefCell::new(Vec::new()),
        consumed_paths: RefCell::new(BTreeSet::new()),
    });
    let previous = SCOPE.with(|active| active.borrow_mut().replace(scope));
    Ok(ScopeGuard {
        previous,
        nested: false,
    })
}

fn current() -> Option<Rc<Scope>> {
    SCOPE.with(|scope| scope.borrow().clone())
}

/// Reads an owned file as UTF-8.
pub(crate) fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "file is not UTF-8"))
}

/// Reads an owned file.
pub(crate) fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    let Some(scope) = current() else {
        return read_outside_scope(path);
    };
    if fs::symlink_metadata(path).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }
    if let Err(err) = scope.worktree.owned_file(path, "consumed file") {
        scope.violations.borrow_mut().push(err.clone());
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, err));
    }
    let bytes = bounded_read(path)?;
    if let Ok(text) = std::str::from_utf8(&bytes) {
        scope.consumed_paths.borrow_mut().extend(
            crate::publication::path_tokens(text)
                .into_iter()
                .map(str::to_owned),
        );
    }
    Ok(bytes)
}

/// Whether `token` occurs in repository content this scope consumed.
pub(crate) fn consumed_path(token: &str) -> bool {
    current().is_some_and(|scope| scope.consumed_paths.borrow().contains(token))
}

/// Every owned file beneath `dir`, in path order.
pub(crate) fn files_under(dir: &Path) -> Vec<PathBuf> {
    match current() {
        Some(scope) => scope.worktree.files_under(dir),
        None => list_outside_scope(dir),
    }
}

fn bounded_read(path: &Path) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::other(format!(
            "{} exceeds {MAX_FILE_BYTES} bytes",
            path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(not(test))]
fn read_outside_scope(path: &Path) -> io::Result<Vec<u8>> {
    panic!(
        "simdoc read {} outside an owned-read scope; every generator must enter one",
        path.display()
    )
}

#[cfg(not(test))]
fn list_outside_scope(dir: &Path) -> Vec<PathBuf> {
    panic!(
        "simdoc listed {} outside an owned-read scope; every generator must enter one",
        dir.display()
    )
}

#[cfg(test)]
fn read_outside_scope(path: &Path) -> io::Result<Vec<u8>> {
    bounded_read(path)
}

#[cfg(test)]
fn list_outside_scope(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
#[path = "owned_tests.rs"]
mod tests;
