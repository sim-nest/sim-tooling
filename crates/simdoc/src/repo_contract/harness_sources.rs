// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Covered-source ties and library tests of a focused harness.
//!
//! A harness may compile a covered package's source through a declared target
//! path or a `#[path]` attribute instead of a path dependency. `..` is accepted
//! only when it cancels an earlier component and the result is an ordinary file
//! this worktree owns. A path that leaves the repository, a symlink, and an
//! untracked or ignored file are not that file.

use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use serde_json::Value;

use crate::worktree::{Worktree, lexical_repo_relative};

/// How many module files one harness walk may open.
const MAX_FOLLOWED_FILES: usize = 64;

/// Whether `declared`, a path from the harness manifest, names an owned file.
///
/// Paths already listed under the package directory match directly. A path
/// with `.` or `..` matches when lexical normalization stays inside the
/// repository and [`Worktree::owned_file`] accepts the result. Absolute paths
/// stay refused.
pub(super) fn declared_path_is_owned(
    worktree: &Worktree,
    dir: &Path,
    declared: &str,
    owned: &BTreeSet<String>,
) -> bool {
    resolve_declared_source(worktree, dir, declared, owned).is_some()
}

/// Repository-relative form of a declared harness target, when it is owned.
pub(super) fn resolve_declared_source(
    worktree: &Worktree,
    dir: &Path,
    declared: &str,
    owned: &BTreeSet<String>,
) -> Option<String> {
    let path = Path::new(declared);
    if path.is_absolute() {
        return None;
    }
    let normalizes = path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir));
    if !normalizes {
        if !owned.contains(declared) {
            return None;
        }
        return worktree
            .owned_file(&dir.join(declared), "harness target")
            .ok();
    }
    let relative = lexical_repo_relative(worktree.root(), &dir.join(declared))?;
    if relative.is_empty() {
        return None;
    }
    worktree
        .owned_file(&worktree.root().join(&relative), "harness target")
        .ok()
}

/// Whether the harness library declares a test `cargo test` would compile.
///
/// An integration test file is handled by the caller. This covers a library
/// whose own source, or a `#[path]` module that source compiles, contains a
/// `#[test]` function or a `#[cfg(test)]` module whose body or file exists.
/// `#[cfg(not(test))]` and any other `cfg` hide the item. Comments do not count.
pub(super) fn library_declares_tests(
    worktree: &Worktree,
    dir: &Path,
    table: &toml::Table,
    owned: &BTreeSet<String>,
) -> bool {
    let declared = table
        .get("lib")
        .and_then(toml::Value::as_table)
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str);
    let Some(path) = declared.or(owned.contains("src/lib.rs").then_some("src/lib.rs")) else {
        return false;
    };
    let Some(relative) = resolve_declared_source(worktree, dir, path, owned) else {
        return false;
    };
    let mut scan = Scan::default();
    let mut seen = BTreeSet::new();
    scan_file(
        worktree,
        &worktree.root().join(&relative),
        &mut scan,
        &mut seen,
    );
    scan.has_tests
}

/// Whether a declared target or a `#[path]` attribute names owned Rust source
/// of a covered package, outside this harness directory.
pub(super) fn tied_by_covered_source(
    worktree: &Worktree,
    metadata: &Value,
    manifest: &Path,
    table: &toml::Table,
) -> bool {
    let Some(dir) = manifest.parent() else {
        return false;
    };
    let Ok(harness_dir) = worktree.owned_file(manifest, "harness manifest") else {
        return false;
    };
    let harness_dir = Path::new(&harness_dir)
        .parent()
        .map_or(String::new(), |parent| {
            parent.to_string_lossy().replace('\\', "/")
        });
    let Ok(packages) = covered_package_dirs(worktree, metadata) else {
        return false;
    };
    let owned = package_owned(worktree, dir);
    for declared in declared_manifest_paths(table) {
        let Some(relative) = resolve_declared_source(worktree, dir, &declared, &owned) else {
            continue;
        };
        if covered_rust(&relative, &harness_dir, &packages) {
            return true;
        }
    }
    let mut scan = Scan::default();
    let mut seen = BTreeSet::new();
    for file in &owned {
        if !file.ends_with(".rs") {
            continue;
        }
        scan_file(worktree, &dir.join(file), &mut scan, &mut seen);
        if scan
            .ties
            .iter()
            .any(|relative| covered_rust(relative, &harness_dir, &packages))
        {
            return true;
        }
    }
    false
}

#[derive(Default)]
struct Scan {
    has_tests: bool,
    ties: Vec<String>,
}

fn package_owned(worktree: &Worktree, dir: &Path) -> BTreeSet<String> {
    worktree
        .files_under(dir)
        .into_iter()
        .filter_map(|file| {
            file.strip_prefix(dir).ok().map(|rest| {
                rest.to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/")
            })
        })
        .collect()
}

fn declared_manifest_paths(table: &toml::Table) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(path) = table
        .get("lib")
        .and_then(toml::Value::as_table)
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
    {
        paths.push(path.to_owned());
    }
    for kind in ["bin", "test", "example", "bench"] {
        for target in table
            .get(kind)
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_table)
        {
            if let Some(path) = target.get("path").and_then(toml::Value::as_str) {
                paths.push(path.to_owned());
            }
        }
    }
    paths
}

fn covered_package_dirs(worktree: &Worktree, metadata: &Value) -> Result<BTreeSet<String>, String> {
    super::workspace_policy::member_packages(metadata)?
        .into_iter()
        .map(|package| {
            let manifest = package["manifest_path"]
                .as_str()
                .ok_or("cargo metadata package missing manifest_path")?;
            let relative = worktree.owned_file(Path::new(manifest), "package manifest")?;
            Ok(Path::new(&relative)
                .parent()
                .map_or(String::new(), |parent| {
                    parent.to_string_lossy().replace('\\', "/")
                }))
        })
        .collect()
}

fn covered_rust(file: &str, harness_dir: &str, packages: &BTreeSet<String>) -> bool {
    if !file.ends_with(".rs") {
        return false;
    }
    if !harness_dir.is_empty()
        && (file == harness_dir || file.starts_with(&format!("{harness_dir}/")))
    {
        return false;
    }
    packages.iter().any(|dir| {
        if dir.is_empty() {
            !file.is_empty()
        } else {
            file.starts_with(&format!("{dir}/"))
        }
    })
}

fn scan_file(worktree: &Worktree, file: &Path, scan: &mut Scan, seen: &mut BTreeSet<String>) {
    if seen.len() >= MAX_FOLLOWED_FILES {
        return;
    }
    let Ok(relative) = worktree.owned_file(file, "harness source") else {
        return;
    };
    if !seen.insert(relative) {
        return;
    }
    let Ok(text) = crate::owned::read_to_string(file) else {
        return;
    };
    let Ok(parsed) = syn::parse_file(&text) else {
        return;
    };
    let Some(dir) = file.parent() else {
        return;
    };
    scan_items(worktree, dir, &parsed.items, scan, seen);
}

fn scan_items(
    worktree: &Worktree,
    dir: &Path,
    items: &[syn::Item],
    scan: &mut Scan,
    seen: &mut BTreeSet<String>,
) {
    for item in items {
        match item {
            syn::Item::Fn(func) if !cfg_disabled(&func.attrs) && is_test(&func.attrs) => {
                scan.has_tests = true;
            }
            syn::Item::Mod(module) if !cfg_disabled(&module.attrs) => {
                if module.attrs.iter().any(is_cfg_test)
                    && module_source_exists(worktree, dir, module)
                {
                    scan.has_tests = true;
                }
                if let Some(path) = path_attribute(&module.attrs) {
                    let next = dir.join(&path);
                    if let Ok(relative) = worktree.owned_file(&next, "harness source") {
                        scan.ties.push(relative);
                    }
                    scan_file(worktree, &next, scan, seen);
                } else if let Some((_, inner)) = &module.content {
                    scan_items(worktree, dir, inner, scan, seen);
                }
            }
            _ => {}
        }
    }
}

fn module_source_exists(worktree: &Worktree, dir: &Path, module: &syn::ItemMod) -> bool {
    if module.content.is_some() {
        return true;
    }
    if let Some(path) = path_attribute(&module.attrs) {
        return worktree
            .owned_file(&dir.join(path), "harness source")
            .is_ok();
    }
    let name = module.ident.to_string();
    worktree
        .owned_file(&dir.join(format!("{name}.rs")), "harness source")
        .is_ok()
        || worktree
            .owned_file(&dir.join(&name).join("mod.rs"), "harness source")
            .is_ok()
}

fn cfg_disabled(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") && !is_cfg_test(attr))
}

fn is_cfg_test(attr: &syn::Attribute) -> bool {
    let syn::Meta::List(list) = &attr.meta else {
        return false;
    };
    list.path.is_ident("cfg")
        && syn::parse2::<syn::Path>(list.tokens.clone())
            .ok()
            .is_some_and(|path| path.is_ident("test"))
}

fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("test"))
}

fn path_attribute(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("path") {
            return None;
        }
        let syn::Meta::NameValue(name) = &attr.meta else {
            return None;
        };
        let syn::Expr::Lit(expr) = &name.value else {
            return None;
        };
        let syn::Lit::Str(literal) = &expr.lit else {
            return None;
        };
        Some(literal.value())
    })
}

#[cfg(test)]
#[path = "harness_source_tests.rs"]
mod tests;
