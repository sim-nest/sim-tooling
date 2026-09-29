// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Finite regular-file trees and bounded filesystem IO: the only writer of
//! materialized inputs, with exact entry and byte limits, content records,
//! read-only permissions, and durable directory syncs.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Serialize)]
pub(crate) struct TreeLimit {
    pub(crate) entries: usize,
    pub(crate) bytes: usize,
}

#[derive(Serialize)]
pub(crate) struct TreeReport {
    pub(crate) entries: usize,
    pub(crate) bytes: usize,
    pub(crate) identity: String,
    pub(crate) files: Vec<FileRecord>,
}

#[derive(Serialize)]
pub(crate) struct FileRecord {
    pub(crate) path: String,
    pub(crate) bytes: usize,
    pub(crate) sha256: String,
    pub(crate) executable: bool,
}

pub(crate) struct TreeWriter {
    pub(super) root: PathBuf,
    limit: TreeLimit,
    entries: usize,
    bytes: usize,
    files: Vec<FileRecord>,
}

impl TreeWriter {
    pub(crate) fn create(root: PathBuf, limit: TreeLimit) -> Result<Self, String> {
        fs::create_dir(&root).map_err(|error| format!("{}: {error}", root.display()))?;
        Ok(Self {
            root,
            limit,
            entries: 1,
            bytes: 0,
            files: Vec::new(),
        })
    }

    pub(crate) fn copy_tree(
        &mut self,
        source: &Path,
        destination: &Path,
        allowed: &[PathBuf],
    ) -> Result<(), String> {
        if let Some(parent) = destination.parent() {
            self.ensure_parents(parent)?;
        }
        let mut active = BTreeSet::new();
        self.copy_entry(source, destination, allowed, &mut active)
    }

    fn copy_entry(
        &mut self,
        source: &Path,
        destination: &Path,
        allowed: &[PathBuf],
        active: &mut BTreeSet<PathBuf>,
    ) -> Result<(), String> {
        validate_relative(destination)?;
        let metadata = fs::symlink_metadata(source)
            .map_err(|error| format!("{}: {error}", source.display()))?;
        let selected = if metadata.file_type().is_symlink() {
            let target = source
                .canonicalize()
                .map_err(|error| format!("{}: {error}", source.display()))?;
            if !allowed.iter().any(|root| target.starts_with(root)) {
                return Err(format!(
                    "source symlink escaped selected owners: {}",
                    source.display()
                ));
            }
            target
        } else {
            source.to_owned()
        };
        let metadata =
            fs::metadata(&selected).map_err(|error| format!("{}: {error}", selected.display()))?;
        if metadata.is_dir() {
            if !active.insert(selected.clone()) {
                return Err(format!("source directory cycle: {}", selected.display()));
            }
            self.create_directory(destination)?;
            let mut members = fs::read_dir(&selected)
                .map_err(|error| format!("{}: {error}", selected.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            members.sort_by_key(|entry| entry.file_name());
            for member in members {
                let name = member.file_name();
                if matches!(name.to_str(), Some(".git" | ".sim" | "target")) {
                    continue;
                }
                self.copy_entry(&member.path(), &destination.join(name), allowed, active)?;
            }
            active.remove(&selected);
            return Ok(());
        }
        if !metadata.is_file() {
            return Err(format!(
                "selected input is not regular: {}",
                selected.display()
            ));
        }
        // A copied package's own file content, unlike a control or record
        // file: a real, legitimately empty file must still copy.
        let bytes = bounded_read_with_limit(&selected, self.limit.bytes, true)?;
        let executable = metadata.permissions().mode() & 0o111 != 0;
        self.write_file(destination, &bytes, executable)
    }

    fn create_directory(&mut self, relative: &Path) -> Result<(), String> {
        let path = self.root.join(relative);
        if path == self.root {
            return Ok(());
        }
        self.add_entry(0)?;
        fs::create_dir(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())
    }

    pub(super) fn write_file(
        &mut self,
        relative: &Path,
        bytes: &[u8],
        executable: bool,
    ) -> Result<(), String> {
        validate_relative(relative)?;
        if let Some(parent) = relative.parent() {
            self.ensure_parents(parent)?;
        }
        self.add_entry(bytes.len())?;
        let path = self.root.join(relative);
        write_new(&path, bytes, executable)?;
        self.files.push(FileRecord {
            path: slash(relative)?,
            bytes: bytes.len(),
            sha256: digest(bytes),
            executable,
        });
        Ok(())
    }

    fn ensure_parents(&mut self, relative: &Path) -> Result<(), String> {
        let mut path = PathBuf::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err("selected output parent is not relative".into());
            };
            path.push(name);
            let full = self.root.join(&path);
            if !full.exists() {
                self.create_directory(&path)?;
            }
        }
        Ok(())
    }

    fn add_entry(&mut self, bytes: usize) -> Result<(), String> {
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or("tree entry count overflow")?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or("tree byte count overflow")?;
        if self.entries > self.limit.entries || self.bytes > self.limit.bytes {
            return Err("materialized tree exceeds its exact finite limit".into());
        }
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<TreeReport, String> {
        self.files.sort_by(|left, right| left.path.cmp(&right.path));
        let identity = digest(&serde_json::to_vec(&self.files).map_err(|error| error.to_string())?);
        sync_tree(&self.root)?;
        Ok(TreeReport {
            entries: self.entries,
            bytes: self.bytes,
            identity,
            files: self.files,
        })
    }
}

pub(crate) fn write_new(path: &Path, bytes: &[u8], executable: bool) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 }),
    )
    .map_err(|error| error.to_string())
}

fn sync_tree(path: &Path) -> Result<(), String> {
    let mut directories = vec![path.to_owned()];
    let mut index = 0;
    while index < directories.len() {
        let directory = directories[index].clone();
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                directories.push(entry.path());
            }
        }
        index += 1;
    }
    for directory in directories.into_iter().rev() {
        sync_directory(&directory)?;
    }
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("{}: {error}", path.display()))
}

pub(crate) fn bounded_read(path: &Path) -> Result<Vec<u8>, String> {
    bounded_read_with_limit(path, MAX_INPUT_BYTES, false)
}

/// Like [`bounded_read`], except a zero-length file is not refused: real
/// Cargo package content (unlike the control and record files `bounded_read`
/// is normally used for -- a manifest, a lock, `cargo metadata`'s own
/// output -- can legitimately be empty, e.g. an empty `build.rs` companion
/// module or marker file shipped in a real published crate).
pub(crate) fn bounded_read_allow_empty(path: &Path) -> Result<Vec<u8>, String> {
    bounded_read_with_limit(path, MAX_INPUT_BYTES, true)
}

fn bounded_read_with_limit(
    path: &Path,
    limit: usize,
    allow_empty: bool,
) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let length = usize::try_from(file.metadata().map_err(|error| error.to_string())?.len())
        .map_err(|_| "selected input length exceeds address space")?;
    if (length == 0 && !allow_empty) || length > limit {
        return Err(format!(
            "selected input length is outside bounds: {}",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(length);
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() != length {
        return Err(format!(
            "selected input changed while read: {}",
            path.display()
        ));
    }
    Ok(bytes)
}

pub(crate) fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "selected path is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn validate_relative(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.components().count() > 32
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!(
            "selected output path is not bounded relative: {}",
            path.display()
        ));
    }
    Ok(())
}

fn slash(path: &Path) -> Result<String, String> {
    path.to_str()
        .filter(|value| value.len() <= 4096 && !value.contains('\\'))
        .map(str::to_owned)
        .ok_or_else(|| "selected output path is not bounded UTF-8".into())
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn staging_path(parent: &Path, destination: &Path) -> Result<PathBuf, String> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("build-inputs destination has no UTF-8 name")?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(parent.join(format!(
        ".{name}.materializing-{}-{nonce}",
        std::process::id()
    )))
}
