// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A private, checksum-verified Cargo home for every cargo run.
//!
//! Cargo builds whatever it finds unpacked in `CARGO_HOME/registry/src`, and
//! reads configuration from Cargo's home, so a shared home is an unbound
//! input. Instead each run gets a fresh directory (owner-only, removed when
//! its owning process is gone) that holds only the registry crates the
//! governing `Cargo.lock` names, each copied from the real cache after its
//! SHA-256 equals the lock's checksum, and the index cache Cargo needs to
//! resolve them. Cargo unpacks the verified archives itself, into the private
//! home. Nothing else of the real home (configuration, credentials, unpacked
//! sources, git checkouts, binaries) is visible. A lock that names a git
//! dependency is refused. Same rules, three copies: the engine (here), sim-tooling's
//! launcher, and sim-platform's launcher.

use std::{
    ffi::OsString,
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

const PREFIX: &str = "simdoc-private-";
/// Ceiling for a copied index cache.
const MAX_INDEX_BYTES: u64 = 512 * 1024 * 1024;

/// A fresh owner-only directory in the temporary directory, after removing
/// the private directories of processes that no longer exist.
pub(crate) fn private_dir(label: &str) -> Result<PathBuf, String> {
    let temp = std::env::temp_dir();
    require_safe_parent(&temp)?;
    if let Ok(entries) = fs::read_dir(&temp) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix(PREFIX) else {
                continue;
            };
            let pid = rest
                .split('-')
                .next()
                .and_then(|pid| pid.parse::<u32>().ok());
            if pid.is_some_and(|pid| !Path::new(&format!("/proc/{pid}")).exists())
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_nanos();
    let dir = temp.join(format!("{PREFIX}{}-{stamp}-{label}", std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|err| format!("create {}: {err}", dir.display()))?;
    Ok(dir)
}

/// The directory that will hold private directories must be one only we (or
/// root) can rename entries in: owned by us or root, and not writable by group
/// or others unless sticky. Otherwise another user could swap our directory
/// for theirs between creation and use.
fn require_safe_parent(parent: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    let ours = fs::metadata("/proc/self")
        .map(|meta| meta.uid())
        .map_err(|err| format!("/proc/self: {err}"))?;
    let shared_writable = meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0;
    if !meta.is_dir() || (meta.uid() != ours && meta.uid() != 0) || shared_writable {
        return Err(format!(
            "refused: {} is not a private place for scratch directories (owner {}, mode {:o})",
            parent.display(),
            meta.uid(),
            meta.mode() & 0o7777
        ));
    }
    Ok(())
}

/// The real Cargo home, as an absolute path: the explicit one a launcher
/// names, else `CARGO_HOME`, else `HOME/.cargo`. Relative or empty values are
/// refused: they would mean different places to different processes.
pub(crate) fn real_cargo_home(
    explicit: Option<OsString>,
    cargo_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, String> {
    let path = match (explicit, cargo_home, home) {
        (Some(path), _, _) | (None, Some(path), _) => PathBuf::from(path),
        (None, None, Some(home)) => Path::new(&home).join(".cargo"),
        (None, None, None) => {
            return Err("no Cargo home: neither CARGO_HOME nor HOME is set".to_owned());
        }
    };
    if !path.is_absolute() {
        return Err(format!(
            "the Cargo home {} is not an absolute path; refusing to guess what it means",
            path.display()
        ));
    }
    Ok(path)
}

/// A private Cargo home holding exactly the registry crates `lock` names,
/// each verified against its checksum. `None` yields an empty home (for runs
/// that resolve nothing from a registry).
pub(crate) fn hermetic_home(
    real_home: &Path,
    lock: Option<&str>,
    label: &str,
) -> Result<PathBuf, String> {
    let home = private_dir(label)?;
    let Some(lock) = lock else {
        return Ok(home);
    };
    for package in registry_packages(lock)? {
        for (what, value, extra) in [
            ("name", &package.name, ""),
            ("version", &package.version, ".+"),
        ] {
            let plain = !value.is_empty()
                && value.chars().all(|ch| {
                    ch.is_ascii_alphanumeric() || "-_".contains(ch) || extra.contains(ch)
                });
            if !plain || value.starts_with('.') {
                return Err(format!(
                    "Cargo.lock has a package {what} {value:?} that is not a plain crate {what}"
                ));
            }
        }
        if package.checksum.len() != 64
            || !package.checksum.chars().all(|ch| ch.is_ascii_hexdigit())
        {
            return Err(format!(
                "Cargo.lock gives {} no valid checksum",
                package.name
            ));
        }
        let file = format!("{}-{}.crate", package.name, package.version);
        let cache_root = real_home.join("registry/cache");
        let Ok(indexes) = fs::read_dir(&cache_root) else {
            continue;
        };
        for index in indexes.flatten() {
            let source = index.path().join(&file);
            let Ok(bytes) = fs::read(&source) else {
                continue;
            };
            let digest = Sha256::digest(&bytes)
                .iter()
                .fold(String::new(), |mut out, byte| {
                    out.push_str(&format!("{byte:02x}"));
                    out
                });
            if digest != package.checksum {
                return Err(format!(
                    "{} in the Cargo cache does not match the checksum {} in Cargo.lock",
                    source.display(),
                    package.checksum
                ));
            }
            let target = home.join("registry/cache").join(index.file_name());
            fs::create_dir_all(&target).map_err(|err| format!("{}: {err}", target.display()))?;
            fs::write(target.join(&file), &bytes)
                .map_err(|err| format!("{}: {err}", target.join(&file).display()))?;
            let index_from = real_home.join("registry/index").join(index.file_name());
            let index_to = home.join("registry/index").join(index.file_name());
            let plain_dir = fs::symlink_metadata(&index_from).is_ok_and(|meta| meta.is_dir());
            if plain_dir && !index_to.exists() {
                let mut budget = MAX_INDEX_BYTES;
                copy_tree(&index_from, &index_to, &mut budget)?;
            }
        }
    }
    Ok(home)
}

struct LockedPackage {
    name: String,
    version: String,
    checksum: String,
}

/// The registry packages of a `Cargo.lock` (`[[package]]` tables with a
/// `source` of a registry), read line by line. A git dependency refuses.
fn registry_packages(lock: &str) -> Result<Vec<LockedPackage>, String> {
    let mut packages = Vec::new();
    let mut current: Option<(String, String, String, String)> = None; // name, version, source, checksum
    let mut finish =
        |current: &mut Option<(String, String, String, String)>| -> Result<(), String> {
            if let Some((name, version, source, checksum)) = current.take() {
                if source.starts_with("git+") {
                    return Err(format!(
                        "Cargo.lock names the git dependency {name}; a private Cargo home holds \
                     registry crates only"
                    ));
                }
                if source.starts_with("registry+") || source.starts_with("sparse+") {
                    if checksum.is_empty() {
                        return Err(format!("Cargo.lock gives {name} {version} no checksum"));
                    }
                    packages.push(LockedPackage {
                        name,
                        version,
                        checksum,
                    });
                }
            }
            Ok(())
        };
    for line in lock.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            finish(&mut current)?;
            current = (line == "[[package]]").then(Default::default);
            continue;
        }
        let (Some(entry), Some((key, value))) = (current.as_mut(), line.split_once('=')) else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        match key.trim() {
            "name" => entry.0 = value,
            "version" => entry.1 = value,
            "source" => entry.2 = value,
            "checksum" => entry.3 = value,
            _ => {}
        }
    }
    finish(&mut current)?;
    Ok(packages)
}

fn copy_tree(from: &Path, to: &Path, budget: &mut u64) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|err| format!("{}: {err}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|err| format!("{}: {err}", from.display()))? {
        let entry = entry.map_err(|err| format!("{}: {err}", from.display()))?;
        let kind = entry
            .file_type()
            .map_err(|err| format!("{}: {err}", entry.path().display()))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, budget)?;
        } else if kind.is_symlink() {
            return Err(format!(
                "the registry index cache holds a symlink at {}",
                entry.path().display()
            ));
        } else if kind.is_file() {
            let length = entry
                .metadata()
                .map_err(|err| format!("{}: {err}", entry.path().display()))?
                .len();
            *budget = budget
                .checked_sub(length)
                .ok_or("the registry index cache exceeds its ceiling")?;
            fs::copy(entry.path(), &target)
                .map_err(|err| format!("{}: {err}", entry.path().display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "cargo_home_tests.rs"]
mod tests;
