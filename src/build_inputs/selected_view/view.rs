// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The selected resolver view tree: owner-bound package links, the finite
//! entry and byte budget, and revalidation that the links still resolve to
//! the same owners.

use super::{
    ResolverPackage, TreeLimit, attach_resolver_package_to_workspace, bounded_read, digest,
};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

#[derive(Serialize)]
pub(super) struct ViewBinding {
    package: String,
    manifest_owner: String,
    manifest_owner_sha256: String,
    manifest_sha256: String,
    members: Vec<MemberBinding>,
}

#[derive(Serialize)]
struct MemberBinding {
    name: String,
    owner: String,
}

pub(super) struct ViewBudget {
    limit: TreeLimit,
    pub(super) entries: usize,
    pub(super) bytes: usize,
}

impl ViewBudget {
    pub(super) fn new(limit: TreeLimit) -> Self {
        Self {
            limit,
            entries: 1,
            bytes: 0,
        }
    }

    pub(super) fn add(&mut self, bytes: usize) -> Result<(), String> {
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or("selected view entry overflow")?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or("selected view byte overflow")?;
        if self.entries > self.limit.entries || self.bytes > self.limit.bytes {
            return Err("selected resolver view exceeds its finite limit".into());
        }
        Ok(())
    }
}

pub(super) fn install_package_view(
    package: &ResolverPackage,
    workspace: &Path,
    destination: &Path,
    owner_roots: &[PathBuf],
    workspace_member: bool,
    budget: &mut ViewBudget,
) -> Result<ViewBinding, String> {
    let source_root = package
        .manifest
        .parent()
        .ok_or("selected package manifest has no parent")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let expected = workspace.join("packages").join(&package.name);
    if source_root != expected || package.manifest != expected.join("Cargo.toml") {
        return Err(format!(
            "selected package path is not canonical: {}",
            package.id
        ));
    }
    let owner_manifest = bounded_read(&package.manifest)?;
    let manifest = if workspace_member {
        owner_manifest.clone()
    } else {
        attach_resolver_package_to_workspace(&owner_manifest)?
    };
    let target = destination.join(&package.name);
    fs::create_dir(&target).map_err(|error| error.to_string())?;
    budget.add(0)?;
    budget.add(manifest.len())?;
    write_view_file(&target.join("Cargo.toml"), &manifest, false)?;
    let mut entries = fs::read_dir(&source_root)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut members = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        if name == "Cargo.toml" {
            continue;
        }
        if matches!(name.to_str(), Some(".git" | ".sim" | "target")) {
            continue;
        }
        let owner = entry
            .path()
            .canonicalize()
            .map_err(|error| format!("{}: {error}", entry.path().display()))?;
        if !owner_roots.iter().any(|root| owner.starts_with(root)) {
            return Err(format!(
                "selected package member escaped owner roots: {}",
                owner.display()
            ));
        }
        let name = name
            .to_str()
            .filter(|name| !name.is_empty() && name.len() <= 255)
            .ok_or("selected package member name is invalid")?;
        symlink(&owner, target.join(name)).map_err(|error| error.to_string())?;
        budget.add(0)?;
        members.push(MemberBinding {
            name: name.into(),
            owner: owner.display().to_string(),
        });
    }
    if members.is_empty() {
        return Err(format!(
            "selected package has no source members: {}",
            package.id
        ));
    }
    Ok(ViewBinding {
        package: package.name.clone(),
        manifest_owner: package.manifest.display().to_string(),
        manifest_owner_sha256: digest(&owner_manifest),
        manifest_sha256: digest(&manifest),
        members,
    })
}

pub(super) fn revalidate_view_bindings(
    view: &Path,
    bindings: &[ViewBinding],
) -> Result<(), String> {
    for package in bindings {
        let root = view.join("packages").join(&package.package);
        if digest(&bounded_read(Path::new(&package.manifest_owner))?)
            != package.manifest_owner_sha256
        {
            return Err(format!(
                "selected package owner manifest changed: {}",
                package.package
            ));
        }
        if digest(&bounded_read(&root.join("Cargo.toml"))?) != package.manifest_sha256 {
            return Err(format!(
                "selected package manifest changed: {}",
                package.package
            ));
        }
        for member in &package.members {
            let path = root.join(&member.name);
            if path.canonicalize().map_err(|error| error.to_string())? != Path::new(&member.owner) {
                return Err(format!(
                    "selected package source changed: {}",
                    package.package
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn write_view_file(path: &Path, bytes: &[u8], mutable: bool) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if mutable { 0o600 } else { 0o444 }),
    )
    .map_err(|error| error.to_string())
}
